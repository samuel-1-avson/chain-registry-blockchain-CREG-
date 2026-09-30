# CREG Testnet Validator Runbook

Operational guide for running a CREG testnet validator, distilled from the
September 2026 reboot. Every command below mirrors a verified live deployment
(validator-1 on GCP, validator-2 on Docker Desktop, observer on GCP).

Status: the n=2 procedures below record the previous live deployment. The
next acceptance target is four real validators on separate hosts plus four
real, non-voting observers on separate hosts. Four additional GCP VMs and the
existing O2 data disk were prepared on 2026-09-25; see
`../../docs/PHASE-C-FLEET.md`. No CREG node was started on those prepared hosts.
The four-validator deployment has not yet been rehearsed or deployed.

## 1. Prerequisites

- Docker with the `creg-node:genesis2` image (currently a retag of the
  genesis3 build; confirm tag with the release notes before joining).
- Two published ports: `8080` (HTTP API) and `9000` (libp2p P2P).
- A data directory the container user can write (bind-mount, see §3).
- On GCP: firewall ingress ALLOW tcp:9000 from `0.0.0.0/0` (validators dial
  each other directly). Port 8080 only needs VPC-internal reachability.
- Secrets, generated per the keygen ceremony and kept operator-side only:
  `CREG_VALIDATOR_KEY` (consensus signing — byte-exact, never repaste from
  memory), plus any `*KEY/*SECRET/*PRIV` lines from the env template.
  Never paste secret values into chat, tickets, or shared docs.

## 2. Quorum math and the four-validator target

Block quorum is `quorum_threshold(n, flag)` in `consensus/src/pbft.rs`:

| n | standard | with `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true` |
|---|---|---|
| 1 | 1 | 1 |
| 2 | 2 | 2 (flag has no effect) |
| 3 | 3 (unanimity!) | 2 |
| 4 | 3 | 3 |

The 2-of-3 flag is a documented testnet shortcut and is not the acceptance
path. Use four active validators with the flag disabled on every node. Normal
PBFT block finality and package approval both require 3-of-4 votes when all
four validators form the mandatory package committee. New-view authorization
also requires a 3-of-4 signed certificate carrying prepared-value evidence.
The `f + 1` (2-of-4) signal in original PBFT can prompt additional
view-change votes; it does not authorize a replacement proposer. Observers
never vote. The current n=2 deployment
stops if either validator is unavailable and is not the target topology.

## 3. Launch (manual `docker run` — the tested path)

Both live validators run via manual `docker run` with an env file (no
compose). This is the historical `genesis2` launch, not the Phase C release
command. Linux example (validator-1):

```bash
sudo docker run -d --name creg-validator-validator-1 --restart unless-stopped \
  --user "$(id -u):$(id -g)" \
  --group-add $(stat -c '%g' /var/run/docker.sock) \
  --env-file /home/samue/.creg-validator-validator-1/v1.env \
  -p 8080:8080 -p 9000:9000 \
  -v /home/samue/.creg-validator-validator-1/chain-data:/data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  creg-node:genesis2
```

Notes:

- Keep the env file OUT of `/tmp` (reboot-clears) — next to `chain-data`.
- `--group-add` for the docker socket is required for the sandbox engines;
  without it the node votes ABSTAIN on every package (verified failure mode).
- Windows (Docker Desktop): same flags minus `--user/--group-add`; the
  `/var/run/docker.sock` bind works as-is. **Write the env file as ASCII.**
  PowerShell `Set-Content` defaults to UTF-16, which Docker rejects with
  `invalid environment variable ... contains nul byte`. Use
  `Set-Content -Encoding Ascii` / `Add-Content -Encoding Ascii`, one line
  each — heredoc pastes can collapse newlines in the console.

## 4. Environment reference

Non-secret baseline (mirrors live v1/v2; adjust IDs, seeds, peers):

