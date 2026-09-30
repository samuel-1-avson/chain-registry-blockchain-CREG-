# CREG Testnet Heal — Status, Issues & Road to Public Testnet

- **Updated:** 2026-09-30 (earlier run evidence is retained below)
- **Purpose:** Single resume point. Read this file to know exactly where the
  heal stands, what is proven, what is blocking, and what to do next — in order.
- **Rule of this doc:** claims in the latest state have evidence observed in
  this session. Historical entries retain their earlier dated evidence; open
  work is listed separately.

## Public service and source release state (2026-09-30)

Public read-only checks at 04:37 UTC confirmed that the explorer serves R7
(`creg-phase-c-testnet-2026-09-26-r7`), with finalized height 20, 21 stored
blocks including genesis, four active validators, and zero pending transactions.
The signed chain specification is verified by the serving node. Earlier
statements below about public routing still selecting the legacy h0 chain
describe the historical captures at their stated dates.

The public R29 faucet is healthy and its ledger is reconciled. It reports
99,900 tCREG in its wallet, 2,100 tCREG distributed across three confirmed
claims, and zero unresolved claims. The approved 92,000 tCREG treasury transfer
and the real 100 tCREG acceptance claim are recorded in the tokenomics plan.
Total outstanding supply remains 20,000,000 tCREG; 42,000,000 is the maximum
outstanding supply and the agreed target, with issuance work still pending.

The corrected `v0.4.9-testnet` CLI release is published for Windows, Linux, and
macOS with checksums. The source publication also includes the remaining
durable PBFT signing/reproposal, transaction retirement, event, metrics,
explorer, and relayer repairs. Production-feature compilation and focused
recovery tests are explicit CI gates. These checks do not replace remaining
real fleet acceptance or authorize reward payouts, minting, or another reset.

See [the tokenomics implementation plan](CREG-TOKENOMICS-IMPLEMENTATION-PLAN.md)
for the remaining economic work. Historical build and acceptance captures
are retained below.

## Historical build, scanner rollout and acceptance gates (2026-09-28 00:07 UTC)

- The optimized R25 follower-capture node and corrected portable Linux CLI
  builds both completed successfully. Node binary SHA-256:
  `24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07`;
  pinned image manifest digest:
  `sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70`.
  The corrected CLI binary SHA-256 is
  `95569797c4c0c3d767641cc7df07307874da4673fc32daa7f25ceb63776f2c69`
  (67,270,408 bytes), with maximum required glibc 2.34.
- The corrected CLI ran on the real Ubuntu 22.04 O1 VM. `creg blocks` read
  the actual h14 revoke and h13 publication; `creg verify npm:kleur@4.1.5`
  returned the exact canonical ID and the real `Revoked` chain status. This
  revoked package does not demonstrate a positive Merkle proof. Fresh
  publication and named-event/proof acceptance remain open. The final Windows
  rebuild and release packaging also remain open.
- R25 image canaries on O2 and V4 passed. The first O2 startup capture failed
  before peer reconnection and remains preserved; the later O2 capture passed.
  V4 preserved its signing journal, identity, environment, six terminal
  records and signed h14. O1 remains R23; O2 and V4 are R25; the other five
  nodes remain R22. Image alignment remains open.
- The real R12 scanner bundle is now deployed to all eight real nodes. The
  fresh Abuse.ch feed has 100 records and the genuine OSV query for
  `npm:escape-string-regexp@5.0.0` returned no advisories. All eight report
  scanner profile `e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2`;
  the final readiness capture passed all eight, with shared signed/finalized
  h14 `2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa`,
  queues clear and no unresolved PBFT round. Validator dead-letter depths are
  V1=9, V2=7, V3=7, V4=6 and were preserved by scanner restarts.
- V1's first postflight sampled Docker before its health state reached
  `healthy`; every other check passed. That failed capture is preserved, and
  the follow-up after health convergence passed. No chain state, node identity,
  signing journal, or dead-letter record changed. Evidence is under
  [the R12 rollout directory](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r12-live-rollout/),
  including `readiness-all-eight-r12-final.json` and both V1 postflight reports.
- Next, perform the real CLI publication for the reviewed fresh canonical
  package, keep CLI/browser event watchers attached, and verify shared finality
  plus a positive package proof. Then use that exact new transaction for V4's
  controlled local durable-COMMIT crash/replay capture and confirm all eight
  nodes converge. Remaining service paths, R25 image alignment, final CLI
  distribution, and public R7 routing remain open.
- The public explorer/API still selects the legacy chain. No chain reset or
  public R7 cutover occurred. Four additional GCP VMs remain running and
  billable. See [the current pipeline audit](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/PIPELINE-AUDIT.md).

## Earlier production pipeline work (2026-09-27 21:32 UTC)

- The public faucet is restored on the fresh Sepolia token contract. Root,
  health, network, challenge and wallet RPC return successful real responses.
  User-approved 10,000-test-CREG funding confirmed; an actual 1,000-token claim
  then confirmed. PostgreSQL and faucet restarts preserved the cooldown, and a
  repeat claim returned HTTP 429. The current reserve is 9,000 test CREG;
  native ETH drips are configured at zero. The legacy broken container is
  stopped and retained. The protected database credential is in Secret Manager.
- Final R25 explorer proxy repairs are live in the dedicated pinned Compose
  project and staged privately against R7 h14. Public/operator paths retain
  their access boundaries, the node upstream
  is configurable, standalone wallet RPC uses real Sepolia, and deployment
  entry points avoid stale immutable caching. Private R7, private legacy and
  public checks each passed fourteen real pages plus actual Sepolia RPC, with
  no JavaScript errors or HTTP 5xx responses. Earlier RPC 404/403 failures are
  preserved. Public R25/API/root spec still select legacy h0; R24 is stopped
  and retained. Final explorer digest is `ae0d80a7bd6f5534d4814dffdf613b66c4fee31843bfec69f713ce4319fbe122`.
- Real Prometheus and O1 restarts preserved identical anchored historical
  samples. The history API returned 121 samples across seven real series after
  both restarts. No validator was interrupted.
- The follower COMMIT capture boundary missing from the earlier V4 drill is
  now implemented in source; compilation is in progress. No validator has
  received that new build and the V4 recovery gate is not claimed passed.
- A release-scope question is pending: publish a limited read-only R7
  explorer/API and repaired faucet with public writes gated, or finish all
  mandatory gates before switching the public chain. Full cutover still requires
  V4 recovery evidence, fleet artifact alignment, remaining relayer/reward/service
  checks and CLI distribution. No new reset or public R7 chain switch occurred.
