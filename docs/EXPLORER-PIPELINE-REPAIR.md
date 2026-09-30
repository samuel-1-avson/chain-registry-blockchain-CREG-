# Explorer pipeline repair — 2026-09-26

## Latest production pipeline review — 2026-09-27 21:32 UTC

The public faucet now runs the repaired standalone image with a dedicated fresh
Sepolia signer and persistent PostgreSQL. An approved reserve transfer and one
genuine token claim confirmed; a repeat claim was rejected after both services
restarted. Public UI, API and RPC checks passed. Native gas drips are configured
at zero; the fresh CREG reserve is 9,000 tokens after acceptance.

R25 explorer nginx now preserves grouped public/operator routes, supports its
actual runtime node-upstream configuration, uses real Sepolia for standalone
wallet RPC, and prevents stale caching of service-worker deployment entry points.
The final immutable build is now deployed publicly through the dedicated
`docker-compose.public-explorer.yml` project, digest
`sha256:ae0d80a7bd6f5534d4814dffdf613b66c4fee31843bfec69f713ce4319fbe122`.
The standalone RPC path uses the existing real public Sepolia gateway after
actual checks found 404/403 failures in earlier candidates. Final private R7,
private legacy and public browser checks each passed 14 pages plus real RPC,
with no JavaScript errors or HTTP 5xx responses. R24 is retained stopped for
rollback. Stored metrics survived real collector and observer restarts.

The public frontend still reads actual legacy h0, two validators and 200 CREG.
The separate private R7 candidate displays h14, four validators and retained
history. API/root spec/contract/default-chain cutover remains pending the
release-scope decision and applicable recovery, CLI and service acceptance gates.

Detailed identities, evidence, outstanding CLI/service paths and rollback limits
are in the [current pipeline audit](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/PIPELINE-AUDIT.md).

## Earlier public deployment — 2026-09-27 19:44 UTC

The public explorer now serves R24, image
`us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:090ba37ef9f0ee228d40d9f296d7fdafde1ef224f66f6015500c13d7f9dae6c7`.
The live dashboard shows aggregate validator stake in native CREG units and
reports genesis as finalized only when the actual block response marks it
finalized. Browser verification confirmed `200 CREG`, finalized height `0`,
HTTP 200, and no page errors or HTTP 5xx responses. R23 is retained stopped
for rollback; R14 also remains preserved. The explorer still reads the actual
public legacy chain at h0; no API, spec, contract or chain cutover was
performed. The deployment and browser evidence is in
[`2026-09-27-explorer-faucet`](testnet-heal-evidence/2026-09-27-explorer-faucet/).

R23's unconfigured relayer proxy repair remains in R24. It returns the node's
truthful 404 response instead of nginx's 502 caused by a nonexistent `relayer`
service. The public legacy node does not provide metrics history, so the
endpoint still returns 401 and the UI reports it unavailable. The live API
reports 4 pending transactions, 2 active validators, and 200 CREG total stake;
the explorer displays those real values. The genesis block endpoint marks h0
finalized, and the dashboard now displays that canonical state.

The faucet remains unavailable. The public URL returns 502 because its
container restart loop reports an empty signing key. The available faucet
Secret Manager key does not match the configured, funded legacy faucet wallet;
the key was not printed, and no tokens were moved. The faucet source now
validates signer/address pairing at startup and requires durable PostgreSQL
cooldown storage. Address and IP cooldowns are claimed atomically before a
transfer, with failure closed if storage is unavailable. Compose now declares a
persistent PostgreSQL volume and requires a database password. This source
repair compiles and both Compose profiles validate. A standalone Linux/amd64
image is pinned in Artifact Registry at digest
`sha256:360c124090e5c3a5c448a69e9838bb2c1755ed18f6d32732bff6498d4cdd5ed5` and
has been pulled to the public-services VM, but it has **not** been started. A
matching funded faucet key or an explicitly reviewed funding action is still
required before the faucet can send a real transaction.