```env
CREG_TESTNET=true
CREG_NODE_ID=validator-3
CREG_DATA_DIR=/data
CREG_IS_VALIDATOR=true
CREG_NODE_ROLE=validator
CREG_ETH_RPC=https://ethereum-sepolia-rpc.publicnode.com
CREG_EXPECTED_L1_CHAIN_ID=11155111
CREG_CHAIN_ID=creg-testnet-1
CREG_STAKING_ADDR=0x30604Aa6f763fbde6aDCffbd8960af3031037331
CREG_REGISTRY_ADDR=0xD0F465ca185a99E268be0C4FF5A135Bdf89FAF3B
CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/chain-spec.json
CREG_SPEC_SIGNATURE_URL=https://spec.testnet.cregnet.dev/chain-spec.json.sig
CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev
CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000
CREG_P2P_SEEDS=/ip4/<v1>/tcp/9000/p2p/<v1-peer-id>,/ip4/<obs>/tcp/9000/p2p/<obs-peer-id>
CREG_PEERS=https://api.testnet.cregnet.dev
CREG_LISTEN=0.0.0.0:8080
CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false
CREG_PBFT_TIMEOUT=30
CREG_PUBLIC_VALIDATOR=true
CREG_DEV_SANDBOX=false
```

Key guidance:

- `CREG_P2P_SEEDS`: list at least two reachable peers. A seed entry for the
  node itself is harmless (libp2p reports "tried to dial local peer id").
- `CREG_PEERS`: HTTP sync sources, comma-separated. Validators seal locally
  via PBFT and rarely need this, but set it anyway for catch-up after
  restarts. Prefer VPC-internal URLs between GCP hosts
  (e.g. `http://10.142.0.4:8080`); keep the public edge as backup.
- `CREG_PBFT_TIMEOUT` is reserved for the unfinished view-change driver. The
  local Phase C release gate currently disables that driver and all
  replacement-view proposals. After safety recovery is implemented, backup
  leadership will require a 3-of-4 signed certificate. A local timeout alone
  does not authorize a proposal. The old
  `CREG_PROPOSER_FALLBACK_SECS` setting has no effect on the Phase C code.
- Validator data backups must include both `chain.rocksdb` and
  `pbft-safety.rocksdb` under `CREG_DATA_DIR`. The second database stores
  synced signing locks bound to genesis and validator identity. Preserve it
  across chain-data restore or reset; startup fails if its identity differs.
- Take contract addresses, spec signing key, and P2P seeds from the new
  verified signed spec before launch; the values above describe the old
  network and are not a ready-to-run Phase C configuration.
- Observers MUST set `CREG_PEERS` (validators' HTTP URLs). Sealed blocks
  reach non-participants only via the 10s HTTP sync loop; with no peers the
  observer sits at tip 0 forever and the explorer stays empty. This was the
  root cause of the "no transactions" outage.

## 5. Four-validator deployment preparation

1. Generate the keypair via the ceremony; record id, alias, eth address
   (lowercase `0x`), ed25519 pubkey. Publish the pubkey; file the secret.
2. Preserve the current divergent block hashes, logs, and data before any
   reset. Prepare one common, signed genesis/spec with all four voting
   validator identities. Record four separate non-voting observer node
   identities; observers must use the same signed chain spec but do not enter
   the voting validator set. Verify the spec signature on every host.
3. Place the validators on this laptop, the existing GCP validator VM, and
   two new GCP VMs in different regions. Keep the public observer on its
   current GCP VM, activate the unused dedicated observer VM, and place two
   more observers on new GCP VMs. Each of the eight nodes needs its own host,
   key, data, and network path. Keep
   `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false` everywhere. This is a real
   multi-region testnet under one provider/account; external operators remain
   a later decentralization milestone.
4. Launch the real nodes (§3). Wait for mesh convergence BEFORE
   submitting any transactions — votes cast during convergence can be lost
   (gossipsub has no delivery retry; a rebroadcast task now covers stuck
   rounds, but a quiet join is still safest).
5. Verify (§6). Submit a real benign package through a validator using the
   real security sandbox; confirm 3-of-4 package and block certificates, equal
   h1/h2 hashes at every validator, and sync to all four observers.

## 6. Verification commands

```bash
# Chain view (any node; validators use :8080, observer edge via Caddy)
curl -s http://localhost:8080/v1/public/chain/stats
# expect: tip_height advancing equally on all validators, spec_signature_verified:true

# Consensus state (validators)
curl -s http://localhost:8080/v1/validator/consensus/state
# expect: total_validators/quorum as configured, no stuck rounds,
#         peers labeled online/self (never offline while meshed)

# Mesh health (logs, not an endpoint)
docker logs <container> --since 10m 2>&1 | grep -c 'Connection established'
docker logs <container> --since 10m 2>&1 | grep -c 'Dropping message'
docker logs <container> --since 40m 2>&1 | grep 'banned for' | head -3
# expect: steady established lines, ~zero drops, zero bans

# Four-validator target: each block certificate needs at least 3 signatures
curl -s http://localhost:8080/v1/public/blocks/1 | grep -o '"validator_id":"[^"]*"'
```

## 7. Failure modes seen in the wild (symptom → cause → fix)

- **Rounds churn, `View-change limit exhausted`, flood of view-changes** →
  our votes aren't reaching quorum (peer lost them, or peer rejects our
  proposals). Check peer's drops/bans and heights first; do NOT wipe blindly.
