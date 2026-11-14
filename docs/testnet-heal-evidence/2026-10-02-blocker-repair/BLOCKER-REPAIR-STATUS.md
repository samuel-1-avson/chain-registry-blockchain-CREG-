# R7 blocker repair — 2026-10-02

## Completed this run

- Captured all eight real nodes before recovery. All agree on signed R7, finalized height 20, 21 stored blocks and tip `730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285`. Recovery queues and unresolved PBFT rounds are clear.
- Recovered V4's stalled L1 sync with one guarded restart of its existing container. Its last poll had remained at 2026-09-29 14:03:48 UTC. After restart, sync was `synced`, cursor and finalized source block were 11,827,708, and the poll timestamp was 2026-10-02 08:11:28 UTC.
- Verified identical container/image IDs, persisted data mount, P2P identity hash, environment hashes, h20 hash and retained recovery counts before/after V4 restart. The signing journal remained present. No chain reset, image replacement or package submission occurred.
- Captured the complete fleet again at 08:14:10 UTC. All four validators now have current sync cursors. Every validator still has only one configured RPC provider. The observer readiness checks pass; fleet image IDs remain different.
- Prepared genuine `npm:has-flag@4.0.0`: verified registry SHA512/SHA1, reviewed five regular archive files, no install lifecycle hooks or runtime dependencies, and captured a real OSV response with zero advisories. All eight nodes returned package absence at the shared h20. The current scanner profile does not yet include this candidate.
- Repaired direct sponsored-request policy enforcement: disabled policy rejects before RPC work/state reservation, actual transaction gas is estimated before reservation, and submission uses an explicit policy ceiling. Scoped relayer tests passed 4/4; targeted formatting and diff checks passed. These changes are source repairs, not deployed economic acceptance.
- Prepared a build from an explicit committed source archive using Rust 1.93.0, locked Cargo dependencies, Phase C features and the accepted signing public key. Builder/runtime images are pinned by digest, source hashes are recorded, and the dirty working tree is excluded.
- Provisioned a dedicated `us-west1-b` Sepolia RPC VM in `fiffi-500411` with a 400 GiB persistent balanced disk. JSON-RPC ingress is limited to the active R7 GCP node addresses, the operator laptop's current IPv4 `/32`, and the private fleet range; execution and beacon P2P ports are open for Ethereum peers, with SSH available through IAP only.
- Started pinned Geth `1.17.6` and Lighthouse `8.3.0-rc.0` containers. Both clients are on real Sepolia; Lighthouse has peers and its checkpoint matches the same finalized epoch/root from ethPandaOps and ChainSafe. Geth has execution peers and is importing Sepolia headers, but its execution head is still at genesis while sync continues.
- Rechecked the GCP RPC at 11:32 UTC: Geth has 42 execution peers, has downloaded 11.72% of the execution chain and 2.56% of the state, and reports an execution head of 0 while syncing. PublicNode and Tenderly both report Sepolia head `0xb47de4`. The GCP endpoint remains out of the validator profile until it catches up and matches block hashes and staking logs.
- Added a shared Alloy HTTP failover transport for bounded, idempotent L1 reads. Replaced direct single-endpoint providers in the API, bridge, consensus admission, governance, package admission and validator-registration sync paths. Startup checks that a strict majority of configured RPCs match the signed L1 chain ID and rejects any responding endpoint on a different chain. Stateful filter calls and transaction submissions remain single-endpoint; transaction recovery needs the durable journal path. The node compiles with `cargo check --locked --offline --features phase-c-real-fleet` on Rust 1.98.1 after adding Alloy's `json-rpc` feature. The new transport's formatter check and `git diff --check` pass. The source is not built with the deployment's pinned Rust 1.93.0 toolchain and has not been rolled out. The separate relayer, faucet and IPFS pinner crates still have direct L1 providers and are outside this node-only repair.
- Rechecked the GCP RPC at 12:11 UTC: Geth has 50 execution peers, reports `eth_syncing`, and still returns execution head `0x0`. PublicNode and Tenderly both report head `0xb47eb0`. The Geth node is not yet accepted or configured on any CREG node.
- Rechecked Geth at 13:28 UTC over its local JSON-RPC: chain ID `0xaa36a7`, 50 peers, execution head `0x0`, and sync progress marker `0x4c2626` toward `0xb47f7e`. PublicNode reports Sepolia head `0xb48022` and the same chain ID. Geth is still syncing and has not passed the common-block hash or staking-log comparison.
- Reviewed Cloud Build `b0fa8435-c9fd-455d-b1ee-c02c16e5eaca`: the pinned node compile completed after about 89 minutes, then the combined image build timed out while compiling the unchanged CLI at two hours. Added a committed-source `node-runtime` image target and node-only Cloud Build recipe to avoid repeating the CLI compile. The retry remains capped at two hours/$0.72 before any free allowance; it has not been submitted yet.
- At 14:07 UTC, Geth remains on Sepolia (`0xaa36a7`) with 50 peers and execution head `0x0`; state sync advanced to `0x53cc65` toward `0xb4807e`. PublicNode and Tenderly report heads 11,829,478 and 11,829,477 and the same hash at common height 11,829,413. Geth is not yet caught up for a three-provider comparison.
- Committed and pushed the read-failover repair as `f19b2e621a92339540ba87a9b60962afcc0aa40c` on `main`. A node-only Cloud Build using pinned Rust 1.93.0 is running as `a77052cf-72a8-4233-bad9-74722feb736d`; the image build step is working and version/hash checks are queued. No validator or observer configuration changed.
- Created Secret Manager profile `creg-sepolia-rpc-profile`, version 1, in `fiffi-500411` with PublicNode, Tenderly, and the static Geth endpoint. Confirmed three distinct provider hosts and no public IAM principal. This profile is staged only; no node environment or container uses it yet.
- GitHub Rust CI passed formatting, Clippy, and tests. The Phase C production recovery workflow passed the real-fleet node compile, durable PBFT signing-recovery verification, and relayer validation. The repository capability matrix workflow failed its freshness check because `config/capability-matrix.json` is stale; that workflow labels itself as not a release gate.

