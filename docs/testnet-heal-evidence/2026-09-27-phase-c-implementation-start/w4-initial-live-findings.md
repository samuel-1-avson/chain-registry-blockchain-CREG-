# W4 Initial Live Findings

Capture date: 2026-09-27 (UTC)

## Scope and chain context

The route audit used read-only GET requests against the candidate and public API/explorer. The candidate remains on the accepted R7 chain at height 12. The public API and explorer still serve the legacy `creg-testnet-1` chain at height 0. No public cutover or chain reset was performed.

The raw route response captures are under this evidence directory's `w4-read-route-audit` subdirectory when present. The audit evidence includes public route responses and contains no operator key.

## Confirmed findings

- The authenticated candidate `GET /v1/operator/metrics/history?range=1h` returned HTTP 501. This is a real implementation gap: durable metrics history is not being served by the candidate node. A `200` from current in-memory metrics would not satisfy retention; implementation and restart evidence remain required.
- Two candidate routes returned different values under the same `genesis_hash` name. The public `/v1/public/chain/stats` response returned the hash of canonical block zero (`0f2a3b…`), while a protected, authenticated `/v1/operator/runtime/config` GET returned the signed chain-spec network identity (`1852b9…`, `spec_signature_verified=true`). The published transaction's authorization is bound to the signed spec identity. These are distinct concepts, not evidence that the R7 package was signed for another chain, but reusing one JSON field name can mislead clients. W4 should expose clearly named `genesis_block_hash` and `network_identity_hash` fields, retain/document any compatibility field, and audit explorer, CLI and trust checks.
- The sampled `npm:is-number@7.0.0` package is revoked. Its package record and transaction route are readable, but its proof route returns HTTP 404. The proof builder explicitly returns no proof unless the current package status is `Verified`, so this revoked-package response alone does not establish that the verified-publication proof path is broken.
- The CLI `creg info` command treats proof retrieval as optional and omits `spv_proof` when the endpoint returns an error. The web Proof Verifier displays the fetch error. W2 must obtain a fresh genuinely verified publication and exercise proof verification; W4 should render revoked/not-eligible proof state with a clear explanation rather than an unexplained fetch failure.
- The public explorer/API are intentionally still on the legacy h0 chain pending W8. This is not evidence of a broken candidate connection; public cutover remains gated.

## Remaining W4 evidence

- Complete the route contract matrix against real consumers and upstream routes.
- Exercise a fresh verified publication's proof in the API, browser verifier, and CLI.
- Trace the metrics-history endpoint to its storage/collector and implement persistent retention and range queries; prove samples survive a real collector/API restart and preserve gaps.
- Verify explorer event reconnect behavior against a fresh real named event from W2.
- Continue dead-letter review after any new real write, preserving terminal duplicates and avoiding replay to manufacture evidence.

## Safety boundaries

No write request, node interruption, chain reset, public cutover, or VM stop/delete was performed for this audit. Secret values and operator credentials are excluded from this report.