- Subsequent storage routing checks found and repaired the public IPFS content
  gateway and the old CLI's spec-host IPFS API path. A real published tarball
  downloads with its recorded SHA-256. `/metrics` now serves real Prometheus
  text, still from the legacy public node until routing changes. The repaired
  faucet is now managed by its own pinned Compose declaration and retained data.
- CLI release-candidate archives/checksums are staged under the versioned R7
  path; root defaults remain unchanged. Actual cold installation exposed two
  release blockers: missing canonical encoding in the first Windows build and
  Linux glibc 2.39 requirements incompatible with the Ubuntu 22.04 host. The
  Windows rebuild now passes cold installation, signed R7 validation and actual
  h13/h14 reads. Staged installation rejects the retained incompatible artifact
  without changing the accepted binary/spec/config. Linux is compiling in a
  pinned Ubuntu 22.04 builder using a separate target cache. Distribution remains
  open: canonical-ID parsing in `verify`, fresh installed-binary proof/events,
  version/self-update/platform metadata and final defaults need resolution.
- Read-only Sepolia inventory confirms all four validators are active. Rewards
  have no emitted packages or pending balances. The existing treasury allowance
  already covers reward `transferFrom`; an empty reward contract balance alone
  is not a funding prerequisite. Current node reward emission occurs before
  finality, has no durable receipt/outbox and counts skipped calls as success.
  The sponsored relayer has non-fatal nonce persistence and memory-only status.
  Activation requires repairs and real receipt/restart checks, not just funding.
- A genuine `npm:escape-string-regexp@5.0.0` tarball is preserved with verified
  registry integrity and its actual OSV response for the next recovery probe.
  No package code was executed, no publication occurred and the fleet scanner
  profile is unchanged. New pinned OSV coverage is needed before publication.

See the current [pipeline audit and evidence](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/PIPELINE-AUDIT.md).

## Earlier public explorer and faucet update (2026-09-27 19:44 UTC)

- The public explorer now runs R24 at image digest
  `sha256:090ba37ef9f0ee228d40d9f296d7fdafde1ef224f66f6015500c13d7f9dae6c7`.
  The live dashboard displays the API's aggregate stake as `200 CREG` and shows
  finalized h0 because the real genesis block response is finalized. Browser
  verification returned HTTP 200 with no page errors or HTTP 5xx responses.
  R23 is retained stopped for rollback. The explorer remains connected to the
  legacy public chain at h0; its API reports 2 validators and 4 pending
  transactions. Public cutover was not performed. See
  [`explorer-faucet evidence`](testnet-heal-evidence/2026-09-27-explorer-faucet/)
  and [the explorer repair record](EXPLORER-PIPELINE-REPAIR.md).
- The public faucet remains unavailable and returns 502. Its container is
  restarting because `FAUCET_PRIVATE_KEY` is empty. The available faucet
  secret does not match the configured funded wallet, and the wallet funded
  from that secret has no tokens for the public legacy contract. No on-chain
  transfer was made.
- A faucet repair is staged in source: startup verifies the key/address pair;
  testnet startup requires PostgreSQL; cooldowns for address and IP are claimed
  atomically and persist across restarts. Both deployment Compose profiles now
  include a persistent database service. The Linux/amd64 faucet image was
  built, pushed and pulled to the public-services VM at digest
  `sha256:360c124090e5c3a5c448a69e9838bb2c1755ed18f6d32732bff6498d4cdd5ed5`;
  it has not been started. `cargo check -p faucet` passed and both Compose
  configurations validated using non-secret placeholder values. The live
  faucet remains on the old restarting container. Deployment still needs a
  matching funded faucet signer (or a reviewed funding action) and a protected
  database credential. No chain/API/spec reset or cutover was done.

## Historical accepted state (2026-09-26, R21 / h12)

This snapshot is superseded for candidate-chain progress by the
[2026-09-27 Phase C implementation plan](PHASE-C-PUBLIC-CUTOVER-IMPLEMENTATION-PLAN.md),
which records R22 and the shared h14 candidate tip. The public explorer and
faucet status above was refreshed independently on 2026-09-27.

