# Node API

The CREG validator fleet exposes a **REST API** for chain reads, package discovery, and operator endpoints. Use it from the CLI (`CREG_NODE_URL`), the explorer, or your own integrations.

## Base URL

| Environment | URL |
|-------------|-----|
| Public testnet | `https://api.testnet.cregnet.dev` |
| Local dev | `http://localhost:8080` |

## Interactive reference (Swagger UI)

The node ships **OpenAPI 3** documentation and a browser UI:

- **[Open Swagger UI](https://api.testnet.cregnet.dev/api-docs/)** — try endpoints, inspect schemas, download the spec
- **OpenAPI JSON:** [https://api.testnet.cregnet.dev/v1/openapi.json](https://api.testnet.cregnet.dev/v1/openapi.json)

The explorer **About** page also links to Swagger when you are browsing [explorer.testnet.cregnet.dev](https://explorer.testnet.cregnet.dev).

## Health check

```bash
curl -s https://api.testnet.cregnet.dev/v1/health | jq .
```

Public health (no auth): `GET /v1/health` and `GET /v1/public/health`.

## Common read endpoints

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/v1/public/chain/stats` | Chain height, validator count, genesis |
| GET | `/v1/public/packages` | List published packages |
| GET | `/v1/public/packages/:canonical` | Package metadata |
| GET | `/v1/public/blocks` | Paginated blocks |
| GET | `/v1/public/blocks/:height` | Block by height |

Publisher and validator routes require the appropriate credentials — see Swagger for auth headers and request bodies.

## Validator-set transitions

`GET /v1/validator/set-transitions` returns the node's persisted quorum-certified history in ascending effective-height order. Unsigned compatibility observations are intentionally omitted, so a new or rebuilt node cannot promote them into validator authority.

`POST /v1/validator/set-transitions` accepts a domain-separated Ed25519 quorum certificate from the validator set active immediately before the transition. The certificate is the authorization: a single node or operator key cannot rotate validator authority. Each receiver independently verifies every declared witness, the prior-set quorum, validator identity/key uniqueness, protocol count bounds, `effective_from_height` as its next block height, and `previous_block_hash` against its canonical tip before persisting a continuous, non-conflicting certificate. The route has a dedicated 1 MiB body limit matching the gossip envelope limit.

Accepted transitions propagate on the dedicated gossip topic `creg/v1/validator-set-transitions`. Gossip does not make a transition trusted; peers repeat the same certificate, continuity, replay, and fork-anchor validation before accepting it.

The stable transition ID hashes the signed transition fields, not the replaceable signature list. Two independently valid quorum subsets therefore converge on the same checkpoint and committee authority. Nodes still validate every received signature and retain the first accepted exact witness encoding; an exact witness digest can distinguish those audit records without changing consensus identity.

Responses are `202` for an accepted certificate (including a logically identical retry while the same tip is current), `401` for an invalid/insufficient certificate, `409` for a stale height, conflicting transition, checkpoint-history violation, or local tip-anchor conflict, `413` for a body above 1 MiB, `500` for a local validator-history read, retention, serialization, or durable persistence failure, and `503` when the configured checkpoint itself is malformed. New or rebuilt nodes should be provisioned with an independently verified `CREG_WEAK_SUBJECTIVITY_CHECKPOINT` before trusting post-checkpoint history.

## Namespace-authority attestations (implemented API, gated rollout)

The node router and shipped OpenAPI document implement the following paths.
Their presence in source is **not a claim that a deployed API has enabled them
or that reviewed ecosystem verifiers are available**:

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/v1/validator/namespace-authority/attest` | Ask one authenticated validator to separately fetch and verify bounded official evidence and `artifact_url` bytes using its independently installed, signed-spec-pinned verifier, then return its signed authority statement |
| POST | `/v1/validator/namespace-authority/certificates` | Validate a caller-assembled deterministic prior-set quorum certificate for a claim or recovery and return its digest |

Both routes require `CREG_OPERATOR_API_KEY` and accept it as either
`X-Operator-Key: <token>` or `Authorization: Bearer <token>`. This service ACL
is additional defense for expensive validator-local work: it does not replace
the controller signature on the fresh intent, anchor-set membership for an
attestation, or the preceding-set quorum required by a certificate. Missing,
empty, or incorrect operator credentials fail closed.

The key is server-side operator material. It must not be placed in a `VITE_*`
variable, browser bundle, public nginx configuration, URL, or request body.
Browser clients must not receive it; operators call these routes from a trusted
client or server-side proxy.
`CREG_OPERATOR_PUBKEY` is public verification material and is never accepted
as a private-route credential; only `CREG_OPERATOR_API_KEY` satisfies this ACL.

The attestation route has a dedicated per-source-IP bucket of 2 requests per
minute. Verifier execution defaults to 2 concurrent processes and rejects a
configured value above the hard maximum of 4. Exhausted rate or execution
capacity is not authority evidence and must be retried without bypassing the
ACL or certificate rules.

The implemented protocol flow is:

1. A proposed controller signs a fresh chain/genesis/namespace/controller intent.
2. A new npm/PyPI artifact contains that intent; registry provenance or attestations identify the upstream publisher and exact artifact.
3. The attestation request includes official `evidence_url` and `artifact_url` values. Each validator separately downloads both with redirects disabled and independent limits (`CREG_NAMESPACE_AUTHORITY_MAX_EVIDENCE_BYTES` and `CREG_NAMESPACE_AUTHORITY_MAX_ARTIFACT_BYTES`). It hashes evidence with SHA-256 and the artifact with the registry-native algorithm: SHA-512 for npm, SHA-256 for PyPI.
4. A successful validator archives evidence as `creg-evidence:sha256:<digest>` and artifact bytes under `CREG_DATA_DIR/authority-artifacts/<sha256|sha512>/<prefix>/<digest>.bin`.
5. The publisher collects `floor(2n/3) + 1` matching signatures from the authenticated validator set, assembles and validates the certificate, and attaches it to the final v4 package authorization or recovery event.
6. Consensus checks the certificate, policy pins, native artifact identity, validity window, and canonical block/set anchor without contacting npm or PyPI.

The sidecar's `verified_at` is validator-local freshness input only. The node
checks that value against the challenge and local clock, but does not place it
in the shared proof or statement. Validators processing the same canonical
request therefore sign byte-identical statement content despite completing
verification at different seconds.

Both routes must remain request-size limited and fail closed when an official artifact cannot be fetched, the native digest differs, or the verifier executable, signed policy, trust root, or chain anchor is unavailable or mismatched. For npm, a provenance envelope is not by itself proof that trusted-publisher authentication was used: the external verifier/policy must require an official npm authentication-mode signal. Real trust material, policy bundles, and reviewed npm/PyPI verifier artifacts remain external deployment gates. The shipped OpenAPI document defines the repository request/response schemas and status codes. Clients must still confirm that the deployed binary exposes and protects these routes before relying on them.

## Namespace lifecycle

`POST /v1/publisher/namespaces/lifecycle` accepts a signed delegation, transfer,
or recovery event. The action-specific routes
`POST /v1/publisher/namespaces/delegate`,
`POST /v1/publisher/namespaces/transfer`, and
`POST /v1/publisher/namespaces/recover` use the same versioned body but require
its signed `action` to match the selected route.

Delegation retains the root controller and grants only a signed, expiring
`publish` scope. Transfer requires the current root and successor to sign the
same event. Recovery requires a fresh recovery-purpose npm/PyPI authority
certificate for the proposed controller. Durable nonces, chain/genesis scope,
validity windows, controller state, and the prior-event hash are checked before
the event is queued and again during deterministic block replay.

## Hub API (this site)

The join portal has a small companion API for health and (later) SIWE sessions:

- `GET https://testnet.cregnet.dev/api/health` — hub-api status (not the chain node)

## Related docs

- [Documentation hub](/docs)
- [Publish guide](/publish)

Note: hub-api lives at `/api/health` on this host; this guide is at `/api-reference` to avoid conflicting with that proxy.
