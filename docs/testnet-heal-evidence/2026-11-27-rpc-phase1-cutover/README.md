# RPC Phase 1 cutover, 2026-11-27

Phase 1 of [docs/rpc-remediation-plan.md](../../rpc-remediation-plan.md): the public Sepolia RPC surfaces now enforce the ECA-38 method allowlist, and the node's `/metrics` is no longer public.

## What ran

| When (UTC) | Step | Result |
|---|---|---|
| 16:46 | `testnet/gcp/capture-rpc-edge-state.sh` on `creg-testnet-vm` (read-only) | Found the live routing: Caddy proxied straight to publicnode, and r28 is the live explorer |
| 16:53 | `testnet/phase-c/stage-r28-allowlist.sh` | A copy of r28 on `127.0.0.1:3031` passed 12 of 12 allowlist checks |
| 17:01 | `testnet/phase-c/cutover-r28-allowlist.sh` (files from commit `3964a40`) | `CUTOVER-OK`, stamp `20261127T170110Z` |
| 17:01 | `testnet/phase-c/probe-rpc-edge.mjs` from the operator PC | `probe.json` in this folder |
| 17:02 | HTTP status checks of every public host | All as expected (table below) |

What the cutover changed:
1. **Explorer:** recreated `creg-explorer-public-r28` (same image, `ed857d58…`) with `CREG_SEPOLIA_RPC_UPSTREAM=https://ethereum-sepolia-rpc.publicnode.com/`. Before, it pointed at the edge itself. The previous container is kept, stopped, as `creg-explorer-public-r28-before-allowlist`.
2. **Caddy:** replaced the VM's `Caddyfile.unified` and `sepolia-public-rpc.caddy` in place with the repo versions. The originals are saved with suffix `.pre-rpc-phase1-20261127T170110Z`. Caddy was validated and reloaded.

**Rollback**, on the VM: `bash ~/creg-rpc-phase1/rollback-r28-allowlist.sh 20261127T170110Z`

## Result

**Probe: PASS 41, FAIL 5, INFO 14** (baseline: PASS 13, FAIL 33). Every R-01 and R-14 check passes on all four surfaces: `api…/sepolia`, `rpc…/sepolia`, `explorer…/rpc` and `faucet…/rpc`.

| Check | Before | After |
|---|---|---|
| `eth_getLogs` | 200, logs returned | 403 `method not allowed` |
| `web3_clientVersion` | 200 (Geth or reth) | 403 |
| Batch with a denied method | 200 | 403 |
| Batch of 21 | 200 | 400 |
| Malformed body | upstream's 400 | gateway's 400 `invalid JSON-RPC request` |
| 70 KiB body | 200 | 413 |
| `GET explorer…/rpc` | 200 descriptor | 405 |
| Response path | `server: cloudflare`, `via 2.0 Caddy` (straight to publicnode) | `server: nginx`, `via 1.1 Caddy` (through the allowlist) |
| `eth_chainId`, `eth_blockNumber` | 200 | 200, unchanged |

The 5 remaining FAILs are native-node checks (R-06 tip age; R-08 batch, positional array, notification, missing `jsonrpc`). They pass only after the Phase 2 node image is deployed.

Other public endpoints after the cutover:

| URL | Status |
|---|---|
| `api…/metrics` | **403** (was 200, so R-24 is closed) |
| `api…/v1/health`, `api…/health`, `api…/` | 200 |
| `explorer…/`, `explorer…/v1/public/health`, `explorer…/v1/public/chain/stats` | 200, and the page renders live chain data |
| `faucet…/` | 200 |
| `spec…/chain-spec.json` and `.sig` | 200 |

## Not changed

- The Sepolia provider is still publicnode. Moving to CREG-run nodes is Phase 3 (3.3, 3.4).
- `CREG_TRUSTED_PROXIES` is still unset on o1. Setting it to `172.21.0.0/16` needs the Phase 2 (CIDR-capable) node image.
- The VM checkout is still the September commit with local edits, and the unrouted explorers r25, r25-canary, r26 and r27-stage still run (R-25, Phase 6).
