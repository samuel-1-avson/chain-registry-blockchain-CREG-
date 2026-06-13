# Ops Hardening Runbook (P3)

Operational hardening for the CREG testnet: monitoring/alerting, disaster
recovery, hot-key management, and edge protection. Several items are GCP
deployment actions; this runbook records what exists, what to run, and what
is still gated on infra/quota.

---

## 1. Monitoring & alerting

The node exposes Prometheus metrics at `GET /metrics`
(`crates/node/src/metrics.rs`). Key series:

| Metric | Meaning |
|--------|---------|
| `creg_chain_tip_height` | L2 tip height (liveness) |
| `creg_active_validators` | validators eligible for PBFT (status online/self) |
| `creg_validator_set_total` | total validators in the set |
| `creg_reorg_events_total` | L2 reorgs recorded (windowed) |
| `creg_validator_set_sync_state_code` | 0 disabled · 1 syncing · 2 reorg-replaying · 3 degraded · 4 synced |
| `creg_validator_set_sync_has_error` | 1 when L1 sync has an error |
| `creg_bridge_anchor_count` | L2→L1 checkpoints committed |
| `creg_bridge_last_anchor_eth_block` | L1 block of the most recent anchor |
| `creg_bridge_finalized_l1_block` | most recent finalized L1 block seen by the bridge |
| `creg_sandbox_dev_bypass` | 1 when `CREG_DEV_SANDBOX=true` (must be 0 on public validators, MAL-001) |
| `creg_sandbox_isolated` | 1 when nsjail/gVisor/Docker isolation is active |

### Wire it up
1. Add a scrape job labelled `creg-node` pointing at each node's `/metrics`
   (validators + observers).
2. Load the alert rules: `testnet/monitoring/creg-alerts.yml` via
   `rule_files:` in `prometheus.yml`.
3. Point Alertmanager at your channel. For GCP, run
   `testnet/gcp/setup-alert-receiver.ps1` (default: **ntfy** mobile push). It stores
   sensitive values in **GCP Secret Manager** and keeps only secret *names* in
   `testnet/gcp/hosting.env` (see `hosting.env.example`):
   - `GCP_ALERT_NTFY_TOPIC_SECRET` + `GCP_ALERT_NTFY_SERVER` — ntfy.sh push (no Slack)
   - `GCP_ALERT_SLACK_WEBHOOK_SECRET` — Slack incoming webhook
   - `GCP_ALERT_WEBHOOK_SECRET` — Discord / Google Chat / custom webhook
   - `GCP_ALERT_EMAIL_TO` + `GCP_ALERT_SMTP_*` — email via SMTP
   - `GCP_ALERT_PAGERDUTY_SECRET` — optional PagerDuty Events API v2 key
   Enable `secretmanager.googleapis.com` on the testnet project before first use.
   `deploy-monitoring.ps1` reads secrets via `_GcpSecret.ps1`, generates
   `testnet/monitoring/alertmanager-gcp.yml` (gitignored; template in
   `alertmanager-gcp.yml.example`).
4. **GCP testnet (edge VM):** run `testnet/gcp/deploy-monitoring.ps1` to
   generate `testnet/monitoring/prometheus-gcp.yml`, sync to `creg-testnet-vm`,
   and start `testnet/monitoring/docker-compose.monitoring.yml`. Verify with
   `testnet/gcp/verify-monitoring.ps1`. Prometheus listens on `127.0.0.1:9090`
   on the edge VM (use IAP SSH port-forward to browse).

### Alerts (see `creg-alerts.yml`)
Node down, block production stalled (15m), PBFT quorum at risk
(`active_validators < 2`), L2 reorg observed, L1 sync degraded/errored/reorg-
churning, bridge L1 stalled (no finalized block in 1h), and **MAL-001 sandbox**
(`CregSandboxDevBypass`, `CregSandboxNotIsolated`).

### Faucet / relayer balances
These run as separate services and are **not** in the node metrics. Options:
- Run faucet/relayer with their own metrics and add the commented rules in
  `creg-alerts.yml`, or
- Add a blackbox balance probe (a small scheduled job calling `eth_getBalance`
  on the faucet/relayer/bridge hot wallets and exporting a gauge), then alert
  when a wallet drops below the gas/drip floor.

---

## 2. Disaster recovery (single-zone GCP)

All authoritative node state lives in `CREG_DATA_DIR` on the validator VM's
persistent disk:

| File | Produced by |
|------|-------------|
| `chain.rocksdb/` | block + package store (the chain) |
| `validator-set-sync.cursor.json` | L1 sync cursor |
| `bridge_anchors.json` | L2→L1 checkpoint journal |
| `validator-registrations.json` | gossiped validator identities |
| `validator-set-history.json` | height-indexed validator sets (ISSUE-050) |
| `pending_pool.json` | pre-consensus mempool |