- Candidate chain `creg-phase-c-testnet-2026-09-26-r7` remains at finalized h12,
  hash `a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d`,
  with genesis `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
  No chain reset or public cutover occurred.
- All four validators and four observers are on the reviewed R21 artifact and
  passed a sequential container restart followed by a fresh readiness capture.
  Each retained h12, the signed chain identity, its node role and P2P identity;
  pending/durable queues and active PBFT rounds are zero. Validator restarts
  were one at a time with the other three validators online. Runtime image IDs
  `sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423`
  and `sha256:d2b83a6525314b17434bc98c5bf05181db4fd6d73c962eabd2b5407968dfa95c`
  are accepted Docker representations of the pinned R21 artifact.
  These were container restarts; no GCP VM or the laptop host was rebooted.
- V3 work `089d7fdd8308d6588563b5a30b41c900b79d8f0c16c908a5cd849b1e195e5b1`
  for `npm:is-number@7.0.0` remains preserved as a terminal dead letter.
  V3 has zero active work; no replay or deletion was attempted. The h12 review
  accounts for 25 terminal validator dead letters (V1=7, V2–V4=6 each); the
  12 additions since h9 are explained by already-committed duplicate publish
  and revoke actions. The original 13 h9 records have item-level review; the
  additions are reconciled from validator logs and live metrics.
- V4 recovered its prepared h11 value after a container crash and reconciled
  the certified canonical block. A local durable COMMIT recovery was not
  observed. V4 later finalized h12 with the four-node fleet and passed both
  committed-state and fleet restart checks.
- The real candidate web explorer captured h11 `npm:ms@2.1.3` publication and
  h12 revocation events. The local CLI build succeeds; `creg blocks` reads h10
  through h12 from V1, and `creg watch` connects to `/v1/public/events` without
  retries. The node also supports the `/v1/events` SSE compatibility route;
  both returned HTTP 200 and real heartbeat/ping frames in live checks. The
  local watcher now uses the explicit public route. A named live event has not
  yet been observed in CLI output; SSE does not replay earlier events.
- The built local CLI also read the public API successfully: it reports
  `creg-testnet-1` at h0 with genesis
  `068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53`,
  matching the live public API response and the public explorer. Its `watch`
  command also held an open connection to the public SSE endpoint without
  retries for the observation window; no named event occurred during that run.
- The public explorer/API still serve the separate legacy chain
  `creg-testnet-1` at h0. Public cutover remains false. The four additional
  billable GCP VMs remain `RUNNING`: `creg-validator-3`, `creg-observer-3`,
  `creg-validator-4`, and `creg-observer-4`.

Evidence for this update is in
`testnet-heal-evidence/2026-09-26-phase-c-events-r16/r21-rollout/`, especially
`fleet-r21-h12-container-restart-recovery.json`,
`fleet-r21-h12-after-restart.json`, `dead-letter-review-r21-h12.json`,
`v4-prepared-recovery-r21.json`, `sse-r21-cli-route-check.json`, and the browser
h11/h12 captures. The GCP inventory was checked with
`gcloud compute instances list --project=fiffi-500411`.

### Remaining work before public cutover

1. Decide whether the V4 recovery gate requires local durable COMMIT replay
   before peer sync. Current evidence proves prepared-value recovery and
   certified canonical reconciliation, but did not observe local COMMIT replay.
   If required, capture that phase during a controlled future prepared round.
2. Capture a real named SSE event in CLI output. Browser publish/revoke event
   rendering is proven; current CLI evidence proves build, candidate/public
   block reads and live streams with no reconnect errors.
3. Verify the CLI publish path with a fresh operator-selected package during
   that live event capture, then package and release the corrected CLI.
4. Verify still-unproven faucet, wallet/staking/reward and relayer writes, and
   complete any required persistent metrics/history path. Keep scanner risk
   coverage scoped to the pinned profile and its actual queried packages.
5. After acceptance evidence is complete, plan the coordinated public API,
   explorer, CLI, signed-spec and contract cutover. Preserve the current public
   chain evidence and do not reset the candidate as part of this validation.

## Historical accepted baseline (h6; superseded by R21 / h12 above)

- All four real validators and four real observers run R18
  `creg-node:phase-c-secure-r18-canonical-round-recovery`. Runtime config ID:
  `sha256:dd07eef4a657707fdad696b1b4cc7397ee2589b82e3511c892ff3a806f3dcb03`;
  the equivalent containerd manifest ID is recorded in the rollout evidence.
- V3's original retained work is terminal: its publisher authorization nonce
  was already committed. Its payload remains in durable dead letter. V4 and
  all eight committed-state restart checks passed on the earlier accepted
  baseline. R18 guarded replacement/startup checks also passed on all eight.
  No signing journal was copied, edited or deleted.
- All eight nodes share h1–h6. Real `npm:toidentifier@1.0.1` finalized at h6,
  hash `d8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6`.
  Independent checks verified hashes, Merkle roots and at least three distinct
  Ed25519 COMMIT signatures on every block and every host. The real candidate
  CLI also verified the publication against the authenticated R7 spec.
- The actual candidate web explorer and CLI received submission, validator
  votes, committed block and package-verification events for h6. The browser
  rendered the events without page errors. Public R14/global CLI R15 still
  serve the legacy context; their packaged update/cutover remains pending.
- All eight nodes use scanner profile
  `d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4`, covering
  three real package versions with actual OSV responses, 100 real MalwareBazaar
  hashes and pinned YARA rules. Validators run isolated nsjail without bypass.
- Stale same-canonical COMMIT rounds cleared after independently certified
  canonical reconciliation. The real h6 block survived a 3-vote certified
  view change with its original prepared hash. This does not establish an
  in-flight crash/restart gate.
- Stopped-copy review accounted for all nine dead letters through h5: every
  authorization nonce was already applied. Preserve these terminal records;
  do not replay them. New h6 records await the next stopped-copy inspection.

### Issues recorded before R21 rollout (superseded where resolved above)

1. A real owner revocation was accepted on V1 but remains queued at h7. The API
   persisted work only locally and did not propagate the signed request. No
   prepared round formed; the first recovery probe timed out without stopping
   any validator. This is preserved failure evidence, not acceptance.
2. Observers retained pending-package admissions despite certified publication.
   R19 compiled a certificate-checked durable retirement repair but is not
   deployed. R20 combines it with durable signed revocation propagation and is
   compiling. Deploy one reviewed artifact with stopped copies and unchanged
   signer journals, then repeat the same authorized revocation through one API.
3. Complete real prepared/COMMIT crash recovery and prove that two online
   validators cannot finalize, followed by recovery with a real quorum.
4. Restore candidate faucet/relayer services and real wallet, stake and reward
   writes. Actual fresh faucet token balance is zero; the fresh relayer has
   zero gas. Public `/api/network` and `/v1/relayer/policy` returned 502. R17
   service binaries compiled, but services/funding/durable restart checks remain.
5. Complete durable historical metrics, packaged CLI delivery and public
   explorer/spec/API cutover after acceptance gates. Public routing still
   serves `creg-testnet-1` at h0; no public reset/cutover occurred.

The four additional GCP VMs run real candidate CREG nodes and continue to accrue
charges. Evidence: `testnet-heal-evidence/2026-09-26-phase-c-events-r16/`, including
`rollout-r18/fleet-rollout.json`, `post-h6/blocks/quorum-check.json`,
`browser-toidentifier-r18.json`, `dead-letter-review-r18.json` and the retained
h7 timeout reports. Older observations below are historical.

## Historical explorer and fleet snapshot (2026-09-26 13:32 UTC)

The public web explorer is connected to the live public REST API and real
Sepolia JSON-RPC. Its production build is R14; it still displays the separate
legacy public chain `creg-testnet-1` at h0. The signed root spec and versioned
r7 spec/signature paths now return real spec-service data. No public routing,
default spec or chain history changed. The Windows CLI is rebuilt as R15 and
both local install paths return live data from the public chain and the
separate candidate. Its packaged installer is pending. Route and image
evidence is in `docs/testnet-heal-evidence/2026-09-26-explorer-pipelines/`.

A fresh read-only snapshot of all eight real candidate hosts at
2026-09-26 13:32 UTC found the same r7 chain, genesis and h4 tip hash on every
node. All eight report process, network, signed-spec, four-validator and
scanner-profile readiness. Recovery counters are separate: V3 has
`pending_block_txs=1`, durable finalized work depth 1 and
`forced_inclusion_pending=1`; the other seven hosts have those three counters
at zero. V1, V2 and V4 retain dead-letter depths 2, 1 and 1. These records
need reason/payload review; they are not counted as active work and have not
been deleted. Thus **all eight are online and scanner-ready, but recovery
acceptance has not passed**.

The fleet-capture script now requires PowerShell 7 and records the queue
counters independently. An initial invocation under Windows PowerShell 5.1
hit an unsupported JSON option; it was a capture-tool compatibility error, not
node failure. That diagnostic is retained beside the successful snapshot.
Secret Manager contains `abusech-malwarebazaar-auth-key` with enabled version
1; the secret value was not read for this status check.

## Historical source repair and scanner state (2026-09-26; later R21 state above)

The transaction/queue and genesis/recovery/client repairs are included in the
reviewed r9 image. It compiled in release mode from a frozen source manifest;
image ID: `sha256:cf9fda37887e0d8c6125280c07bdb36737e495e75d070b80abcc722187180d33`.
The r7 signed chain identity remains `creg-phase-c-testnet-2026-09-26-r7`, with
actual block zero `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
Its signed spec is now served at the versioned `/phase-c/r7/` URL; the existing
default spec remains unchanged. Source, build and CLI snapshot evidence are in
`docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9/`.

