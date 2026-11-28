# RPC Phase 2 deploy to observer o1, 2026-11-27

Phase 2 of [docs/rpc-remediation-plan.md](../../rpc-remediation-plan.md): the public observer `creg-phase-c-r7-o1` on `creg-testnet-vm` now runs the node with the Phase 2 JSON-RPC and rate-limit changes, and it rate-limits per real client.

## Image

| | Before (v4) | After |
|---|---|---|
| Reference | `creg-node:r7-auto-admission-20261127-v4` | `creg-node:r7-rpc-phase2-20261127` = `creg-node@sha256:403144636cb14310899a460b78d6c6df327918049ae32a14ea4867b504b32430` |
| Source revision label | `e1442213…` (`e144221`) | `b02c76576a46e2fe0a876751e65214ec331ae2ef` (`b02c765`, which is `e144221` plus the RPC commits) |
| Dockerfile target | `node-runtime` | `node-runtime` |
| Other labels | features `phase-c-real-fleet`, Rust 1.98.1, Ubuntu 24.04 base `sha256:a853f94d…`, default spec-signing key | identical |

**How it was built:** from a clean LF worktree (`git -c core.autocrlf=false worktree add --detach … b02c765`), so no uncommitted edits reached the image. It used Docker Desktop on the operator PC with the Dockerfile default of 2 cargo jobs, and took 2 h 30 m. The Dockerfile's chain-spec signature check passed. The image was then pushed to Artifact Registry.

**A first attempt used 16 cargo jobs and failed.** It ran the 7.4 GiB Docker VM out of memory and hung Docker Desktop. All five validator containers on the PC went down with it (v1, v2 and three external validators) for roughly 18:10–19:10 UTC, until Docker Desktop was fully quit and reopened. The chain was idle at tip 23 the whole time, so no finalized work was lost. The successful 2-job build ran alongside a watchdog that checked v1 and v2 every minute; they stayed healthy throughout.

## Swap

`testnet/phase-c/replace-o1-image.sh` cloned o1's configuration from `docker inspect`:
- 64 env settings;
- 4 mount arguments (2 bind mounts, each a flag pair);
- ports 18080→8080 and 9001→9000;
- both networks with the `creg-phase-c-r7-o1` alias;
- user 10002:10002 and restart `unless-stopped`.

It added `CREG_TRUSTED_PROXIES=172.21.0.0/16`, the Docker network that holds Caddy and the r28 explorer. The firewall keeps the node ports VPC-internal, so trusting that range is safe.

| When (UTC) | Event |
|---|---|
| 21:47:03 | Old o1 stopped, kept as `creg-phase-c-r7-o1-before-20261127T214703Z` |
| before 21:47:49 | New o1 up: tip 23 on `creg-phase-c-testnet-2026-09-26-r7`, spec verified, `creg_health` reports `tip_age_secs`, batches answered |
| 21:48:02 | `ensure-r7-o1-data-pipes.sh` passed: `signed_r7_o1`, `real_history` (61 samples), `private_explorer_same_tip` |
| 21:48:39 | Public probe with `--burst` (`probe.json` in this folder) |

**Rollback**, on the VM: `bash ~/creg-rpc-phase2/rollback-o1-image.sh 20261127T214703Z`

## Result

**Probe: PASS 47, FAIL 0, WARN 0, INFO 14.** The morning baseline was PASS 13 / FAIL 33, and after Phase 1 it was PASS 41 / FAIL 5.

| Check | After Phase 1 | Now |
|---|---|---|
| `creg_health` reports tip age (R-06) | FAIL | PASS: adds `tip_timestamp`, `tip_age_secs`, `pending_pool_size`, `peer_count`, `sync` |
| Batch answered (R-08) | FAIL (`-32700`) | PASS (array of 2) |
| Positional array rejected (R-08) | FAIL (answered) | PASS (`-32600` per member) |
| Notification gets no reply (R-08) | FAIL (answered) | PASS (HTTP 204) |
| Missing `jsonrpc` rejected (R-08) | FAIL (answered) | PASS (`-32600`) |
| 31 reads in a minute (R-03) | 429 from call 31 | PASS, 31 of 31 returned 200 |
| Sepolia allowlist, all four surfaces (R-01, R-14) | PASS | PASS |

Public endpoints afterwards: `api…/metrics` returns 403. The API health endpoints, explorer page, explorer `/v1`, faucet and chain spec all return 200. The PC validators v1 and v2 answer 200.
