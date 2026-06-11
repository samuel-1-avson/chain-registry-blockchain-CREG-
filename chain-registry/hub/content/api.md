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

## Hub API (this site)

The join portal has a small companion API for health and (later) SIWE sessions:

- `GET https://testnet.cregnet.dev/api/health` — hub-api status (not the chain node)

## Related docs

- [Documentation hub](/docs)
- [Publish guide](/publish)

Note: hub-api lives at `/api/health` on this host; this guide is at `/api-reference` to avoid conflicting with that proxy.