The bundled placeholder hash database has been removed from active scanner
inputs and preserved at
`docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness/legacy-unverified-bootstrap-threats.json`.
The scanner now requires a provenance-bearing MalwareBazaar hash snapshot, a
pinned OSV snapshot with explicit queried package-version keys, the complete
YARA rule-bundle digest, and an operator-pinned combined profile digest. The
real OSV CLI snapshot for `npm:is-number@7.0.0` has a present key with zero
advisories (SHA-256
`39d74eb091cbe986073f3a711c2f25f3be6975dbb38e2083e5fe5e445e7d4ade`), saved
privately under `C:\Users\samue\CREG-backups\phase-c-scanner-r8\`.

The operator added `abusech-malwarebazaar-auth-key` to Secret Manager in
`fiffi-500411`. The snapshot builder retrieved it in memory and fetched 100
unique real SHA-256 records from MalwareBazaar. The credential was not printed
or written to disk. The source response digest is
`59217e0f3351528410efe04580879cdb44ff2322689fb770a34b641aa4733211`; the
threat database digest is
`5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef`.
The YARA bundle digest is
`2ec033d1446c6280f46d7d74467f78223c67c9abe45c282080db78cfe3177766`, and the
OSV snapshot digest is
`39d74eb091cbe986073f3a711c2f25f3be6975dbb38e2083e5fe5e445e7d4ade`. The
combined scanner profile digest is
`648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f`.
Identical local copies and a digest manifest are staged privately under
`C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9\`.

The same read-only bundle is mounted on the laptop and all seven GCP hosts.
V1 now runs the r13 node API build and the other seven candidates remain on
r9. All eight report the same actual genesis, a verified spec signature, four
active validators, live peers and the pinned scanner profile. All four
validators report `nsjail`, real isolation and no development bypass. P2P
identities were checked against the preserved r4 files and both signed bootnode
identities before first launch. The r7 spec and signature return HTTP 200 at
the versioned endpoint; the existing default spec is unchanged. See the
13:32 UTC fleet snapshot linked below for current host and recovery counters.

The real upstream `npm:is-number@7.0.0` tarball was downloaded, checked against
its npm SHA-512 integrity, pinned to IPFS, and signed with the reviewed r9 CLI
and fresh publisher keys. All eight nodes report its committed publication at
h1, hash `0121be9bb637aec634cd5f081fb903ee4749875d8225683f057af005ae16f436`.
The block carries three independent package approvals under the pinned scanner
profile and three PBFT COMMIT signatures. H1 first stalled, then recovered after
certified view changes. The run exposed duplicate suppression of PBFT retries
and a local view timer advancing ahead of the certified replacement view; these
remain items for liveness review. LLM review was explicitly skipped; this run
does not establish LLM readiness.

A certified empty block followed at h2 and another at h3. Revocation finalized
at h4, hash `0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0`,
on all eight nodes. Independent checks reproduced the header hashes and
transaction Merkle roots and verified at least three distinct COMMIT signatures
on every block. Real committed-state restarts previously passed on O1–O4, V1
and V2. V3 restored the right chain and revoked package but retains one pending
block transaction, one durable-work item and one forced-inclusion entry; the
drill stopped on that failed queue check before V4's restart. Eight-node
recovery has not passed. The prior r4 data and signing journals are preserved.
Separate legacy A/B services on V1, V2 and O1 remain running. No old-chain
reset or public cutover has occurred. The four new GCP VMs remain running and
billable and run real CREG candidate nodes.

See `docs/PHASE-C-GENESIS-RECOVERY-REVIEW.md` for signing-state/client trust
repairs and `docs/PHASE-C-IMPLEMENTATION-PLAN.md` for the remaining acceptance
sequence. Explorer findings are in `docs/EXPLORER-PIPELINE-REPAIR.md`; the
current host snapshot is `docs/testnet-heal-evidence/2026-09-26-explorer-pipelines/fleet-final-r15.json`.

## Previous Phase C deployment state (2026-09-25, 20:58 UTC)

Fresh Sepolia contracts and a real four-validator roster are live. The four
prepared GCP VMs have been started and accrue compute charges. A signed
genesis with four active, 100-CREG-staked validators was deployed as an
isolated candidate on eight real hosts: four voting validators and four
non-voting observers. All eight reported the same genesis block, four active
validators, signature-verified chain spec, and live P2P peers. The validators
reported a real `nsjail` sandbox and quorum 3-of-4.

A benign npm tarball `npm:creg-phase-c-heal-20260925@0.1.0` was pinned to
IPFS, signed with fresh publisher keys, admitted on V1, and independently
approved by three validators. H1 did **not** finalize. The proposed block
failed namespace transition validation because pre-activation v4 publisher
authorization incorrectly required an upstream authority certificate. A
source repair is building. All eight candidate containers were stopped with
their failed-run data and signing journals intact. No h2 acceptance, public
cutover, or coordinated reset of the old divergent chain has occurred. The
pre-reset archives and public block-hash evidence remain preserved.

The detailed A/B findings and earlier infrastructure snapshots below describe
the old chain and the state before this Phase C candidate run. Current host,
contract, and image details are in `docs/PHASE-C-FLEET.md`.

## 1. Baseline after Phases A/B, before the isolated Phase C runs

This section records the earlier divergent public-chain snapshot. The current
candidate/source state is recorded at the top of this document.

Phases A and B were run on 2026-09-25. The smoke publish finalized as h1 on both
validators, then the revoke finalized as h2 on validator-1 only. Current tips
are v1=h2, v2=h1, observer=h0. The observer rejects h1 because its header
commits a node-local validator-set hash instead of the authenticated bootstrap
authority. Phase C remains local and undeployed; see §13 for hashes, logs, and
the selected history-recovery approach. The next acceptance run requires four
real, independently hosted validators and four real, independently hosted
non-voting observers; the current two-validator fleet does not meet that gate.

## 2. Initial network state before Phases A/B (verified 2026-09-24)

| Node | Host | Tip | Genesis | Peers | Pending | Notes |
|------|------|-----|---------|-------|---------|-------|
| validator-1 | `creg-validator-1` (GCP) | 0 | `068364cf…96b53` | 2 | 0 | Fresh-wiped, idling, healthy |
| validator-2 | cloud VM (see I4) | 0 | same | 2 | unknown | Up, fallback live, keys intact |
| observer | `creg-testnet-vm` (GCP) | 0 | same | 2 | **1 (ghost, see §4)** | `CREG_PEERS=http://10.142.0.4:8080`, recreated, healthy |
| edge | `https://api.testnet.cregnet.dev` | 0 | same | 2 (via observer) | 1 | `active_validators: 2`, set `synced`, spec sig verified |

