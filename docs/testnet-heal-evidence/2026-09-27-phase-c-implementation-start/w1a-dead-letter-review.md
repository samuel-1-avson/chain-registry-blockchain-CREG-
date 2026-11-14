# W1a Validator Retained Work and Dead Letter Review

**Captured:** 2026-09-27 02:35 UTC for canonical correlation; validator snapshots completed earlier the same day.
**Disposition:** Pass. All 25 records are confirmed terminal duplicates of canonical transactions. No item was requeued or cleared.
**Chain:** `creg-phase-c-testnet-2026-09-26-r7`, finalized at h12, `a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d`.

## Snapshot method and evidence

Each validator's `chain.rocksdb` was copied from a stopped container and inspected with the pinned read-only `creg_recovery_inspect` binary. The signer journal and signing key paths were left in their original node data locations. Three validators remained online during every capture; no VM was stopped. The local V1 container was stopped for less than one second during its successful copy, then returned on the same image and h12 tip.

| Validator | Dead letters | Active work | Snapshot evidence | Copy files |
| --- | ---: | ---: | --- | ---: |
| V1 | 7 | 0 | [V1 export](w1a-v1/w1a-v1-export-20260927T022943Z/) | 60 |
| V2 | 6 | 0 | [V2 export](w1a-v2/w1a-v2-export-20260927T014200Z/) | 46 |
| V3 | 6 | 0 | [V3 export](w1a-v3/w1a-v3-export-20260927T012700Z/) | 54 |
| V4 | 6 | 0 | [V4 export](w1a-v4/w1a-v4-export-20260927T014900Z/) | 56 |

V1's first copy attempt is retained separately and marked incomplete because the inspector inherited the node entrypoint. The successful V1 capture explicitly ran the recovery inspector as the entrypoint. V3's export contains the database, diagnostics and per-file hash manifest; its original capture did not save a `capture-metadata.json` file. Its contemporaneous live post-copy check and the all-node post-capture readiness report remain available. This metadata gap does not change the read-only classification result, but it is recorded for audit completeness.

V1, V2 and V4 captures include metadata, and their copied RocksDB file hashes were checked. V2 and V4 source/copy hash comparisons matched all 46 and 56 database files respectively. V3's hash manifest contains 54 database files. V1's successful export contains 60 files in its copied database and a per-file SHA-256 manifest.

## Canonical transaction reconciliation

The offline reports contain 25 validator records and 18 distinct work IDs. They reduce to six semantic operations: four authorized package publications and two revocations. Shared work IDs for each revocation are present on all four validators. V1 contains two separate `is-number` publication work IDs with the same already-committed publisher nonce; V4 shares one of those work IDs.

The reconciliation script fetched finalized blocks 0–12 and package states from V1's real public API. It confirmed signed-spec verification, chain identity, parent continuity, all 25 record matches and no unresolved entries. The detailed record mapping and captured canonical responses are in [dead-letter-review.json](w1a-v1/canonical-ledger-review-20260927/dead-letter-review.json) and [canonical-chain-capture.json](w1a-v1/canonical-ledger-review-20260927/canonical-chain-capture.json).

| Operation | Validator records | Canonical block | Resulting package state |
| --- | ---: | --- | --- |
| Publish `npm:is-number@7.0.0` | 5 | h1, `0121be9bb637aec634cd5f081fb903ee4749875d8225683f057af005ae16f436` | revoked |
| Publish `npm:escape-html@1.0.3` | 4 | h5, `daab438828a678aee3aa2eebf041f5147bc6f76fed837a207e6a1f4747d427cf` | revoked |
| Publish `npm:toidentifier@1.0.1` | 4 | h6, `d8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6` | revoked |
| Publish `npm:ms@2.1.3` | 4 | h11, `a87ff93fbdb2ea01549125a9f0a20c717049f0e8f8e28b952078cbf66b2117f3` | revoked |
| Revoke `npm:escape-html@1.0.3` | 4 | h10, `990091981261dce3160614e37e7b2faab4934d1d1f99101adcc9d37727b6e9d1` | revoked |
| Revoke `npm:ms@2.1.3` | 4 | h12, `a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d` | revoked |

Each publication record's canonical ID, publisher address, authorization nonce and chain ID match its finalized transaction. Each revoke record matches the finalized revoke for the same canonical ID. Every terminal reason agrees with the canonical authorization nonce or already-revoked package state. The R7 h12 chain reports zero active durable work; the dead-letter records remain preserved as evidence.

## Post-capture fleet status

The fresh [eight-node readiness capture](w1a-after-v1-snapshot.json) completed at 2026-09-27 02:32 UTC. All four validators and all four observers passed readiness, recovery-clear and PBFT-clear checks. All reported R7 h12 and the same accepted tip hash. No R22 image was deployed. No chain reset, public cutover, requeue, dead-letter deletion, GCP VM stop or VM deletion occurred.

## W1a gate

**Pass:** all 25 records are individually accounted for; all are terminal duplicates; active work is zero; the original diagnostics and database copies remain preserved; the fleet returned to a healthy all-eight state. Proceed to the R22 image rollout review and normal commit-recovery evidence. Keep public cutover gated on the remaining W2–W8 work.