Losing the disk without a backup means rebuilding the chain from peers (if any)
and re-registering identities. Back it up.

### Backup
- On-demand / Windows operator: `testnet/gcp/backup-vm-disks.ps1`
  (snapshots the validator/edge/geth VM disks; `-Prune -RetentionDays N` to
  age out old snapshots).
- Preferred for continuous protection: a GCP **resource snapshot schedule**
  attached to the disks (daily, 14-day retention, multi-region storage). The
  script covers on-demand and ad-hoc; the schedule covers unattended.

### Restore
1. `gcloud compute snapshots list --filter labels.app=creg-testnet`.
2. Create a disk from the snapshot — **in the target zone** (can differ from
   the original for zonal-outage recovery):
   `gcloud compute disks create creg-validator-restore --source-snapshot <snap> --zone <zone>`.
3. Create/replace the VM using the restored disk (or detach the bad disk and
   attach the restored one), keeping the same `CREG_DATA_DIR` mount path.
4. Start the node; it resumes from the restored RocksDB + cursors. Validator-set
   sync re-verifies against L1; identity gossip re-converges.
5. Verify: `l2-gate-verify.ps1 -Live`, `verify-sepolia-rpc-endpoints.ps1`, and
   `active_validators >= 2`.

### Multi-zone
The deployment is single-zone (`us-central1-a`). Until true multi-zone HA
(independent validators in separate zones — see
`docs/L1_L2_HARDENING.md` validator-split runbook), the snapshot→restore-in-
another-zone path above is the zonal-outage recovery mechanism. RTO is bounded
by snapshot age + VM bring-up.

---

## 3. Hot-key management (KMS / Vault)

Vault support is **already implemented** in the `chain_registry_secrets` crate:
- `CREG_SECRETS_BACKEND=vault` switches bridge / faucet / relayer / validator
  hot keys from env vars to HashiCorp Vault KV v2 (`HotKeyRole`).
- `validate_production_secrets_policy` rejects the env backend off-testnet, and
  services log a warning when a hot key is sourced from env (`warn_hot_key_if_env`).

### Migration steps
1. Stand up Vault (or a Vault-compatible KMS) reachable from the VMs.
2. Store each hot key at its role path (see `HotKeyRole::default_vault_path` /
   the `*_VAULT_PATH` env overrides): bridge, faucet, relayer, validator Ed25519.
3. Set `CREG_SECRETS_BACKEND=vault` + Vault auth env (`VAULT_*`) on each
   service; remove the plaintext `*_PRIVATE_KEY` / `CREG_*_KEY` env vars.
4. Restart services; confirm no "hot key from env" warnings remain.

### Governance / deployer keys
These are **deploy-time** signers, not runtime node hot keys. Keep them off the
servers entirely: hardware wallet or an offline signer, and raise
`GOVERNANCE_THRESHOLD >= 2` with independent signers (the deploy script now
warns at threshold 1). See `docs/CONTRACT_FIXES_REDEPLOY_PLAN.md`.

---

## 4. Edge protection (Cloud Armor) & WAF

Scripts already exist:
- `testnet/gcp/request-cloud-armor-quota.ps1` — request the Cloud Armor quota.
- `testnet/gcp/setup-cloud-armor.ps1` — create the security policy + rules and
  attach it to the global HTTPS LB backend.

**Status:** blocked on the Cloud Armor quota grant (per
`L2_PUBLIC_ALPHA_GATE_STATUS.md`). Once granted: run the quota script, then the
setup script, then verify the policy is attached to the `api.*` LB backend.
Until then the public edge relies on Caddy/nginx rate limits and the node's own
P2P/REST rate limiting.

---

## 5. Assurance backlog (process, not code)

- **SEC-401 external audit:** outreach prepared; not yet executed. Must precede
  any mainnet-beta. Contracts are unaudited (incident runbook assumes this).
- **IPFS pin check vs. real packages:** the pin gate currently passes vacuously
  (`total_packages: 0`). Publish real packages on the testnet and re-run
  `run-ipfs-pin-check.ps1` so the pin pipeline is exercised against real CIDs.

---

## Quick verification checklist
- Prometheus scrapes `creg-node`; `creg-alerts.yml` loaded; a test alert fires.
- `backup-vm-disks.ps1` produces snapshots; a restore has been rehearsed once.
- No "hot key from env" warnings in production service logs.
- Cloud Armor policy attached (once quota granted).
- SEC-401 scheduled; IPFS pin check run against real packages.