Chain facts: `chain_id: creg-testnet-1`, `block_count: 1` (genesis only),
`validator_set_source: chain-authoritative`, L1 (Sepolia) tracking live
(finalized source block advancing).

## 3. Proven working (with evidence)

- [x] **Full P2P mesh.** `peer_count: 2` on v1 and observer; fresh
      `Connection established` on both after the Round 3 recreate; v1 count
      stable across runs (holding, not flapping).
- [x] **Round 3 (observer `CREG_PEERS` + recreate, no volume wipe).**
      Observer rejoined the mesh; P2P identity `WSUsW…` preserved.
- [x] **Single chain, no fork.** Identical `tip_hash`/`genesis_hash` on v1,
      observer, and edge.
- [x] **Edge + explorer path healthy end-to-end.** Public `/v1/public/chain/stats`
      and `/v1/health` serve live observer data; validator set synced 2/2.
- [x] **VRF proposer fallback live on v2** (verified in an earlier round).
- [x] **Hardening batch code-complete + tested:** `cargo test -p consensus`
      53/53 pass, `rustfmt --check` clean. 10 files, +434/−12 (see §7).
- [x] **Runbook + n=3 decision written** (`chain-registry/testnet/VALIDATOR-RUNBOOK.md`
      §2: quorum math, why n=3 standard math hurts liveness, flag warning).
- [x] **Disk space recovered** (~5.4 GB: regenerable `target/`, `node_modules`,
      tarball caches). Codebase untouched.
- [x] **Ghost pending TX root-caused** (see §4). Benign, non-blocking.

## 4. The ghost pending TX (root-caused, NOT an issue)

- Observer shows `pending_tx_count: 1` with **zero** submission lines in 60 min
  of logs (no `Publish request`, no `Received … via gossip`, no `dropping`).
- Cause: `PendingPool::open()` restores `pending_pool.json` from disk on startup
  (`pending_pool.rs:49`). The entry is a **pre-wipe publish resurrected from the
  observer's preserved volume**; its only log line is
  `[PendingPool] Restored N pending package(s)`, which earlier greps missed.
- Why harmless: the validator pipeline explicitly no-ops on observers
  (`validator_pipeline.rs:60`), so the entry is never processed and cannot block
  anything. v1's pool is empty, so a fresh publish admits with a fresh committee
  (no 409 conflict possible).
- Residue: cosmetic `pending=1` on stats/explorer until optionally cleaned
  (delete `pending_pool.json` on the observer volume + recreate — optional, not
  required for h1/h2).

## 5. Operational rules learned (do not re-learn these)

1. **Submits must target a VALIDATOR (v1), never the edge.**
   `api.testnet.cregnet.dev` proxies to the observer, which files publishes and
   never processes them. Gossip delivery is best-effort (no retry on the
   submissions topic), so observer-side submits may never reach validators.
2. **Two pools, two meanings.** `pending_tx_count` = gossip package publishes
   awaiting pipeline. Revokes skip that pool and go straight to finalized work,
   picked up on the producer's next tick.
3. **`/v1/pending` needs private API credentials** (401 even on localhost).
   Use container logs for pool forensics instead.
4. **`Connection refused` dials toward the observer are noise.** Observer inbound
   TCP 9000 is filtered; the mesh runs over observer-initiated connections.
   `peer_count: 2` is the health signal, not the absence of dial errors.
5. **Keys stay user-side.** Signing happens on the PC; only signed (public)
   payloads travel (SSH tunnel recommended, no firewall changes).

## 6. Open issues

| ID | Issue | Severity | Owner | Status / Next action |
|----|-------|----------|-------|----------------------|
| I1 | Existing A/B history diverges: v1=h2, v2=h1, observer=h0 | **Blocking** | team | Preserve evidence, then coordinated reset to signed genesis or define a compatibility migration (§13) |
| I2 | Current two-validator fleet cannot demonstrate the intended independent four-validator PBFT topology | **Blocking** | user + team | Provision four real validators on separate hosts; require 3-of-4 package votes and block finality, without the small-cluster shortcut |
| I3 | v1 peer ID `EtMX…` chain-spec update still pending | Medium | user | Update spec + mirrors, re-verify `spec_signature_verified` |
| I4 | v2 host/access details need runbook update | Medium | team | Located as local Docker Desktop container `creg-validator-validator-2`; access repaired without wiping its chain volume |
| I5 | Hardening batch and repair edits are uncommitted | Medium | team | Finish protocol review and isolated validation before committing. Untracked `creg-test/` holds keys — never commit it |
| I6 | Fleet still runs `genesis2` images; Phase C is not deployed | Medium | team | Build/deploy only after certified view-change review and real eight-node acceptance pass |
| I7 | Observer inbound P2P 9000 filtered (works around via outbound mesh) | Low | infra | Open or document as intentional before public testnet |
| I8 | Ghost `pending=1` cosmetic residue on observer | Low | — | Optional cleanup (§4); ignore until h1/h2 verified |
| I9 | Certified view change cannot yet preserve PBFT safety and recovery | **Blocking** | team | A synced, identity-bound signing-decision journal now protects PREPARE, COMMIT, and view-change votes across restarts. Exact prepared-block persistence/recovery, immutable-value reproposal, and end-to-end restart recovery remain open; the compile-time release gate stays closed |
| I10 | Four additional GCP hosts are provisioned, but no new CREG nodes are running | **Blocking** | team | V3/V4/O3/O4 have separate VMs and mounted data disks in four regions. Signed genesis, keys, Phase C safety work, and real-node acceptance remain open |
| I11 | Existing external-validator launcher can select `genesis2` and lacks secure sandbox setup | **Blocking** | team | Its small-cluster default and key printing were removed locally; do not use it for Phase C until a four-validator, real-sandbox deployment profile is ready |

## 7. Uncommitted batch (I5 detail, `git status` 2026-09-24)

Modified (unstaged), 10 files, +434/−12:

- `chain-registry/crates/common/src/validator_snapshot.rs`
- `chain-registry/crates/consensus/src/{lib,pbft,validator_set}.rs`
- `chain-registry/crates/node/src/{block_producer,p2p,p2p_rate_limit,validator_set_history}.rs`
- `chain-registry/testnet/caddy/Caddyfile.unified`
- `chain-registry/testnet/docker-compose.unified.yml`

