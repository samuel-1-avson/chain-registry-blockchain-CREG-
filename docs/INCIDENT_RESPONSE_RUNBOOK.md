# Incident Response Runbook

> Track: MAIN-003 (public alpha gate)  
> Scope: CREG public testnet (`creg-testnet-1`) — node fleet, contracts on Sepolia, IPFS, explorer, faucet, spec server, waitlist  
> Companion docs: [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md), [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md), [CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md](./CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md)

## Severity Levels

| Sev | Definition | Examples | Response target |
| --- | --- | --- | --- |
| SEV-1 | Active harm to users or trust root | Malicious package verified and installable; validator key compromise; contract exploit | Acknowledge < 1h, mitigate < 4h |
| SEV-2 | Core function degraded, no active harm | Consensus halt, IPFS outage for verified packages, RPC outage breaking validator-set sync | Acknowledge < 4h, mitigate < 24h |
| SEV-3 | Partial degradation | Explorer down, faucet drained, spec server stale, waitlist down | Acknowledge < 24h |

Roles (alpha-scale): **Incident lead** (maintainer on point), **Operator** (infra access: GCP, contracts, keys), **Comms** (status + user notification). One person may hold multiple roles; name them in the incident notes.

## General Procedure (all incidents)

1. **Triage** — confirm the report, assign severity, open an incident note (timestamped log; GitHub issue or private doc for security-sensitive cases).
2. **Contain** — stop ongoing harm first (revoke, pause, isolate). Do not destroy evidence: snapshot logs/state before restarting containers.
3. **Communicate** — post user-facing status (explorer banner / GitHub issue / waitlist email for broad impact). Never overclaim; state what is known.
4. **Remediate** — fix root cause, redeploy, verify health.
5. **Post-mortem** — within 5 days: timeline, root cause, what failed, follow-up tasks filed in [NEXT_WORK.md](./NEXT_WORK.md) or [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md).

Evidence to preserve in every incident: container logs (`docker logs`), node data-dir snapshots if state-related, relevant L1 tx hashes, package IDs/CIDs, vote records (evidence digests), and the incident timeline.

---

## Scenario 1: Malicious Package Approved (SEV-1)

A package with malicious behaviour reached `verified` status.

**Contain**

1. Record package ID, version, CID, evidence digest, and the validator votes that approved it.
2. Revoke/quarantine the package via the operator API so `creg install` refuses it and explorer shows revoked status:
   - Use the operator revocation endpoint (`/v1/operator/*`, requires `CREG_OPERATOR_API_KEY`).
3. Unpin the CID from operator IPFS infrastructure if the payload itself is dangerous (`ipfs pin rm <cid>`), after archiving a copy for analysis in an isolated location.

**Communicate**

4. Publish a revocation notice: package name/version, why it was revoked, what users who installed it should do. Template: MAIN-005.
5. If installs likely happened, notify through every available channel (explorer banner, GitHub issue, waitlist email).

**Remediate**

6. Reproduce the miss against the malicious-fixture suite (MAL-002); add the evasion pattern as a new fixture + scanner rule.
7. Determine whether any validator voted without consensus-grade evidence (dev bypass, stale scanner profile) — if so, follow Scenario 2 (validator misbehaviour/compromise path).
8. Rescan related packages from the same publisher; consider publisher suspension.
9. Post-mortem includes: detection gap, time-to-revoke, and whether vote transparency data was sufficient.

## Scenario 2: Validator Key Compromise Or Misbehaviour (SEV-1)

A validator Ed25519 key (or operator EOA) is exposed, or a validator emits provably bad votes.

**Contain**

1. Identify the validator (node ID, EVM address, pubkey) and freeze its participation: the operator with governance access removes/suspends it from the active set (consensus removal path), or at minimum the affected operator stops the node immediately.
2. If an EOA holding stake is compromised: move/lock what can be saved and treat the staked position as hostile — coordinate slashing/suspension on the Staking contract.
3. Rotate keys per [SECURITY_OPS_RUNBOOK.md → Rotation procedure](./SECURITY_OPS_RUNBOOK.md): new Ed25519 key, re-register identity, re-admit via consensus.

**Investigate**

4. Pull the validator's vote history; flag votes signed after the suspected exposure window.
5. Re-validate packages whose quorum depended on the compromised validator's votes; revoke any that no longer meet quorum with clean votes (Scenario 1).

**Remediate**

6. Document the slashing/suspension evidence trail (VAL-007) even if executed manually during alpha.
7. Post-mortem: how the key was exposed, whether hot-key rules were followed, monitoring gaps.

## Scenario 3: Contract Pause / L1 Contract Issue (SEV-1/2)

A bug or exploit in Staking, Registry, Governance, token, or ZK verifier contracts.

