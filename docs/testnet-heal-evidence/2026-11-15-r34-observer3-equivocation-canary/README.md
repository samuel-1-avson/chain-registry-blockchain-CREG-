# r34 node image: equivocation-detection canary on observer-3 (2026-11-15)

**Result: passed.** Observer-3 runs the r34 node image with equivocation detection turned on (ADR-016 §11, detection only, no slashing). Its `node.env`, RPC configuration, ports, mounts and P2P identity are unchanged. No validator, other observer or DNS entry was touched.

**Not yet shown:** that the detector stays quiet under real PBFT traffic. The chain has produced no block since late September, so in the first minutes the detector observed 0 votes. ADR-016 §11 step 5 (watch at least one day of PBFT traffic) is still open; see [What is still open](#what-is-still-open).

## Image

| | |
|---|---|
| Source | `d67e9204e5963bcc981e7e51290d011e0b957052` on `phase1/hardening-and-tests`: the r33 source (`eea0e1ca`, merge of #59) plus 20 commits, of which the code commits are strict Ed25519 verification, the observer journal-warning fix, golden-vector and message-format tests, the rebuilt equivocation detector and its node wiring, and the faucet fixes (the faucet is not in this image) |
| Cloud Build | `841d0700-02d5-4eff-af91-a5915fafc676`, project `gen-lang-client-0022105784`, `us-central1`. SUCCESS, 08:39–09:40 UTC, Rust 1.93.0, `node-runtime` target, `phase-c-real-fleet` |
| Tag | `us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:phase-c-equiv-r34-rust193-d67e9204e596` |
| Registry digest | `sha256:47503cb12e40905fac36e532100d1c58fd968daa7fa7ba064a178717fb9e1953` |
| Config digest (image ID on the VM) | `sha256:7b954e40529abe4ed7b3b01e94ed5a7732f69a2a97bbc3dcfc7f50e525ac8249`, matched against the registry manifest |
| Source manifest SHA-256 | `e484ebbd3d0fa79f5c05f04f54c7d33bb5b4519d45b5716adf483d98d8761dc4` (240 files), also carried as an image label |
| `--version` | `creg-node 0.1.0` |
| `/app/creg-node` SHA-256 | `2171ba8162ba54c93cf242a5838a934deb7ead3bc9dc9690865799260323f466`: recorded by Cloud Build, checked on the laptop after the pull, on observer-3 after loading, and in the running container |

Observer-3 has no Artifact Registry read access. The image was pulled on the maintainer laptop through a throwaway Docker config that used the gcloud credential helper (the laptop's own Docker config was not changed), saved (397 MB, `c807b572…`), copied over IAP, checked against the same hash on the VM, and loaded. No credential was placed on the VM. Build and transfer record: [`build-and-transfer.json`](./build-and-transfer.json).

## Swap

Host `creg-observer-3` (`fiffi-500411`, `us-west1-b`), container `creg-phase-c-r7-o3`, swapped at 09:49:22 UTC with [`scripts/o3-swap-r34.sh`](./scripts/o3-swap-r34.sh).

What was kept the same:
- The env file `/var/lib/creg/phase-c-r7/node.env`, byte for byte (SHA-256 checked before and after). Detection was turned on with `docker run -e CREG_EQUIVOCATION_DETECTION=true`, so the file needs no edit and the old container needs none to come back.
- Ports `8080` and `9000`.
- The data bind mount, read-write.
- The scanner mount `scanner-r14-ansi-regex-live`, read-only.
- Restart policy `unless-stopped`.

What was preserved for rollback:
- The r33 container, `creg-phase-c-r7-o3-r33-before-r34`, stopped (image `dbc881f36615…`).
- A data copy, `/var/lib/creg/phase-c-r7/data-before-r34-20261115T094922Z`, taken while the node was stopped and checked with `diff -r`.

## Readiness and detection checks (all passed)

| Check | Result |
|---|---|
| New image running, binary matches the build | yes |
| Signed chain spec verified, genesis `0f2a3b10…` unchanged | yes |
| Chain not behind: tip and finalized height 20, tip hash `730b01c1…` unchanged | yes |
| Packages kept (8) | yes |
| 4 of 4 validators | yes |
| Observer role, peers > 0 (2) | yes |
| Scanner profile `7e1d95f9…` unchanged and ready | yes |
| Durable work, pending and forced-inclusion queues clear; PBFT rounds clear | yes |
| P2P identity unchanged | yes |
| `node.env` unchanged | yes |
| Container carries `CREG_EQUIVOCATION_DETECTION=true` | yes |
| Startup log: `Equivocation detection enabled (detection only, no slashing; ADR-016) loaded=0` | yes |
| `creg_equivocation_detection_enabled 1` | yes |
| `creg_equivocation_evidence 0`, `creg_equivocation_persist_failures_total 0` | yes |
| `GET /v1/public/evidence/equivocation`: `detection_enabled: true`, `count: 0`, `evidence: []` | yes |

Full evidence JSON: [`r34-o3-equivocation-canary-20261115T094922Z.json`](./r34-o3-equivocation-canary-20261115T094922Z.json) (also on the host under `/var/lib/creg/phase-c-r7/evidence/`).

## Soak check, two minutes after the swap

From [`scripts/o3-soak-r34.sh`](./scripts/o3-soak-r34.sh) (read-only):

- Healthy, 0 restarts. Height 20, 2 peers, 4 of 4 validators, 8 packages.
- 117 log lines: 110 INFO, 7 WARN, 0 ERROR. The 7 WARN lines are all known and unrelated: validator-set sync did not reach staking-log quorum (5), only one L1 RPC endpoint is configured (1), and legacy checkpointing is disabled (1).
- **The observer journal warning is gone.** The r33 container logged "PBFT signing is blocked because its durable safety journal is not initialized" 125 times in its last 10 minutes; the r34 container has logged it 0 times. This confirms the fix in `702050c` on a live observer.
- Detector counters all 0: votes observed, rejected, pending and unresolved; cached headers; persist failures. The `equivocation-evidence/` directory does not exist yet. It is created only when there is evidence to write.

## Unchanged, as expected

- **Validator-set sync is still degraded** (single RPC provider without the historical Staking logs). Same as r33.
- **`creg_chain_tip_age_seconds` ≈ 4,149,000** (about 48 days). The chain is idle, so no PBFT messages reach the detector.

## What is still open

1. **Traffic.** ADR-016 §11 step 5 needs PBFT traffic. An idle chain produces none, so the detector has not yet processed a single vote. One canary publish would produce a full PREPARE/COMMIT round for observer-3 to watch. After it, expect `votes_observed` > 0, `votes_rejected` and `pending_votes` near 0, and `creg_equivocation_evidence` still 0.
2. **A day of watching.** Re-run `scripts/o3-soak-r34.sh` on the host after 24 hours and after any traffic.
3. **Optional staging drill** (§11 step 6): a deliberate double-sign on a staging copy, to see one piece of evidence that `verify()` accepts.
4. **Source not on GitHub yet.** Commit `d67e9204` is on the local branch `phase1/hardening-and-tests`. It should be pushed and merged so the image's source revision is public.

## Rollback

Detection writes nothing the r33 binary reads, so no data step is needed. On the host:

```bash
bash /tmp/o3-rollback-r34.sh
```

It stops the r34 container, renames it to `creg-phase-c-r7-o3-r34-failed-<ts>`, renames `creg-phase-c-r7-o3-r33-before-r34` back to `creg-phase-c-r7-o3` and starts it. To also restore the pre-swap data copy, pass the backup path:

```bash
bash /tmp/o3-rollback-r34.sh /var/lib/creg/phase-c-r7/data-before-r34-20261115T094922Z
```

The script is also in this folder: [`scripts/o3-rollback-r34.sh`](./scripts/o3-rollback-r34.sh).