Untracked: `creg-test/` (test package + `publisher.key`, `evm.key` — user signer
material; keep out of git).

## 8. Phase A — First blocks (completed 2026-09-25; evidence in §13)

**A1. Open an SSH tunnel from the PC to validator-1** (keys stay on the PC):

```powershell
<your usual v1 SSH command> plus: -L 8080:localhost:8080
# e.g. gcloud compute ssh samue@creg-validator-1 --zone=<your-zone> -- -L 8080:localhost:8080
```

**A2. Run your signer + publish flow against `http://localhost:8080`**
(not the edge URL). Expect: `202 accepted` + canonical.

**A3. ~60 s after submit, watch v1** (on validator-1):

```bash
sudo docker logs creg-validator-validator-1 --since 3m 2>&1 | grep -iE 'publish request|pipeline processing|proposer created|sealed|committed' | tail -10
curl -s http://localhost:8080/v1/public/chain/stats | grep -oE '"tip_height":[0-9]+|"package_count":[0-9]+'
```

Expect: `Pipeline processing 1 package(s)`, then `tip_height: 1` (h1).

**A4. Submit the revoke** (same v1 target). Expect: `tip_height: 2` (h2).

**A5. If A3 shows nothing:** paste the exact submit response + target URL.
A `202` with no v1 log lines means the submit went to the wrong host.

## 9. Phase B — Historical two-validator verification (observer sync did not pass)

Run from anywhere with internet (edge checks) plus one v1/v2 stats pair:
These were the checks for the 2026-09-25 A/B run; the next acceptance gate is
the real eight-node topology in §14.

1. **Shared seals:** h1 and h2 each carry 2/2 validator certs; both validators
   report the same `tip_hash` at heights 1 and 2.
2. **Observer sync:** edge `tip_height` tracks validators with ~0 lag;
   `finalization_lag: 0`.
3. **Explorer:** blocks and transactions visible; package page for the published
   canonical leaves `pending`.
4. **No vote-loss repeat:** no stuck PBFT rounds in validator logs
   (`Rebroadcasting own vote` lines would indicate a repeat).

## 10. Phase C — Repair, validate on real nodes, then deploy

1. Preserve the §13 run evidence and choose the chain-history recovery path.
2. Complete the certified view-change protocol; a local stall timer must not
   authorize a backup proposer.
3. Prepare four validators on separate real hosts and four observers on separate
   real hosts (§14). Use a freshly signed common genesis and independent data
   volumes, keys, processes, and network paths.
4. Submit a real benign package through a validator with its real security
   sandbox enabled. Require 3-of-4 consensus-grade package approvals and PBFT
   finality. Verify identical h1/h2 hashes and valid certificates on all four
   validators, then confirm all four observers sync both blocks. Inspect every
   validator for stuck rounds and failed view changes.
5. Only then prepare a coordinated deployment. Do not reset or deploy the live
   testnet as part of this code review.

## 11. Phase D — Public testnet readiness checklist

- [ ] I2 resolved: four independent, active validators; 3-of-4 finality and
      package-vote quorum demonstrated on real hosts
- [ ] Four independent, non-voting observer hosts sync the same h1/h2 and serve
      read APIs; none is co-located with another required node
- [ ] I3/I4/I7 closed: spec current, v2 access recorded, firewall posture documented
- [ ] Hardening images deployed fleet-wide (I6) + Phase B green
- [ ] 48 h soak: tip advancing, no stuck rounds, no rate-limiter bans, disk growth sane
- [ ] Faucet funded and rate-limited; publisher onboarding doc linked
- [ ] Explorer + edge monitored (uptime check on `/v1/health` and `/v1/public/chain/stats`)
- [ ] Incident runbook reachable; on-call path defined
- [ ] `creg-test/` keys confirmed out of git (gitignore or relocated)

## 12. Quick reference

- Edge stats: `https://api.testnet.cregnet.dev/v1/public/chain/stats`
- Edge health: `https://api.testnet.cregnet.dev/v1/health`
- Chain spec: `https://spec.testnet.cregnet.dev/chain-spec.json`
- v1 API (on box): `http://localhost:8080/v1/public/chain/stats`
- Observer API (on main VM): `http://localhost:28182/v1/public/chain/stats`
- Containers: v1 `creg-validator-validator-1`, observer `creg-unified-node`,
  v2: `sudo docker ps --format '{{.Names}}' | grep -iE 'valid|creg-node'`
- v1 internal IP (VPC): `10.142.0.4`; observer external: `35.225.225.20`
- Runbook: `chain-registry/testnet/VALIDATOR-RUNBOOK.md`

## 13. Phases A/B execution evidence and Phase C decision (2026-09-25)

This section supersedes the tip-0 status and pending Phase A/B actions above.
It records the run before any chain reset or Phase C deployment.

### A/B results

- Published benign package `npm:creg-heal-smoke-20260925@0.1.2` to validator-1.
  Analysis completed; the package reported `SB010` because Docker sandbox
  isolation was degraded in this local test environment.
- h1 finalized on both validators with 2/2 PBFT signatures. Block hash:
  `916e01fde31fcc16ba5cb6bdaa69f8b95c97e1d428a7b5105fa682febf15fc2d`.
- The revoke finalized on validator-1 as h2. Block hash:
  `832ee073abc136a80725b880f28f2e1f96d475ee022f84d5ef61b95d0f9bf22b`.
  The package status is revoked on validator-1.
- Last observed tips: validator-1=h2, validator-2=h1, observer=h0. Validator-2
  had 28 non-terminal PBFT rounds. The observer rejected h1 with
  `block validator-set hash does not match authenticated authority`.
- The h1 header committed validator-set hash
  `d1373093dbad6f7a235d7e13840d38973e95161c277980c921e0cbe9f1ee4070`.
  The signed bootstrap authority hashes to
  `18edf57ff34767b7de1cf82e575465f1caa65c47cfd43b1fc43ae6488e127e66`.
  The h1 hash matches the runtime set after performance score/contribution
  updates, confirming node-local metadata entered the header commitment.

### History recovery decision

For this testnet, use a coordinated reset to the same signed genesis after
backing up the current data and retaining the hashes above. Do not wipe only one
node. The reset has **not** been performed. A compatibility migration is an
alternative if preserving this chain becomes a requirement.

### Phase C gate

- Local code now derives new block headers and PBFT quorum from the authenticated
  authority for the block height, and rejects proposal processing when that
  authority cannot be resolved.