## Remaining gates

| Gate | Current state | Next action |
| --- | --- | --- |
| RPC resilience | Read-failover source passes local check and pinned-toolchain CI. Node-only Cloud Build `a77052cf-72a8-4233-bad9-74722feb736d` is running, not yet built or deployed. Secret Manager profile version 1 now contains three distinct endpoints but is not applied. Geth remains at execution head 0, with 50 peers and sync marker `0x53cc65` toward `0xb4807e`. PublicNode and Tenderly agree on chain ID and a common block hash. | Complete the pinned candidate build; let Geth finish execution and state sync; verify common block hashes and contract-filtered staking logs against all three providers; then configure one validator at a time and verify read failover on a real candidate. Complete durable transaction recovery before claiming write failover. Review standalone relayer, faucet and IPFS pinner RPC paths separately. |
| V4 freshness | Recovered on current older image | Deploy the tested sync supervision repair with the candidate image; verify continued fresh polling and restart recovery. |
| Deployment consistency | Five differing image IDs across eight nodes | Build and verify the committed candidate, stage on an observer, then use guarded canary/validator rollouts with preserved data/signing state. |
| Fresh acceptance | Real artifact and eight-node absence captured; not published | Extend the immutable scanner snapshot, capture readiness and absence again, attach clients, sign/publish once, verify three distinct validator approvals and certified matching finality across all eight. |
| Economic acceptance | Policy guards repaired; execution/recovery remains incomplete | Finish relayer transaction journaling and finalized receipt/accounting verification; implement reviewed reward contract and canonical-finality outbox/authorized dispatcher before real transactions. |

The earlier laptop build attempt ran out of disk and WSL failed. Disk space later recovered; a local incremental check now passes after loading the verified LLVM 23.1.2 runtime DLLs into a user-local folder. Cloud Build is enabled in the dedicated build project and its node-only Rust 1.93.0 candidate is running. This is not yet a verified image or rollout, and no validator or observer RPC profile has changed.

## Economic review

The faucet already has real funding, claim, reconciliation and restart evidence. That evidence does not establish rewards or sponsored staking.

Sponsored staking still needs a journal of the exact signed transaction and deterministic hash before broadcast, reserved signer nonce/intent binding, recurring recovery of ambiguous submissions, canonical finalized receipt checks, the exact staking event, and token/stake balance deltas. Duplicate/restart acceptance must prove one durable stake change.

Reward dispatch is intentionally skipped before canonical finality. There is no reward outbox or dispatcher yet. The current contract permits Governance/Staking only, has no finalization replay key, and needs participant uniqueness/bounds, vote-correctness points, bounded reputation and fully funded accounting repairs. Review the contract/dispatch change and exact authorized amounts before executing rewards. A node key alone is not sufficient authorization.

## Evidence

- `fleet-readiness-before.json` — live eight-node baseline; V1 state inferred from live health in this capture.
- `v4-sync-recovery.json` — guarded restart and exact preserved-state verification.
- `fleet-readiness-after-v4.json` — fresh complete capture including actual runtime image IDs/provider hostnames.
- `fresh-package/PREPARATION-REVIEW.md` — genuine artifact review and eight-node absence evidence.
- Build records/manifests are recorded separately; check their status before treating an image as built.
- `gcp-sepolia-rpc-startup-20261002.json` — VM, allowlisted network, immutable client image digests, independently verified beacon checkpoint and initial sync state.
- `gcp-sepolia-rpc-sync-progress-20261002.json` — latest RPC comparison and execution/state sync progress. No existing CREG node configuration was changed.
- `gcp-sepolia-rpc-sync-progress-20261002T1211.json` — latest live head and peer comparison after the read-failover source change. No existing CREG node configuration was changed.
- `rpc-failover-compile-and-sync-20261002T1328Z.json` — successful local node compile and latest live Geth/PublicNode sync comparison. No node configuration or containers were changed.
- `rpc-resilience-progress-20261002T140747Z.json` — private three-endpoint profile metadata, matching PublicNode/Tenderly common block hash, Geth sync progress, pinned image build state, and passing code CI. No fleet configuration or containers were changed.

No reward or sponsored-stake transaction was sent. RPC URLs and signing secrets are excluded from this evidence.

## RPC provider decision

Google Blockchain RPC is no longer a suitable new fleet dependency: Google disabled provisioning new endpoints on 2026-06-15 and scheduled service shutdown for 2026-12-15. The project API may remain enabled, but no endpoint was available to configure.

Use the existing no-subscription PublicNode and Tenderly Sepolia endpoints plus the self-hosted Geth execution client on GCP. The VM runs in `fiffi-500411` on `creg-fleet-vpc`; its JSON-RPC port is limited to the active R7 nodes' static egress IPs, the operator laptop's current public IPv4, and the private fleet range. Execution P2P and Lighthouse beacon P2P are reachable from public peers. Geth data lives on a separate persistent balanced disk. List-price cost is estimated at about $150/month before egress and taxes; applied or remaining GCP credits were not verified. PublicNode and Tenderly remain best-effort shared services.

This is not yet a fleet configuration change. Do not treat RPC resilience as accepted until the Geth node reaches Sepolia head, all three sources agree on chain ID, a common block hash and contract-filtered staking logs, and every validator's quorum path succeeds. The laptop firewall entry may need updating if its public IPv4 changes. Historical beacon backfill and Geth execution sync are still running.
