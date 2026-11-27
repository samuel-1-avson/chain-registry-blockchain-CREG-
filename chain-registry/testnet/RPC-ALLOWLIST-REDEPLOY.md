# Make the live edge enforce the Sepolia RPC allowlist (ADR-019 ECA-38)

Written 2026-11-27 from the repo and from read-only probes of the public hosts. Nothing here has been run on the VM. Every command below is a proposal to check before you use it.

## Why this is needed

Probes of the live hosts on 2026-11-27 showed that the method allowlist in `explorer/rpc-allowlist.js` is **not** enforced:

| Probe | Expected with allowlist | Seen live |
|---|---|---|
| `POST api…/sepolia` `eth_getLogs` | 403 | 200, logs returned |
| `POST api…/sepolia` `web3_clientVersion` | 403 | 200 |
| `POST api…/sepolia` batch | 200 only if every method is allowed | 200 |
| `POST explorer…/rpc` `eth_getLogs` | 403 | 200, logs returned |
| `GET explorer…/rpc` | 405 | 200 with a JSON descriptor |
| `GET api…/sepolia` descriptor text | "allowlisted" | "POST JSON-RPC 2.0 eth_* methods" (no "allowlisted") |

The committed `caddy/Caddyfile.unified` is correct: `/sepolia` goes to the explorer container, and the explorer vhost proxies to that container with no direct RPC import. The running Caddy and explorer image are older than the repo. ADR-019 already lists this as remaining gap 3.

Target path after the change:

```
client -> Caddy (64 KB body cap, loop guard) -> explorer container (njs allowlist, 10 r/s burst 20) -> one real Sepolia node
```

## 0. Read-only checks on the VM first

`gcp/capture-rpc-edge-state.sh` runs all of these, plus the active Caddy config and each node's `CREG_TRUSTED_PROXIES`, and prints a summary with URLs reduced to scheme and host. The commands below are the manual equivalent.

The Phase 0 probe (2026-11-27) showed `via: 2.0 Caddy` on all four Sepolia surfaces: Caddy proxies them straight to an external HTTPS pool, and the explorer container is not in the path. A new explorer image alone therefore changes nothing; step 4 (the Caddy reload) is what closes the proxy.

Run these before changing anything, and keep the output for rollback.

```bash
docker ps --format '{{.Names}}\t{{.Image}}'
docker inspect creg-explorer-public-r27-stage --format '{{.Config.Image}}'
docker exec creg-explorer-public-r27-stage ls /etc/nginx/njs/rpc-allowlist.js
docker exec creg-explorer-public-r27-stage grep -c 'rpc.proxy' /etc/nginx/conf.d/default.conf
docker exec creg-explorer-public-r27-stage printenv CREG_SEPOLIA_RPC_HOST
```

- If the `ls` or `grep` fails, the running explorer image predates Increment 1, so step 2 is required.
- If both succeed, the image is fine and the open path is the running Caddyfile. Compare it with the repo's `Caddyfile.unified`; the explorer vhost must not contain `import sepolia-public-rpc.caddy`.
- Do not paste `CREG_SEPOLIA_RPC_UPSTREAM` anywhere shared. A provider URL can embed an API key. The host name is enough.
- Save copies of the current `Caddyfile`, `sepolia-public-rpc.caddy` and the compose files, and record the current explorer digest.

## 1. Choose the upstream

The explorer container proxies to **one** URL (`CREG_EXPLORER_SEPOLIA_RPC_UPSTREAM`, host in `CREG_EXPLORER_SEPOLIA_RPC_HOST`). The compose default is the third-party `ethereum-sepolia-rpc.publicnode.com`.

- Preferred: a CREG-run node (Geth node A, or Nethermind node B once it is synced). Use its internal address, for example `http://<internal-ip>:8545/`. Keep the trailing `/`, because the nginx config forwards the URI as configured.
- Never use `api…/sepolia` or the explorer's own `/rpc`. The edge refuses that loop with HTTP 508.
- The VM running the explorer must be allowed to reach port 8545 on the node (the README says the firewall allows the fleet allowlist and 10.96.0.0/16). The node A and node B VMs sit in different regions from the unified VM, so confirm routing and firewall rather than assuming them.
- Geth rejects requests whose `Host` header is a name it does not know. Set `CREG_EXPLORER_SEPOLIA_RPC_HOST` to the node's IP or to a name its `--http.vhosts` accepts. Step 3 shows quickly whether it works.
- A single upstream is a single point of failure for wallet RPC. The quorum and failover logic in `common::l1_rpc` is for node-internal reads only; the gateway does not use it.

