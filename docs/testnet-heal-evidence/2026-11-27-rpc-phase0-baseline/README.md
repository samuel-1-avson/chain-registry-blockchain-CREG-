# RPC Phase 0 baseline, 2026-11-27

Phase 0 of [docs/rpc-remediation-plan.md](../../rpc-remediation-plan.md). Everything here is read-only. Nothing on the VM or the public hosts was changed.

## Files

| File | What it is |
|---|---|
| `probe.json` | Output of `chain-registry/testnet/phase-c/probe-rpc-edge.mjs`, run from the operator PC with default options (no `--burst`). `startedAt` gives the exact time. |

Re-run from the repo root (Node 18+). Use a new file name each time, because the script refuses to overwrite:

```powershell
node chain-registry\testnet\phase-c\probe-rpc-edge.mjs --out docs\testnet-heal-evidence\<date>-rpc-probe\probe.json
```

## Result

**PASS 13, FAIL 33, WARN 1, INFO 13.** Findings not at target: R-01, R-06, R-08, R-14. That is what the plan predicts before Phases 1 and 2.

### Native CREG RPC (`api…/rpc`, `rpc…/rpc`, `api…/jsonrpc`)

- `creg_chainId`, `creg_blockNumber` and `creg_health` work on all three paths, and all return the same chain ID, `creg-phase-c-testnet-2026-09-26-r7`.
- **Tip age (WARN):** height 23, stamped 12:05:42Z, was 74 minutes old at probe time. `creg_health` has no field for it (R-06).
- **Protocol gaps:** a batch gets `-32700`; a positional array, a notification and a request without `jsonrpc` all get a normal result (R-08).
- `OPTIONS` gets Caddy's static descriptor with no `Access-Control-Allow-Origin` (R-09).
- The probe's 11 calls drew no 429.

### Sepolia gateways (`api…/sepolia`, `rpc…/sepolia`, `explorer…/rpc`, `faucet…/rpc`)

All four are open proxies (R-01, R-14):

- `eth_getLogs` and `web3_clientVersion` are answered.
- A batch containing a denied method is answered.
- A batch of 21 calls is answered.
- A 70 KiB body is accepted.
- The GET descriptors carry the pre-ECA-38 text.

`eth_chainId` is correct everywhere, and the heads were within 2 blocks of each other.

## What the evidence says about the upstream (task 0.5)

1. **One external pool sits behind all four surfaces.** Every response carries `server: cloudflare` and `via: 2.0 Caddy`, and `web3_clientVersion` alternates between `Geth/v1.17.9` and `reth/v2.11.0` on the same surface from one run to the next.
2. **Caddy talks to that pool directly.** `via: 2.0` means Caddy reached its upstream over HTTP/2. The explorer container serves plain HTTP on port 80, so it is not in the path for any of the four surfaces, `explorer…/rpc` included. Rebuilding the explorer image alone would therefore change nothing; the Caddy reload in Phase 1 is required.
3. **The owner is inferred, not confirmed.** A mixed-client, load-balanced pool behind Cloudflare looks like a public provider. The compose files default to `ethereum-sepolia-rpc.publicnode.com`, but I have not confirmed that is the provider. The `active_upstreams` line from the VM capture names the dial target.

## Trusted proxies (task 0.3)

**Answered from the repo, and it is worse than the plan assumed.**

- `CREG_TRUSTED_PROXIES` is set in **no** compose file, env file or deploy script. Only the node code reads it (`crates/node/src/rate_limit.rs:49`).
- Without it, the node rate-limits on its TCP peer. For traffic through Caddy that peer is the Caddy container, and for explorer traffic it is the explorer container.
- So all users share one 30/min bucket for native `/rpc` and one 3000/min bucket for `/v1` per entry path. One client sending 30 RPC calls a minute blocks native RPC for everyone for up to 60 s.
- The 36-call test I ran earlier today, and each probe run (11 calls), drew on that shared budget.
- Setting it is not enough on its own. The variable accepts exact IPs only, with no CIDR support, and Docker assigns container IPs dynamically. The fix needs a pinned Caddy IP or CIDR support (new finding R-23, plan task 2.10).

Not yet confirmed on the VM: the capture script prints `CREG_TRUSTED_PROXIES` for every node container. A two-network test is still worth doing after the fix, to confirm that clients get separate buckets.

## Deploy-path inventory (task 0.4)

| Role | Definition | Container | Status |
|---|---|---|---|
| Caddy | `docker-compose.unified.yml` + `caddy/Caddyfile.unified` | `creg-unified-caddy` | **Live family.** The live descriptors are older revisions of this file and of `sepolia-public-rpc.caddy` |
| Caddy | `docker-compose.cloud-edge-ingress.yml` + `Caddyfile.{bootstrap,hybrid,fleet}` | `creg-cloud-caddy` | Legacy |
| Caddy | `docker-compose.waitlist-edge.yml` + `Caddyfile.waitlist-only` | `creg-cloud-caddy` | Waitlist only |
| Explorer | `docker-compose.public-explorer-r27-stage.yml` | `creg-explorer-public-r27-stage` | **The one `Caddyfile.unified` routes to** |
| Explorer | `docker-compose.unified.yml` service `web-explorer` | `creg-unified-explorer` | Not routed by Caddy |
| Explorer | `docker-compose.public-explorer.yml`, `-r26.yml` | `creg-explorer-public-r25`, `-r26` | Earlier stages |
| Explorer | `docker-compose.cloud-edge.yml` | `creg-cloud-explorer` | Legacy; mounts `nginx/explorer-hybrid.conf` over the image config, so no allowlist (R-13) |

**Proposed canonical path:** `docker-compose.unified.yml` for Caddy, plus the r27-stage compose for the explorer. Everything else is legacy until retired in Phase 6. The VM capture confirms which containers actually run.

## Still to do in Phase 0

1. **0.1 / 0.5, on the VM.** Copy `chain-registry/testnet/gcp/capture-rpc-edge-state.sh` to the VM and run `bash capture-rpc-edge-state.sh`. Share only `summary.txt`; the full copies can contain secrets.
2. **Decisions 1, 2 and 5** in the plan. Decision 5 is answered once the capture names the dial target.
