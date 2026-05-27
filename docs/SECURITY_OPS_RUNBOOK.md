# Security Operations Runbook

Operational security guidance for Chain Registry nodes and satellite services.

## Hot private keys (critical)

These environment variables load **secp256k1 signing keys** into process memory:

| Variable | Service | Risk if leaked |
|----------|---------|----------------|
| `CREG_BRIDGE_KEY` | Node bridge worker | Unauthorized L1 transactions |
| `FAUCET_PRIVATE_KEY` | Faucet | Drain testnet funds |
| `RELAYER_PRIVATE_KEY` | Relayer | Sponsored stake abuse |

**Rules:**

- Never commit keys to git. Use `.env` locally and a secret manager in production.
- Rotate immediately if a key is exposed.
- Prefer KMS/Vault integration (see SEC-301 in the implementation plan).

Startup logs emit a **warning** (key fingerprint only) when bridge/faucet/relayer hot keys are loaded outside testnet mode.

## Unsafe environment variables

| Variable | Effect | Allowed when |
|----------|--------|--------------|
| `CREG_DEV_SANDBOX=true` | Skips behavioural sandbox | Local dev / `CREG_TESTNET=true` only |
| `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true` | 2-of-3 PBFT quorum | Local 3-validator bootstrap only |
| `CREG_ALLOW_UNSAFE=true` | Bypasses production security guards | **Never** on mainnet; emergency local only |

The node **refuses to start** on production-like networks (`CREG_TESTNET=false` and chain/network id contains `mainnet`) when unsafe flags are set unless `CREG_ALLOW_UNSAFE=true`.

## Operator API

| Variable | Purpose |
|----------|---------|
| `CREG_OPERATOR_API_KEY` | Protects `/v1/operator/*` and `/v1/internal/*` |

If unset, private routes return **503** (fail-closed).

## TLS

Enable HTTPS on the node REST API with:

- `CREG_TLS_CERT`
- `CREG_TLS_KEY`

(requires build with `tls` feature)

## Production compose checklist

Before running `docker-compose.prod.yml` or K8s prod manifests:

- [ ] `CREG_DEV_SANDBOX=false`
- [ ] `CREG_TESTNET=false` only for intentional mainnet-beta
- [ ] `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false` on production networks
- [ ] `CREG_OPERATOR_API_KEY` set
- [ ] Contract addresses non-zero
- [ ] No hot keys in ConfigMaps (use secrets)

## Verification

```bash
creg doctor
```

Fails on `CREG_DEV_SANDBOX=true` and reports other unsafe combinations on non-testnet profiles.