## 2. Build and push the explorer image (skip if step 0 showed the allowlist is in the running image)

1. From `chain-registry/explorer`, run the allowlist unit tests the header of `rpc-allowlist.js` refers to (`rpc-allowlist.test.js`, under `node --test`).
2. Build with the **same** `VITE_*` build arguments as the image now in production. The Dockerfile bakes `VITE_API_BASE`, `VITE_SEPOLIA_*`, `VITE_STAKING_ADDR`, `VITE_NETWORK` and others into the JS bundle, so they cannot be read back from the running container. I found no script in the repo that records them for the deployed digest, so recover them from your last build. If they differ, the explorer's behaviour changes along with the RPC fix.
3. Push to `us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer` and note the new `sha256:` digest.
4. Put the digest in `docker-compose.public-explorer-r27-stage.yml`. The `web-explorer` service in `docker-compose.unified.yml` (line 166) pins a different digest and runs as `creg-unified-explorer`. Caddy does not route to it, but update it too so a later `compose up` does not bring back an open-proxy image.

## 3. Stage the new explorer container, with no public traffic

Follow the repo's stage pattern (see `phase-c/repair-staged-explorer-verified-rpc.sh`): start the new container on a loopback port and keep the previous container until the checks pass.

```bash
BASE=http://127.0.0.1:3029   # or the canary port you chose
rpc() { curl -s -w ' [HTTP %{http_code}]\n' -X POST "$BASE/rpc" -H 'content-type: application/json' -d "$1"; }

rpc '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
rpc '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":2}'
rpc '{"jsonrpc":"2.0","method":"eth_getLogs","params":[{"fromBlock":"latest"}],"id":3}'
rpc '{"jsonrpc":"2.0","method":"web3_clientVersion","params":[],"id":4}'
rpc '[{"jsonrpc":"2.0","method":"eth_chainId","id":5},{"jsonrpc":"2.0","method":"eth_getLogs","id":6}]'
rpc 'not json'
curl -s -o /dev/null -w 'GET %{http_code}\n' "$BASE/rpc"
head -c 70000 /dev/zero | tr '\0' 'a' | curl -s -o /dev/null -w 'big body %{http_code}\n' -X POST "$BASE/rpc" -H 'content-type: application/json' --data-binary @-
seq 60 | xargs -P 30 -I{} curl -s -o /dev/null -w '%{http_code}\n' -X POST "$BASE/rpc" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' | sort | uniq -c
```

Expected, from the ADR-019 edge validation:

| Call | Result |
|---|---|
| `eth_chainId` | 200, `0xaa36a7` |
| `eth_blockNumber` | 200 |
| `eth_getLogs` | 403 `method not allowed: eth_getLogs` |
| `web3_clientVersion` | 403 |
| batch with one denied method | 403 |
| `not json` | 400 |
| GET | 405 |
| 70 KB body | 413 |
| 60-call burst | a mix of 200 and 429 |

If `eth_chainId` returns an upstream 403 or a Host error, fix `CREG_EXPLORER_SEPOLIA_RPC_HOST` or the node's firewall before going further.

Then run each client once against the staged container (see step 6).

## 4. Promote and reload Caddy (in this order)

1. Promote the new explorer container under the name Caddy expects (`creg-explorer-public-r27-stage:80`, the default of `CREG_EXPLORER_RPC_GATEWAY`).
2. Copy **both** `caddy/Caddyfile.unified` and `caddy/sepolia-public-rpc.caddy` to the VM. `gcp/deploy-unified-stack.ps1` uploads only `Caddyfile.unified`, so the faucet vhost's `/rpc` would keep the old open-proxy copy.
3. Validate and reload. Do not run `deploy-unified-stack.ps1` as the way to do this. It also recreates every service from `docker-compose.unified.yml` (including the `creg-node:genesis2` observer), and the live stack has already diverged from that file (r7 observer, r27-stage explorer). Never pass its `-TearDownRedundantResources` flag: it deletes the VM named `creg-sepolia-geth-vm`.

