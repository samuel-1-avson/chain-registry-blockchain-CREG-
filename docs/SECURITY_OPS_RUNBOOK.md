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
- On Sepolia and other testnets, set `CREG_TESTNET=true` on the node so hot-key warnings are suppressed when keys are expected.

Startup logs emit a **warning** (key fingerprint only, never the secret) when `CREG_BRIDGE_KEY`, `FAUCET_PRIVATE_KEY`, or `RELAYER_PRIVATE_KEY` are loaded while `CREG_TESTNET` is not `true`.

### CREG Ed25519 keys vs Ethereum wallets (SEC-105)

`creg keygen` produces **Ed25519** secrets for consensus and publish signatures. The CLI may print a **derived** `0x` address — that is **not** a MetaMask/BIP-44 account. Do not send funds to it without verifying control with a real secp256k1 key.

`creg stake --key-file` requires a standard Ethereum private key for `cast send`; it **rejects** key files from `creg keygen`.

See [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md).

### Rotation procedure (exposed or departing operator)

1. **Stop** the affected service (node bridge worker, faucet, or relayer).
2. **Generate** a new key (`cast wallet new` or your KMS).
3. **Fund** the new address on the target L1 network if it must submit transactions.
4. **Update** the secret in your vault / `.env` (never commit).
5. **Revoke** old key on-chain if it held roles (governance, staking operator, etc.).
6. **Restart** the service and confirm logs show the new fingerprint in the hot-key warning (non-testnet) or no warning when `CREG_TESTNET=true`.
7. **Record** incident in your security log if the old key was exposed.

### Sepolia RPC for validator set sync

`validator_set_sync` polls `eth_getLogs` on `CREG_STAKING_ADDR`. Lightweight public RPCs often return empty or malformed `eth_getLogs` results.

- Prefer Infura, Alchemy, or QuickNode: set `CREG_ETH_RPC` or pass `-RpcUrl` to `run-sepolia-reuse.ps1`.
- Default reuse script RPC is `https://1rpc.io/sepolia` (better than some public endpoints).
- On failure, logs include the JSON-RPC `error` field when the provider returns one.

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
