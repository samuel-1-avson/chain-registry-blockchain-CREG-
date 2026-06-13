# L1/L2 Hardening — Configuration & Runbook

This document covers the hardening work from the L1/L2 architecture review and
how to operate it. It is the reference for the new environment knobs and for
splitting the validator set across hosts.

## Summary of changes

| Area | Mechanism | Default |
|------|-----------|---------|
| L2 reorg handling | Fork detection + longest-valid-chain recovery, recorded to `/v1/reorgs` | always on |
| L1 checkpoint anchors | Persisted anchor journal with real L1 tx hashes (`/v1/bridge/anchors`) | always on |
| Validator-set finality lag | `CREG_VALIDATOR_SET_FINALITY_LAG` | 2 (testnet), 6 (mainnet) |
| Multi-RPC L1 quorum | `CREG_ETH_RPC_FALLBACKS` | unset (single RPC + warning) |
| Proposer-failure fallback | `CREG_PROPOSER_FALLBACK_SECS` | 2 × block interval |
| Bridge self-approve guard | `CREG_BRIDGE_SELF_APPROVE` | true |
| Relayer CORS allowlist | `RELAYER_ALLOWED_ORIGINS` | unset (any origin + warning) |
| Relayer proxy trust | `RELAYER_TRUST_PROXY` | false |
| Relayer nonce persistence | `RELAYER_DATA_DIR` | `.` |

## Node environment knobs

### `CREG_ETH_RPC_FALLBACKS` (multi-RPC quorum)
Comma-separated list of **additional independent** Sepolia RPC endpoints used
alongside `CREG_ETH_RPC` for validator-set sync. With 3+ total endpoints the
head block height is taken as the median (one outlier cannot move it), and the
cursor block hash used for reorg detection must reach a **strict majority**
before any rebuild — so a single stale or malicious RPC can no longer skew
membership or force a destructive resync.

```
CREG_ETH_RPC=https://sepolia.infura.io/v3/KEY1
CREG_ETH_RPC_FALLBACKS=https://ethereum-sepolia-rpc.publicnode.com,https://rpc.sepolia.org
```

Use endpoints from **different providers**. Two co-located endpoints from the
same provider do not add Byzantine resistance.

### `CREG_VALIDATOR_SET_FINALITY_LAG`
L1 blocks to trail head before applying staking events. Raising it makes
membership changes slower but more robust to shallow Sepolia reorgs. `0` is
discouraged (logs a warning).

### `CREG_PROPOSER_FALLBACK_SECS` (liveness)
How long the chain tip may stall before the next-ranked proposer steps in.
Each elapsed window promotes one more fallback rank, so a single offline
proposer no longer halts block production. Set conservatively above the block
interval (default is 2× the interval) to avoid two proposers racing the same
height during normal operation.

### `CREG_BRIDGE_SELF_APPROVE`
When `true` (default) the bridge votes `approve` on its own rollup-checkpoint
proposal — required for liveness while governance runs at threshold 1. Once an
independent second signer co-approves checkpoints, set this to `false` so the
bridge key alone can no longer both propose and execute. The node also logs a
warning whenever the on-chain governance threshold is 1.

## Relayer environment knobs

### `RELAYER_ALLOWED_ORIGINS`
Comma-separated exact CORS origins. Leave unset only in development; production
must pin the explorer origin(s):

```
RELAYER_ALLOWED_ORIGINS=https://explorer.testnet.cregnet.dev
```

### `RELAYER_TRUST_PROXY`
Only set to `true` when the relayer sits behind a trusted reverse proxy that
sets `X-Forwarded-For`/`X-Real-IP`. When `false` (default) the real TCP peer
address is used for per-IP quotas, so clients cannot spoof headers to evade
rate limits.

### `RELAYER_DATA_DIR`
Directory for the sponsor-nonce journal (`sponsor-nonces.json`). Persisting it
means a relayer restart cannot reset a per-owner nonce to 0 and accept a
stale/replayed sponsored-stake intent. Point this at a persistent volume.

The relayer also now recovers the ERC-2612 permit signer **off-chain** and
rejects the request before spending gas if it does not match the owner.

## Splitting the validator set across hosts

The protocol already supports independent validators; the current testnet
co-locates two on one VM, which is the main decentralization gap. To split:

1. **Separate keys (already separate):** each validator has its own Ed25519
   consensus key (`CREG_VALIDATOR_KEY_*`) and its own EVM staking key. Never
   share a key file between hosts.
2. **Separate host / zone:** run the second validator from
   `testnet/docker-compose.validator-fleet.yml` (or `observer-pool.yml` for a
   read replica) on a **different VM and ideally a different zone**. Point it at
   the shared edge IPFS/spec endpoints via env, not at localhost.
3. **Independent L1 view:** give each validator its own `CREG_ETH_RPC` plus
   `CREG_ETH_RPC_FALLBACKS` so they do not share a single L1 trust root.
4. **Register identity on every node:** POST `/v1/validators/register` to each
   node and the observer pool so L1 `Active` status and the API
   `active_validators` count agree (avoids the identity-drift incident class
   documented in `testnet/OPERATOR.md`).
5. **Raise governance threshold:** move `GOVERNANCE_THRESHOLD` to ≥ 2 with
   signer keys held by different operators, then set
   `CREG_BRIDGE_SELF_APPROVE=false`.
6. **Verify quorum:** confirm `>= 2` active validators and PBFT progress
   (NET-301) before relying on the split for fault tolerance.

## Verification checklist

- `GET /v1/reorgs` returns recorded events after an induced fork (no longer a
  permanent empty list).
- `GET /v1/bridge/anchors` shows real `l1_tx_hash` values and a growing
  `anchor_count`.
- `validator_set_sync` logs `L1 quorum reads enabled across N RPC endpoints`.
- Block production continues when the primary proposer is stopped (fallback
  rank logs `Proposer fallback engaged`).
- Relayer logs the configured CORS origin count and `Trust proxy hdr: false`
  unless explicitly enabled, and `Restored nonces: N` after a restart.