1. Confirm the issue on Sepolia (tx hashes, state reads). Snapshot relevant state with `cast call`.
2. If pause controls exist on the affected contract, the governance owner pauses it (AUD-007 checklist documents pause controls and ownership for each deployment).
3. Set validators to a safe mode if the contract feeds consensus inputs (e.g., validator-set sync frozen at last good block — document the block number).
4. Announce: which contract, what is frozen, what user actions are unsafe (staking, registration).
5. Fix path: contracts are not yet audited (SEC-401 pending) — any redeployment targets fresh Sepolia addresses, with chain spec + compose defaults + docs updated together, and the old addresses marked deprecated.
6. Post-mortem feeds the SEC-401 audit scope.

## Scenario 4: IPFS Outage / Verified Content Unavailable (SEV-2)

Verified packages cannot be fetched (operator IPFS node down, gateway failing, or content unpinned).

1. Check operator IPFS container/VM health (`docker ps`, `ipfs swarm peers`, gateway probe on `https://ipfs.testnet.cregnet.dev`).
2. Restart or restore the IPFS node; verify the pinset is intact (`ipfs pin ls --type=recursive`).
3. If pins were lost: re-pin all accepted/verified CIDs from the registry index (IPFS-001 pinning job is the source of truth for the expected pinset).
4. Verify availability end-to-end: fetch a verified package through the public gateway and check its hash (`creg verify` / `creg install`).
5. If outage exceeds 1h, post a status notice — installs are degraded even though chain state is fine. Use the Track 3 messaging: trust records are on-chain; availability depends on pinning.
6. Post-mortem: add/repair the CID availability checker alerts (IPFS-002) if they failed to catch it.

## Scenario 5: RPC Outage / Validator-Set Sync Degraded (SEV-2)

Sepolia RPC failures break `validator_set_sync`, staking checks, or bridge anchoring.

1. Confirm: node logs show `eth_getLogs` errors or `validator_set_sync_state=degraded`; check provider status page.
2. Switch `CREG_ETH_RPC` to a backup provider (keep at least one alternate configured; archive-capable required). For the GCP fleet this is a `sepolia-3node.env` update + `deploy-validator-fleet.ps1 -SkipSync`.
3. Internal geth option: point validators at the internal Sepolia geth VM once its sync is verified (`eth_getCode` at the Registry address) — see `testnet/gcp/deploy-sepolia-geth.ps1`.
4. Validators keep operating on the last finalized validator set during the outage; new registrations and stake changes queue until sync recovers.
5. Verify recovery: `validator_set_sync_state` healthy, `validator_set_last_finalized_source_block` advancing.

## Scenario 6: Public Endpoint Outage (SEV-2/3)

API, explorer, faucet, spec server, or waitlist down.

1. Identify the failing layer: DNS/Cloudflare → Caddy edge (`creg-testnet-vm`) → upstream service (fleet VM or edge container).
2. Edge checks: `testnet/gcp/ssh-vm.ps1 -Command "docker ps"`; fleet checks: `testnet/gcp/ssh-validator-vm.ps1 -Command "docker ps"`.
3. Restart the failed service via the corresponding start script (`start-remote-stack.sh`, `start-validator-fleet-gcp.sh`, `start-remote-waitlist.sh`).
4. Spec server outage is consensus-sensitive: nodes fall back to cached chain spec (`CREG_CHAIN_SPEC_OFFLINE` path) — restore before cache-less nodes restart.
5. Faucet drained or abused: rotate `FAUCET_PRIVATE_KEY` per the security runbook, refund, and consider rate-limit tightening.

---

## Communication Templates (MAIN-005 stubs)

**Incident notice**

> [CREG testnet] Incident — <component>: <one-line impact>. Started <UTC time>. Current status: <investigating/mitigating/resolved>. User action: <none/specific>. Updates: <link>.

**Package revocation notice**

> Package <ecosystem>:<name>@<version> has been revoked (<reason class>). If you installed it after <date>, <action>. Evidence digest and vote record: <explorer link>.

**Resolution notice**

> [CREG testnet] Resolved — <component>. Root cause: <short>. Duration: <window>. Follow-ups: <issue links>. Post-mortem: <link, within 5 days>.

## Drill Requirements (Public Alpha Gate)

Before L2 public alpha, run and archive evidence for:

- [ ] One revocation drill (Scenario 1 on a benign test package) — output saved (MAL-005).
- [ ] One validator removal/rotation dry run (Scenario 2 steps 1–3 on a dev validator).
- [ ] One IPFS unpin/repin recovery on a test CID (Scenario 4).
- [ ] One RPC failover (Scenario 5) on the fleet.

Record drill dates and evidence paths here when completed:

| Drill | Date | Evidence |
| --- | --- | --- |
| Revocation | TBD | TBD |
| Validator rotation | TBD | TBD |
| IPFS recovery | TBD | TBD |
| RPC failover | TBD | TBD |