## Live connection

The public web explorer and public REST API serve the same **old public
chain**, `creg-testnet-1`, at height **0**. Its actual genesis block hash is
`068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53`.
The old public roster still reports two validators; that is not Phase C
acceptance evidence. The separate four-validator/four-observer R7 fleet is at
finalized **h12**, hash
`a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d`, with actual genesis
`0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
No chain reset, public cutover or default-spec replacement occurred.

The production web explorer is connected to the old public chain through the
live public REST API and the real Sepolia JSON-RPC route. It does not yet serve
R7; public routing remains on `creg-testnet-1` at h0. The R7 browser checks
used the real candidate V1 endpoint and eight real candidate nodes, with no
fixtures or intercepted API responses. Browser captures show the h11
`npm:ms@2.1.3` publication and h12 revocation through candidate event delivery.

The local CLI build now reads the same candidate h10–h12 blocks. `creg watch`
uses `/v1/public/events`; the node also exposes `/v1/events` as a working SSE
compatibility route used by other CLI surfaces. Both returned HTTP 200 with
real heartbeat/ping frames. The watcher stayed connected without retrying.
A named event has not yet been observed in CLI output because the stream has no
history replay. The R21 sequential restart and post-restart readiness evidence
show all eight candidate nodes on the pinned artifact at h12 with empty work
queues and no active PBFT round.

## Applied and verified

| Path | Repair | Evidence |
| --- | --- | --- |
| Dashboard and metrics height | Map `tip_height` to the display field | Real r7 h4 and public h0 browser reads |
| Block list/details | Read nested header, proposer and PBFT signatures; normalize authorized publication records | Real h1 publication and h4 revocation |
| Public genesis list | Fetch real `/blocks/0` when an older observer omits it from the list | Deployed public genesis page |
| Transaction details | Read nested record/publisher and distinguish revocation from publication | Real r7 package and transaction views |
| Validator list/profile | Display active consensus roster even when registration cache is empty; accept node IDs; use CREG stake units | Four real r7 validators and two old public validators |
| Network and pending panels | Report protected details as unavailable; obtain aggregate counts from public stats | HTTP 401 retained; no operator credential exposed |
| Live browser events | Subscribe to six named registry event types, preserve event timestamps and fix stale-state closure | Browser subscription checks and real SSE heartbeat |
| CLI live watch | Use the explicit public SSE route; verify the legacy SSE alias used by other CLI surfaces | [Live candidate route and CLI capture](testnet-heal-evidence/2026-09-26-phase-c-events-r16/r21-rollout/sse-r21-cli-route-check.json); named event rendering in CLI still needs capture |
| Runtime identity | Use public health/stats for chain identity | Both actual genesis identities |
| Browser caches | Use network reads for chain data so a reset cannot reuse cached blocks at the same URL | Production build and new service worker |
| Rich list | Show actual active validator stakes when the old registration cache is empty | Real consensus response |
| Metrics and proofs | Label samples as browser observations; remove guessed throughput; permit empty sibling path for single-transaction Merkle proof | Build and real-response projections |
| CLI block explorer | Honor saved endpoint configuration; reject invalid limits/HTTP failures; display genesis and authorized publication | Real CLI output on both chains |
| CLI console | Read public roster/identity, parse actual canonical IDs, represent unavailable consensus/finality honestly, bound initial block load and retry failed reads | Live console and exported report |

R14 (`sha256:1e9b0b54d4bd66b83c3bea3452db371005b4d3bd835bf5571fa27e81824dd244`)
was the previous production web release. It was checked on O1's existing
Docker network before deployment; R23 now supersedes it. R13 and R14 deployment
state remains available for rollback, and the public node was not restarted.
Production nginx routes the signed root spec and versioned r7 spec/signature
paths to the actual spec service; `/rpc` reaches real Sepolia and returned
`0xaa36a7`. Native CREG RPC reaches the public chain and returns h0.

The revised UI passed **17 routes against the actual r7 node**, and **8 extended
routes on the deployed R14 public explorer**, with no JavaScript page errors or
failed requests on that public run. The later R23 public deployment also passed
its eight-route check, recorded in the current deployment section above.
Earlier API checks covered 22 public API
routes and 44 Swagger operations. No API responses were intercepted or
replaced. The first browser runs exposed a case-sensitive assertion and a
health-probe timing assumption; their failures were retained. Public
verification waits for the real subscription to open. The real h5 publication event capture timed out because committed events were
missing and consumer envelopes differed. That failure remains preserved.
The repaired node/CLI compiled into R18 and all eight guarded replacements
passed. The actual h6 toidentifier publication delivered submission, validator
votes, committed block and package verification through the real candidate
web explorer and Docker CLI; the browser rendered them without errors. The
CLI independently verified its Merkle/quorum proof against signed R7.

The actual h7 revocation exposed a missing signed-intent propagation path; its
failure and timeout are preserved as historical evidence. Later R21 rollout
and h11/h12 browser captures prove candidate publication, revocation and
event delivery. The earlier Windows CLI R15 binary was installed at both
`C:\Users\samue\.cargo\bin\creg.exe` and
`C:\Users\samue\.creg\bin\creg.exe`; the corrected watcher has since been
built locally from source, but the packaged/public CLI installer has not been
released. The h12 watcher route and block-read results are recorded above.

## Unresolved node/API data paths to recheck before public cutover

- Recheck that canonical CREG finality remains distinct from the L1 checkpoint
  height across R21 candidate and public APIs.
- Verify genesis listing, pagination boundaries, bounded package search and
  validator proposal history against live R21 responses.
- Return an explicit unavailable response for unconfigured historical metrics;
  browser samples are not a durable server time series.
- Use persisted bridge anchors only. The bridge remains disabled under the
  safety freeze.
- Reconfirm stake scaling, block ordering and reset detection against both
  live endpoints before cutover.

The earlier R12/h5 fleet paragraph was a prior snapshot and is superseded by
the current R21/h12 evidence at the top. The public legacy node remains
unchanged. Prepared-state evidence is limited to V4 recovering its h11 prepared
value and reconciling the certified canonical block; local durable COMMIT
recovery was not observed. Reward acceptance remains open.

## Remaining work before public cutover

1. Decide whether V4 acceptance requires observing local durable COMMIT replay
   before peer sync. Existing evidence proves prepared-value recovery and
   certified canonical reconciliation; local COMMIT replay was not observed.
2. Verify CLI publish and render a named SSE event with a fresh operator-
   selected package. The real web explorer already rendered h11 publish and
   h12 revoke events; CLI build, block reads and candidate/public SSE connections
   are proven.
3. Release the corrected CLI package and verify the installed binary against
   candidate and public APIs.
4. Recheck the unresolved node/API data paths above, then verify faucet,
   wallet/staking/reward and relayer writes. Complete durable metrics history
   if required by the product. Broaden advisory coverage before treating
   scanner output as general package-risk clearance.
5. After these gates pass, prepare the coordinated public API, explorer, CLI,
   signed-spec and contract cutover. Preserve existing public-chain evidence;
   no public cutover or chain reset has been performed.

## Evidence

- [Real HTTP, browser and CLI evidence](testnet-heal-evidence/2026-09-26-explorer-pipelines/)
- [r11 frozen source/build evidence](testnet-heal-evidence/2026-09-26-explorer-node-review-r11/)
- [Real fleet scanner/finality/restart evidence](testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9/README.md)
- [R14 web, R15 CLI and current eight-node readiness/recovery evidence](testnet-heal-evidence/2026-09-26-explorer-pipelines/README.md)
- [R21 restart, V3 dead-letter, V4 recovery, and candidate publish/revoke evidence](testnet-heal-evidence/2026-09-26-phase-c-events-r16/r21-rollout/)
