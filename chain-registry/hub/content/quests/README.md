# Hub quest definitions

Versioned quest and journey copy for the CREG testnet join portal (`join.testnet.cregnet.dev`).

## Layout (Phase 2+)

- `*.yaml` — quest definitions (`id`, `path`, `title`, `order`, `verification`, `prerequisites`)
- Shared, publish, and validate paths per [TESTNET-HUB-DESIGN.md](../../../docs/TESTNET-HUB-DESIGN.md)

## Phase 0

No quest files yet. `hub-api` will load definitions from this directory at startup in Phase 2.

## Example (future)

```yaml
id: publish_first_package
path: publish
title: Publish your first package
order: 4
verification: manual
prerequisites: [siwe_signin, install_cli]
```
