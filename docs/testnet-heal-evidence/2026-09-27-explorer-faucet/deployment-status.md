# Public explorer and faucet deployment — 2026-09-27

## Explorer

- Public URL: `https://explorer.testnet.cregnet.dev/`
- Release: R24, image digest
  `sha256:090ba37ef9f0ee228d40d9f296d7fdafde1ef224f66f6015500c13d7f9dae6c7`.
- Live root returned HTTP 200. Browser verification confirmed the aggregate
  stake displays as `200 CREG`, genesis displays as finalized height `0`, and
  the page produced no JavaScript errors or HTTP 5xx responses. See
  [`explorer-public-r24-verification.json`](explorer-public-r24-verification.json)
  and [`explorer-public-r24-dashboard.png`](explorer-public-r24-dashboard.png).
- R24 is active; R23 and R14 are stopped and retained for rollback. The node/API
  was not restarted or rerouted.
- The explorer continues to report the public legacy chain at h0. Its live API
  reports 2 validators, 200 CREG stake and 4 pending transactions. R7 remains a
  separate candidate network; no public chain, API, signed-spec or contract
  cutover was performed.

## Faucet

- Public URL: `https://faucet.testnet.cregnet.dev/health` returned HTTP 502.
- `creg-unified-faucet` is restarting from `creg-node:genesis2`. Its startup
  failure is an empty `FAUCET_PRIVATE_KEY`.
- Read-only Sepolia `balanceOf` queries against the public legacy token contract
  `0x167Ab975dc95b79727Ed76BD8AAb70463F6499E2` returned:
  - Configured legacy faucet wallet: `495000` CREG.
  - Address derived from the available faucet Secret Manager key:
    `0` CREG.
  - Other tracked faucet wallet: `20500000` CREG, with no matching known
    signer secret.
- No signing key was printed and no token transfer was sent. The available
  secret cannot sign for the configured funded wallet. Pointing the faucet at
  the secret-derived wallet would require funding it with at least one normal
  1000 CREG drip from an authorized token holder.

## Faucet repair prepared in source

- `crates/faucet/src/main.rs` now checks that the configured signing key derives
  the configured faucet address before serving.
- PostgreSQL is mandatory; absent or unreachable cooldown storage prevents
  startup. Address and IP cooldowns are reserved together under PostgreSQL
  transaction locks before funds are sent. Storage failure returns unavailable
  and never proceeds to a transfer.
- Both `docker-compose.unified.yml` and `docker-compose.cloud-edge.yml` now
  declare a PostgreSQL 16 service with a persistent data volume and require a
  URL-safe database password. Neither publishes the database port. Both
  reference the standalone faucet image by immutable digest.
- The standalone Dockerfile now installs the protobuf/build tools required by
  the real Rust dependency graph, pins both base images, runs as UID 65532, and
  probes the configured 8082 service port.
- `cargo check -p faucet` passed. `cargo fmt --check -p faucet`,
  `git diff --check` for the changed files, and both Compose `config --quiet`
  checks passed. Compose checks used non-secret placeholder values; no service
  was started from those values.
- The Linux/amd64 faucet image built and was pushed to Artifact Registry as
  `us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-faucet@sha256:360c124090e5c3a5c448a69e9838bb2c1755ed18f6d32732bff6498d4cdd5ed5`.
  It was pulled to the public-services VM and inspected there: UID 65532 and
  the same image digest were confirmed. The container was **not started**; no
  database was created and the live faucet remains down.

## Deployment gates

1. Supply a Secret Manager key that matches the existing funded faucet wallet,
   or fund the address derived from the approved faucet key with a reviewed,
   bounded transfer on the legacy public token contract.
2. Store/provision the required faucet database credential for the public
   services host.
3. Deploy the pinned faucet image and its database without recreating the
   explorer, API or validator containers; then verify health, network/challenge
   routes, one actual Sepolia drip receipt, and cooldown rejection after restart.

No public chain reset or cutover is included in these steps.