- A local stall timer cannot authorize a backup proposer. The fallback path
  retains work until a quorum-certified PBFT view change selects a new leader.
  Local implementation requires protocol review and real-network validation.
  The 2026-09-25 review corrected idle view exhaustion, allowed a certified
  replacement to propose even without locally buffered work, and retained
  prepared values across later views in memory. Review found that copying only
  a prepared transaction root into a replacement header changes the block
  hash (the hash also binds proposer, timestamp, and view). A validator that
  committed the old hash could then commit the new hash before learning either
  quorum, allowing two hashes at one height. Local code now rejects such a
  replacement proposal and refuses to broadcast a locally rejected PBFT vote.
  This fail-closed rule sacrifices failover liveness for a prepared round.
  Durable vote/commit locks, exact-block reproposal, and prepared-state
  recovery across restarts remain unresolved; this is a safety gate, not a
  passing acceptance result. See `docs/PHASE-C-PBFT-SAFETY.md` for the
  counterexample and implementation gate.
- A Phase C feature image was built locally on 2026-09-25 but not deployed.
  Do not deploy until the real four-validator Sepolia roster is funded,
  admitted, and bound to a valid signed genesis, and the fresh-chain h1/h2
  agreement and four-observer sync are verified.

## 14. Required real-node topology and acceptance gate (2026-09-25)

This is the target for the next Phase C acceptance run. The four new hosts
are provisioned, but the new identities and CREG processes are not. The
new signed genesis must contain the four voting validator identities; the
four observers use that genesis but do not enter its voting set. The checked-in
`chain-spec.sepolia.json` contains two active validator entries. The
whitepaper's older snapshot calls its four foundation `validator-1`…
`validator-4` identities non-voting observers. Keep the new four voting
validator identities distinct from the four observer identities, regardless
of those historical names.

| Slot | Role | Required placement |
|------|------|--------------------|
| V1 | Voting validator and real package security pipeline | This laptop |
| V2 | Voting validator and real package security pipeline | Existing `creg-validator-1` GCP VM in `us-east1-b` |
| V3 | Voting validator and real package security pipeline | `creg-validator-3`, GCP `europe-west1-b` |
| V4 | Voting validator and real package security pipeline | `creg-validator-4`, GCP `asia-southeast1-b` |
| O1 | Non-voting observer and existing edge services | Existing `creg-testnet-vm` GCP VM in `us-central1-b` |
| O2 | Non-voting observer | Existing `creg-observer-vm` GCP VM in `us-central1-a`; node not installed yet |
| O3–O4 | Non-voting observers | `creg-observer-3` in `us-west1-b` and `creg-observer-4` in `africa-south1-a` |

All eight nodes must have their own runtime, persistent data volume, node
identity, and reachable peer/sync path. Distinct VMs reduce shared host failure;
independent operators or cloud accounts further strengthen the whitepaper's
decentralization goal. Observer identities do not count toward validator,
package-vote, PBFT, or view-change thresholds. With four active validators,
the normal package and block quorum is 3-of-4, and a new-view certificate
also requires 3-of-4 signed view-change messages. The `f + 1` (2-of-4)
threshold in the original PBFT protocol can prompt a replica to send its own
view-change message; it does not authorize replacement leadership. The local
Phase C code uses 3-of-4 for new-view certificates but still needs protocol
review and proof from real independent nodes before deployment.

No simulated nodes, mock packages, mock votes, degraded scanner profiles, or
in-process network substitutes count as acceptance evidence. Record for each
real host its region/zone, operator, node ID, public key, chain-spec signature
verification, reachable peers, observed tip and block hashes, package-vote
evidence, PBFT certificates, and relevant logs. Keep secret keys operator-side.
Preserve the §13 hashes and data/log backups before any coordinated reset;
the existing divergent h1/h2 cannot be repaired by the code change.

### Live host inventory and preserved evidence (2026-09-25)

The current network remains running on its old images. No reset occurred.
`docs/testnet-heal-evidence/2026-09-25-pre-reset/` contains captured public
block/stats responses, selected live logs, host inventory, and SHA-256 digests.
These are evidence snapshots. Full data-volume archives for the three running
nodes and the stopped O2 node were taken on 2026-09-25 under
`C:\Users\samue\CREG-backups\2026-09-25-pre-phase-c`; hashes are recorded in
the evidence manifest. No coordinated reset has occurred.

| Host | Region / placement | Current CREG role | Current tip | Phase C placement |
| --- | --- | --- | --- | --- |
| This laptop, Docker Desktop | Operator laptop | `creg-validator-validator-2` | h1 | V1 after coordinated reset |
| `creg-validator-1` | GCP `us-east1-b` | `creg-validator-validator-1` | h2 | V2 after evidence-preserving reset |
| `creg-testnet-vm` | GCP `us-central1-b` | Public observer plus edge/spec/IPFS services | h0 | O1 after evidence-preserving reset |
| `creg-observer-vm` | GCP `us-central1-a` | No running CREG container; old stopped containers preserved | none | O2 after observer installation; fresh 30 GiB data disk mounted at `/var/lib/creg` |

On 2026-09-25, four new Compute Engine VMs were provisioned in distinct
regions in billed project `fiffi-500411`. Each has its own 20 GiB Debian 12
boot disk and a separate persistent data disk mounted at `/var/lib/creg`.
Docker is active on all four, but there are no CREG containers, validator
keys, or observer processes on them yet. The existing laptop and three CREG
GCP VMs can fill V1, V2, O1, and O2 after the evidence-preserving coordinated
reset and observer installation. This is machine and regional independence
within one provider/account; it does not demonstrate independent validator
operators or provider diversity described in the whitepaper's broader
decentralization goal. See `docs/PHASE-C-FLEET.md` for
the live infrastructure inventory. The four new prepared hosts were stopped
on 2026-09-25 after confirming that they carried no CREG processes or keys.
All four now report `TERMINATED`; their disks and static addresses still incur
charges.

### Sepolia roster and custody blocker (2026-09-25)

A read-only snapshot at Sepolia block `11781329` found that the deployed
Staking Hub has only one active validator. All four planned Phase C EVM
addresses from the fresh key manifest are state `None`, with zero stake,
zero liquid CREG, and zero ETH. The old spec's validator-1 address is the
single active address with 100 CREG staked; its validator-2 address is also
state `None`. The governance contract is a 2-of-3 multisig and its emergency
validator-admission path remains enabled. A first signed Phase C spec had
hardcoded these four stakes; it was removed from the local deployment path
and public candidate endpoint, and archived privately as invalid. The spec
generator now refuses any inactive or underfunded roster before writing.

