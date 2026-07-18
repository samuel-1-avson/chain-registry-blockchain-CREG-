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

`POST /v1/validator/set-transitions` accepts a domain-separated Ed25519 quorum certificate from the validator set active immediately before the transition. The certificate is the authorization: a single node or operator key cannot rotate validator authority. Each receiver independently verifies the prior-set quorum, requires `effective_from_height` to be its next block height, checks `previous_block_hash` against its canonical tip, and persists only a continuous, non-conflicting certificate.

Accepted transitions propagate on the dedicated gossip topic `creg/v1/validator-set-transitions`. Gossip does not make a transition trusted; peers repeat the same certificate, continuity, replay, and fork-anchor validation before accepting it.

Responses are `202` for an accepted certificate (including an exact retry while the same tip is current), `401` for an invalid/insufficient certificate, `409` for a stale height, conflicting certificate, checkpoint-history violation, or local tip-anchor conflict, and `503` when the configured checkpoint itself is malformed. New or rebuilt nodes should be provisioned with an independently verified `CREG_WEAK_SUBJECTIVITY_CHECKPOINT` before trusting post-checkpoint history.

## Hub API (this site)

The join portal has a small companion API for health and (later) SIWE sessions:

- `GET https://testnet.cregnet.dev/api/health` — hub-api status (not the chain node)

## Related docs

- [Documentation hub](/docs)
- [Publish guide](/publish)

Note: hub-api lives at `/api/health` on this host; this guide is at `/api-reference` to avoid conflicting with that proxy.
