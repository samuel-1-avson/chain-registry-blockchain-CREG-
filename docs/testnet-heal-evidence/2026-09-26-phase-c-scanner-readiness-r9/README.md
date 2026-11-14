# r9 real scanner and r7 fleet evidence — 2026-09-26

## Established on real systems

- The operator's `abusech-malwarebazaar-auth-key` secret in `fiffi-500411`
  authenticated a real MalwareBazaar query. The snapshot contains 100 distinct
  SHA-256 records. Its credential was held in memory and is absent from the
  scanner bundle and evidence.
- Four independent validators and four independent observers run the pinned
  r9 image on the laptop and seven existing GCP VMs. All eight passed the
  scanner, signed-spec, genesis, validator roster and peer readiness checks.
  All four validators report isolated `nsjail` with no development bypass.
- The upstream `npm:is-number@7.0.0` tarball passed npm SHA-512 integrity
  verification, was pinned to IPFS and received fresh publisher authorization
  from the reviewed CLI. Three independent package approvals use the real
  pinned scanner profile.
- Publication finalized at h1 and revocation at h4 on all eight nodes. H2 and
  h3 were empty blocks. Independent checks reproduced every header hash and
  transaction Merkle root and verified at least three distinct Ed25519 COMMIT
  signatures per block. These checks are over captured real node responses.
- Real committed-state restarts passed on O1–O4, V1 and V2. They retained the
  same tip, revoked package state, scanner profile, peer connections and empty
  retired-work queues.

| Height | Contents | Shared hash |
| --- | --- | --- |
| 0 | Signed r7 actual genesis | `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4` |
| 1 | Authorized real npm publication | `0121be9bb637aec634cd5f081fb903ee4749875d8225683f057af005ae16f436` |
| 2 | Certified empty block | `6d1d4da2fb62b88af608b070d2a28fe1a2b41d29475d88061dabb87df6b3681e` |
| 3 | Certified empty block | `1fcd541e428fec047d1b518ed314fcc1370cd56aef505157b71821e2561362c3` |
| 4 | Publisher-signed revocation admitted and committed | `0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0` |

## Failed or outstanding acceptance

V3's real restart restored the correct chain and revoked package, but reported
`creg_pending_block_txs 1` and `creg_forced_inclusion_pending 1` after the restart
window and again during direct inspection. Its PBFT round count was zero.
The drill stopped at that failed queue-retirement check; V4 has not been
restarted in this drill. Do not describe eight-node recovery as passed.
All eight candidate containers remain running; no data or signing journal was
reset. The old public chain and default spec are unchanged.

The producer source retires applied/expired authorization variants inside
`prepare_unprepared_batch`, after selecting the local proposer. Hydration and
startup forced-work restoration load ready durable work independently of that
selection. Review this path and the enqueue-after-commit race against the real
V3 record before attributing or repairing the retained item. Preserve the
payload/reason atomically, retire its forced-inclusion entry, and retain all
prepared/COMMIT locks. A backup must not need to become leader to stop counting
permanently ineligible unprepared work.

H1 required certified view changes before finality. The logs also recorded
gossip duplicate suppression of signed PBFT retries and a local view lock
advancing immediately after a replacement view became certified. Review
transport retry identity and certified-view timeout tracking. Observer round
status can lag blocks imported by sync and also needs reconciliation review.
No in-flight prepared-round restart or quorum-loss drill has been completed.

Scanner readiness here covers the pinned YARA rules, real 100-record hash
snapshot and explicit OSV lookup for one real package version. It does not
establish general OSV package coverage. LLM advisory review was explicitly
skipped. L1 reward settlement and public reset/cutover remain pending.

## Evidence files

- `scanner-bundle-stage.json`: source/profile hashes, all eight placements,
  protected staging and image distribution.
- `fleet-runtime-readiness.json`: live scanner, chain, role and sandbox checks.
  The initial capture retains the failed SSH-client capture separately.
- `acceptance-package-provenance.json`: real npm origin and integrity result.
- `publish-admission.json`, `signed-publish-request.json`,
  `revoke-admission.json`: real publisher actions and their results; no keys.
- `fleet-publication-observation.json`: all eight committed h1 observations.
- `blocks-before-revoke/` and `blocks-after-revoke/`: full real public block
  responses and independent hash/Merkle/COMMIT verification results.
- `fleet-committed-restart.json`: changed process start times and checks,
  including V3's failed queue check. Only six nodes passed.

The four new GCP VMs are running CREG candidates and remain billable. No
additional VMs were provisioned in this continuation.