```bash
CADDY=<name from docker ps>
docker exec "$CADDY" caddy validate --config /etc/caddy/Caddyfile
docker exec "$CADDY" caddy reload   --config /etc/caddy/Caddyfile
```

The order matters: if Caddy reloads while the explorer still forwards to the edge, `/sepolia` answers 508.

## 5. Verify from outside

From the repo root on any machine with Node 18+, run the probe and compare it with the Phase 0 baseline (`docs/testnet-heal-evidence/2026-11-27-rpc-phase0-baseline/`). Every `R-01` and `R-14` line should now say PASS:

```powershell
node chain-registry\testnet\phase-c\probe-rpc-edge.mjs --out docs\testnet-heal-evidence\<date>-rpc-phase1\probe.json
```

It checks the following per host:

| Host | Call | Expected |
|---|---|---|
| `api…/sepolia` and `rpc…/sepolia` | `eth_chainId` | 200, `0xaa36a7` |
| | `eth_getLogs` | 403 |
| | GET | 200, descriptor now says "allowlisted" |
| `explorer…/rpc` | `eth_getLogs` | 403 |
| | GET | 405 |
| `api…/rpc`, `rpc…/rpc`, `/jsonrpc` | `creg_blockNumber` | 200, unchanged (separate Caddy matcher) |
| `faucet…/rpc` | `eth_getLogs` | 403 |

**Per-client rate limit.** ADR-019 says the realip path was not exercised: nginx keys `limit_req` on the recovered client IP, and it trusts `X-Forwarded-For` only from private peers. If the traffic reaches Caddy through a load balancer, Caddy may record the balancer's address, and then every user would share one 10 r/s bucket. Test it: burst from one network until you see 429s, then from a second network send a single call. It must still return 200.

## 6. What the allowlist can break

| Caller | Methods | Effect |
|---|---|---|
| `creg testnet stake-*` (Foundry `cast`) | chainId, nonce, fees, estimateGas, `eth_sendRawTransaction`, receipt | Allowed (the allowlist was written for it) |
| `creg doctor` | `eth_chainId`, `eth_blockNumber`, `eth_getCode` | Allowed |
| `validator_onboarding.js` | `eth_call`, `eth_getBalance`, receipts | Allowed |
| `testnet/approve-validator.js`, `stake-validator.js` (ethers `JsonRpcProvider`; the first defaults to `explorer…/rpc`) | No `getLogs` or `queryFilter` found | Probably fine. ethers v6 batches concurrent calls (default up to 100) and the gateway rejects batches over 20, so run each script once against the staged container |
| Wallet "add network" | Standard reads and sends | Allowed. dApps that rely on `eth_getLogs`, `eth_getStorageAt`, `eth_syncing` or `web3_clientVersion` now get 403. Tell anyone who uses the public RPC |
| Faucet service | Its own `FAUCET_RPC_URL`; `FAUCET_PUBLIC_RPC_URL` is only shown to wallets (`faucet/src/main.rs:4097`) | Unaffected |
| Node, relayer, indexer, external validators | `CREG_ETH_RPC` directly; they need `eth_getLogs` and the quorum reads | **Must not point at the gateway.** The external validator scripts default to a third-party URL, not the CREG edge. On the VM, search the fleet env files for `cregnet.dev/sepolia` and `cregnet.dev/rpc` |

## 7. Rollback

- Caddy: restore the saved `Caddyfile` and `sepolia-public-rpc.caddy`, then `caddy reload`.
- Explorer: recreate the container from the digest you recorded in step 0.
- Roll back if legitimate wallet or staking calls get 403, normal users get 429, or any host returns 508.

## 8. Afterwards

- Update ADR-019 remaining gap 3, and add the date and the probe results.
- Still open after this change: the legacy cloud-edge nginx configs (`testnet/nginx/explorer-{bootstrap,fleet,hybrid}`) proxy `/rpc` without the allowlist; the gateway has a single upstream and no quorum; the self-hosted RPC gateway (Inc 6) is not built.
- The observer's local env file sets `CREG_ETH_RPC` to a third-party host with no fallbacks. That is separate from the gateway but has the same self-hosting gap.
