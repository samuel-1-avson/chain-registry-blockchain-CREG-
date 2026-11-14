# API, Explorer and CLI route contract audit

**Captured:** 2026-09-27 (Africa/Accra)  
**Purpose:** W4 route audit for the Phase C public-cutover plan.  
**Method:** source review plus read-only requests to the live R7 candidate node and public API edge. No API credentials, writes, mocked responses or chain changes were used.

## Live environment findings

| Surface | Observed result | Meaning |
| --- | --- | --- |
| Candidate node `http://127.0.0.1:18080` | `/v1/public/chain/stats` returned chain `creg-phase-c-testnet-2026-09-26-r7`, tip h14. `/metrics` returned Prometheus text (`text/plain; version=0.0.4`, 9,459 bytes). | Candidate R7 API is live and has real current metrics. |
| Public API `https://api.testnet.cregnet.dev` | `/v1/public/chain/stats` returned `creg-testnet-1`, tip h0. `/metrics` returned HTTP 200 JSON gateway metadata, not Prometheus metrics. | Public API and candidate R7 remain separate. The public metrics URL does not reach the node's Prometheus endpoint. No cutover was made. |
| Candidate and public operator history routes | Before repair, unauthenticated requests to `/v1/operator/metrics/history` and `/v1/metrics/history` returned HTTP 401; the old authorized handler returned HTTP 501. New source adds `/v1/public/metrics/history?range=...` with fixed series and chain-identity filters. | Operator ACL behavior remains fail closed. The public route is source-only: the currently deployed R22 candidate returns 404. No operator credential is sent to browser code. |
| Metrics collector configuration | `creg-gcp-prometheus` is restored and healthy using the persistent `monitoring_prometheus-gcp-data` volume with 14-day retention. Its 15-second R7 observer target is `up`, and filtered queries return real h14 samples labelled with the R7 chain ID and signed-spec genesis identity. | Collector restoration and initial real samples are verified. See [collector restoration evidence](metrics/collector-restoration.md). Seven-day history fits inside the current retention; 30-day retention still requires measured capacity review. |

## Route-to-consumer matrix

| Data path | Web explorer consumer | CLI consumer | Contract and current status |
| --- | --- | --- | --- |
| Health and chain stats: `/v1/public/health`, `/v1/public/chain/stats` | `useChainStats`, dashboard, About, Metrics | TUI bootstrap and status views | Candidate and public routes returned HTTP 200, but identify different chains (R7 h14 vs legacy h0). Public cutover remains pending. |
| Block list/detail/hash: `/v1/public/blocks*` | Dashboard, block list/detail, transaction links | TUI block views and `creg blocks` | Candidate h13 and h14 block details returned HTTP 200. W2/W3 evidence records shared R7 finality. Public edge still serves h0. |
| Package list/detail: `/v1/public/packages*` | Dashboard, package list/detail, publisher profile | TUI package list and CLI package commands | Candidate Kleur detail returned HTTP 200 and `status: revoked`, with the committed reason. The detail page previously rendered status with a neutral badge and did not display the reason; the local source now renders revoked as an error state with its reason. |
| Transaction by canonical: `/v1/public/transactions/:canonical` | Transaction detail | CLI search/inspection paths | Candidate returned the real Kleur revoke transaction at h14. W2/W3 live CLI and browser captures include the actual named events. SSE remains live-only; callers must attach before the write. |
| Publication inclusion proof: `/v1/public/packages/:canonical/proof` | Proof verifier and transaction detail | `creg verify` full light-client path | Candidate returned HTTP 404 after Kleur was revoked even though its publication remains in canonical h13 and the CLI had verified that proof before revocation. Cause: `proof::build_proof` rejected any current record not `Verified`. Local source now retains proofs for both `Verified` and `Revoked` records and returns current `package_status`; the UI labels current revocation and explains that inclusion does not establish install safety or quorum finality. This source fix is not deployed to R7 yet. |
| Events: `/v1/public/events` (legacy `/v1/events`) | Shared `useSse` feed | `creg watch`; TUI event panel | W2/W3 captured actual `block_produced`, package verification and `package_revoked` events with live consumers attached. No event replay is provided by SSE; this is a documented live-stream contract, not a historical feed. |
| Validator consensus and registrations | Validator pages and consensus page | CLI TUI health/validator views | Validator endpoints are not part of the public route set; some require identity/signature admission. Keep them distinct from public observer reads. Do not expose operator credentials to browser clients. |
| Operator runtime, topology, peers and pending pool | Network and Pending pages | Operator CLI / TUI when configured | Private API ACL applies. Explorer client intentionally strips any `scope` option and renders an explicit unavailable fallback on 401/403/404/405/501/503. This is fail-closed behavior, not public data. |
| Metrics history: `/v1/public/metrics/history?range=...` | Web Metrics page requests durable samples from the public API; CLI TUI refreshes the same route once every 30 seconds. | Node source queries a fixed Prometheus metric allow-list using the node's configured chain ID and signed network genesis identity. Web and CLI source now render stored samples and unavailable states. | Node/resolver and CLI cargo checks plus web production build pass. The API and clients are not yet deployed; candidate returns 404. Verify API range data, browser/CLI rendering and retained samples after collector/API restart before closing W4. |
| Prometheus scrape: `/metrics` | No direct web chart consumer | Monitoring scrape | Candidate node returns Prometheus text. Public API edge currently returns gateway metadata JSON at the same path, so it is not a working public scrape route. Preserve the private collector boundary; expose only an allow-listed history API after collector integration. |
| Bridge, governance, reorgs, rich list and search | Corresponding routed pages with optional fallbacks | CLI search/data views where implemented | Source has grouped public endpoints and explicit optional fallbacks. W4 browser verification and response capture must report empty/unavailable data distinctly from a genuine empty chain result. |

## Repairs started and remaining

1. **Started in source:** preserve a historical publication proof after later revocation; include the current package lifecycle status in the proof response; show the revoked status/reason in Package Detail and Proof Verifier.
2. **W4 integration gate:** collector restoration, server-side history source and web/CLI source integration are complete. Roll the reviewed source to the candidate API and verify real range results, browser/CLI rendering and retained samples after a collector/API restart. Decide whether measured disk growth supports extending the existing 14-day retention to 30 days. Do not fabricate or backfill samples.
3. **Confirmed public routing gap:** the public `/metrics` URL resolves to gateway metadata. Keep public cutover pending; route only after the actual collector/API target is reviewed.
4. **Contract docs:** align OpenAPI metric-history parameters with the implemented request shape and document authorization/status codes. Capture route response shapes for the remaining advertised pages before closing W4.
5. **Not a dead historical pipeline:** SSE is intentionally non-replaying. W2/W3 acceptance attached the CLI and browser before each real package write and captured the resulting named events.

## Verification limitation

The explorer production build completed successfully. `cargo check --locked -p chain-registry-node -p resolver` could not compile `librocksdb-sys` on this Windows host because bindgen could not locate `clang.dll`/`libclang.dll`. This is an environment prerequisite, not a Rust diagnostic. The source changes have not been built or deployed to any live node. No test suite was run.