Two live governance signer private keys were embedded in the tracked
`testnet/approve-validator.js` source. Their literals were removed in the
working tree, but both signers must be rotated before using governance or
moving treasury funds. Four validator wallets also need real CREG, ETH gas,
stake applications, and on-chain admission. No funding, admission, chain
reset, or Phase C deployment has occurred. A locally built image is not an
acceptance result; the eight-host real-package run remains outstanding.

GCP project `gen-lang-client-0022105784` has its global
`CPUS_ALL_REGIONS` quota fully used (12/12). Four `e2-medium` additions need
eight more vCPUs. A request for a limit of 20 was submitted on 2026-09-25;
GCP immediately returned a granted value of 12 and said it could not grant
20 at that moment. The newly created empty project `creg-net-fleet-20260925`
could not be linked to the existing billing account because that account has
reached its project-link quota. It has no VMs and billing is disabled.

Read-only checks found that billed projects `fiffi-500411` and
`gen-lang-client-0098858574` each had 32 global CPUs available, with
regional E2 CPU and IP quotas available in Europe West, US West, Singapore,
and Johannesburg. The four new labeled CREG hosts use `fiffi-500411`;
its unrelated terminated VMs were not changed or repurposed. Do not count
these prepared VMs as running CREG nodes.

On 2026-09-25, created the recurring GCP billing budget
`CREG multi-region fleet - fiffi` for project `fiffi-500411`:
US$200 per calendar month, current-spend alerts at 50%, 80%, and 100%, with
credits excluded so alerts track gross usage. Budget ID:
`fe247f07-e41c-46bd-a9cd-652334e2302d`. The signed-in operator has the
billing-admin role and receives default billing alerts. This is an alert
budget, not a spending cap; existing unrelated persistent disks in this
project also count. The four new VMs now accrue compute, disk, and address
charges while running. The budget itself did not create or start them.

## 15. Historical Phase C deployment status (2026-09-26; superseded by R21 state above)

### Latest r9 / r7 result

The current r9 candidate is running on all eight real hosts. Scanner readiness,
shared publication h1 and shared revocation h4 passed; h2/h3 are empty blocks.
Independent hash, transaction-root and three-signature COMMIT checks passed for
heights 1–4 on all eight hosts. Committed-state restart checks passed on six
hosts, but V3 retained one queued transaction and forced entry after restart.
V4's restart and prepared-state recovery remain outstanding. The first section
of this document and the r9 evidence README give the current details. Earlier
r1–r4 paragraphs below describe preserved historical runs. No public reset or
cutover has occurred.

This section supersedes the earlier provisioning and custody blocker snapshots.
Fresh Sepolia contracts and governance keys were deployed after the user chose
to retire the exposed old custody setup. All four real validator addresses were
funded, staked 100 CREG each and admitted; a separate publisher staked 10 CREG.
The exact contracts and roster are in
`chain-registry/testnet/phase-c/deployment-manifest.json` and the signed candidate
spec. Four real validators and four real observers ran independently on the
laptop and seven GCP VMs, using the normal 3-of-4 quorum and real package analysis.

Run r1 exposed a pre-activation namespace certificate mismatch. That repair was
built and deployed to all eight hosts in r2. A fresh publish collected three
signed approvals, but the block producer kept retrying an older authorization
that had expired during the rebuild. No h1 finalized. A real-laptop diagnostic
run on 2026-09-26 recovered three-vote view-change certificates and confirmed
the nested error: `publisher authorization is not live at block time`.

The expiry repair is written and compiled. It durably preserves expired,
unprepared publish work and its reason before retiring it from new proposals;
prepared-value reproposals remain unchanged. All eight candidate processes
are stopped, with their data and PBFT signing journals preserved. A new signed
chain identity is being prepared for the next acceptance run. The old public
chain is still running: **no old-chain reset or public cutover has occurred**.
Shared publish h1, revoke h2, observer sync and failover acceptance remain
outstanding. The four new GCP VMs are running and continue accruing charges.

### Latest r4 result and next repair

The expiry repair compiled and ran on all eight real hosts. A fresh, signed
0.1.2 publish was accepted and reached VERIFIED, but no h1 finalized. The next
failure was duplicate publisher authorization nonces within one proposed
batch. Source review found that transaction hashes serialized a nested
HashMap directly, so a record could acquire different hashes after durable
reloads. The queue treated these variants as new work and the block validator
correctly rejected the repeated nonce.

The next repair uses canonical sorted-key encoding for both durable work IDs
and Merkle leaves, and preserves ineligible unprepared nonce variants before
removing them from new proposals. It also binds the actual block-zero hash to
the verified signed spec: the earlier candidates used a constant actual
genesis block, so a new signed chain ID alone did not isolate PBFT parent
domains. The encoding change is gated by the Phase C build feature and
requires a fresh data directory; old signing journals must remain preserved.
The updated node and CLI image is compiling. All eight r4 processes are
stopped, with logs, data and final stats preserved. No public-chain reset or
cutover occurred. The four new GCP VMs remain running and billable.

See `docs/testnet-heal-evidence/2026-09-26-phase-c-candidate-r4/README.md`
for the r4 artifact, signed spec, real-package outcome and recovery evidence.

### Transaction and queue source review

The follow-up review found unordered-map serialization in prepared-value PBFT
signature payloads as well as transaction hashes. The reviewed batch now uses
one explicitly versioned sorted JSON encoder for both. It retains competing
nonce payloads until a valid candidate commits, checks durable prepared/COMMIT
locks before unprepared cleanup, and makes queue persistence and retirement
synchronous. Proposers and voting peers now use the same package approval
verifier as observers. Duplicate notifications and forced delivery obey terminal
status and retry deadlines.

Source parsing and whitespace checks passed. Compilation and real-node evidence
for these follow-up changes remain pending; the active r5 build contains an
earlier source snapshot. The queued r6 review has since been superseded by
the r7 genesis/recovery/client review recorded at the top of this document. Public
reset/cutover remains pending. Details are in
`docs/PHASE-C-TRANSACTION-QUEUE-REVIEW.md`.
# Explorer pipeline update — 2026-09-26

The public web explorer is connected to the old public `creg-testnet-1` chain
at h0. The separate real r7 four-validator/four-observer fleet reached h4.
Web display/event/cache/roster/access-state repairs were deployed and verified
on 14 real public routes; 17 real r7 routes also passed. The CLI reads both
chains and displays the real publication/revocation. Node monitoring repairs
are in the frozen r11 build and are not yet a public node deployment. See
[the complete explorer repair and remaining work](EXPLORER-PIPELINE-REPAIR.md).
The V3 retained-work failure, deferred V4 restart, recovery acceptance and
public cutover remain open. No chain reset was performed.