- **`banned for Ns` lines** → rate limiter tripped (default limits assume a
  quiet mesh; a spinning proposer trips them in minutes). Bans are
  in-memory: restart clears them. Auto-ban now defaults OFF
  (`CREG_P2P_AUTO_BAN` opt-in).
- **Two nodes seal different blocks at one height** → inspect proposal and
  commit certificates before any restart. The Phase C code requires a quorum
  certificate for replacement leadership and now rejects a prepared-value
  reproposal that changes the original block hash. This fail-closed rule is
  not a complete failover path. Do not deploy Phase C until exact-block
  reproposal, durable vote/commit locks, and prepared-state recovery are
  resolved.
- **Observer stuck at tip 0 while validators advance** → `CREG_PEERS` unset
  or unreachable. Set to a validator's HTTP URL and recreate the observer.
- **Edge 502s from Caddy** → check upstreams: `docker exec <caddy> grep -c
  <suspect> /etc/caddy/Caddyfile`. NOTE: `sed -i` on a bind-mounted Caddyfile
  replaces the inode — the container keeps seeing the old file and `caddy
  reload` reports "config is unchanged". Recreate the container instead.
- **Validator votes ABSTAIN on everything** → inspect the real sandbox service,
  Docker socket permissions, and nsjail logs. Keep `CREG_DEV_SANDBOX=false` on
  every public validator; a bypass does not satisfy the live acceptance gate.
- **Windows `docker run` fails with `contains nul byte`** → env file is
  UTF-16. Rewrite ASCII, one `Add-Content` per line.

## 8. Backups that must exist (per validator)

- The env file WITH secrets (e.g. `validator.env`), stored once outside the
  container's reach (operator vault; local file as minimum).
- The raw validator key + a dated copy (`validator.key`,
  `validator.key.bak-YYYYMMDD`). Key bytes must be exact — any corruption is
  a different identity and consensus will reject it.
- Preserve chain data, logs, and block hashes before the coordinated reset
  so the divergent h1/h2 evidence remains available. P2P identity has so
  far survived wipes (deterministic from node id) but treat that as observed,
  not guaranteed.

## 9. Open items before mainnet-grade operation

- Real four-validator and four-observer deployment rehearsed on eight separate
  hosts, using the laptop and seven GCP VMs across multiple regions.
- Laptop validator reliability and connectivity measured during the soak.
- Full L1 stake → validate → rewards cycle observed end-to-end.
- Hardening PR deployed (vote rebroadcast, proposal guard, vote lock,
  no auto-ban, certified-set quorum) and soaked ≥48h.
