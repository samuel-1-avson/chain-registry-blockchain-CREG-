# ADR: KMS and secret storage for hot secp256k1 keys

| Field | Value |
|-------|--------|
| **Status** | Accepted (testnet); implementation tracked as SEC-301b |
| **Date** | 2026-05-28 |
| **Work items** | SEC-301a (this ADR), SEC-301b (provider code) |
| **Related** | SEC-101, SEC-101b, [SECURITY_OPS_RUNBOOK.md](../SECURITY_OPS_RUNBOOK.md) |

## Context

Three satellite capabilities load **Ethereum secp256k1 private keys** from environment variables today:

| Env var | Consumer | On-chain risk |
|---------|----------|----------------|
| `CREG_BRIDGE_KEY` | Node bridge worker | Spurious or malicious L1 txs |
| `FAUCET_PRIVATE_KEY` | Faucet service | Testnet fund drain |
| `RELAYER_PRIVATE_KEY` | Relayer (gas sponsorship) | Abuse of sponsorship policy |

Phase 2 added **startup warnings** with key fingerprints when keys are loaded outside `CREG_TESTNET=true` (SEC-101b). That improves observability but does not remove plaintext secrets from process memory, compose files, or CI caches.

Mainnet launch sign-off must not depend on long-lived hex keys in `.env` or Kubernetes `Secret` objects without rotation, audit, and least-privilege access controls comparable to a KMS.

## Decision

1. **Sepolia / dev testnet (now):** Continue **env-based keys** with `CREG_TESTNET=true` where operators accept the risk; document rotation in the security runbook. Optional **HashiCorp Vault** for teams that already run it locally.
2. **Staging / pre-mainnet:** Require **SEC-301b** — a single `SecretsProvider` abstraction with **Vault** as the first integrated backend and **env fallback** only when `CREG_SECRETS_BACKEND=env` is explicitly set (dev).
3. **Mainnet:** **No plaintext hot keys** in production compose or ConfigMaps. Keys must be loaded via KMS/Vault; env fallback is **disallowed** when `CREG_TESTNET` is not true (fail-fast at boot, extending `validate_production_security()`).

Ed25519 validator/publisher keys (`CREG_VALIDATOR_KEY`, publish key files) are out of scope for this ADR except where the same provider later stores file paths or HSM handles.

## Options considered

| Option | Pros | Cons | Fit |
|--------|------|------|-----|
| **Env / `.env` only** | Zero new infra; matches current scripts | No audit trail; leaks via logs/backups; manual rotation | Testnet only |
| **HashiCorp Vault** | Self-hosted; dynamic secrets; familiar to ops | Another service to run; policy authoring | **Recommended first backend (SEC-301b)** |
| **AWS KMS** | Managed; IAM integration; CloudTrail | AWS lock-in; signing latency; not ideal for local dev | Production on AWS |
| **GCP Cloud KMS** | Same as AWS on GCP | GCP lock-in | Production on GCP |
| **Azure Key Vault** | Enterprise AD integration | Azure lock-in | If deploy target is Azure only |

We do **not** adopt cloud KMS for SEC-301b first because the repo’s testnet path is Docker Compose + optional k8s without a mandated cloud. Vault (or env) keeps the abstraction testable in CI.

## Recommended architecture (SEC-301b)

```
┌─────────────────┐     get_secp256k1_signing_key("bridge")
│  Node / Faucet  │ ───────────────────────────────────────► SecretsProvider trait
│  / Relayer      │                                              │
└─────────────────┘                    ┌──────────────────────────┼──────────────────────────┐
                                       ▼                          ▼                          ▼
                              EnvSecretsProvider          VaultSecretsProvider        (future) AwsKmsProvider
                              CREG_*_KEY                  VAULT_ADDR + path           IAM role + key ARN
```

**Configuration (proposed):**

| Variable | Purpose |
|----------|---------|
| `CREG_SECRETS_BACKEND` | `env` \| `vault` (default `env` until SEC-301b ships) |
| `VAULT_ADDR` | Vault API base URL |
| `VAULT_TOKEN` | Auth token (dev); production uses K8s auth or AppRole |
| `CREG_VAULT_SECRET_PATH_BRIDGE` | e.g. `secret/data/creg/sepolia/bridge` |
| `CREG_VAULT_SECRET_PATH_FAUCET` | Faucet key path |
| `CREG_VAULT_SECRET_PATH_RELAYER` | Relayer key path |

**Interface sketch (Rust):**

```rust
pub trait SecretsProvider: Send + Sync {
    fn secp256k1_signing_key_hex(&self, role: HotKeyRole) -> anyhow::Result<String>;
}

pub enum HotKeyRole { Bridge, Faucet, Relayer }
```

Load once at process startup; reuse existing `warn_hot_key_from_env` only when backend is `env`.

## Testnet operator guidance

1. Keep using `.env` for Sepolia reuse ([TESTNET_SEPOLIA_RUNBOOK.md](../TESTNET_SEPOLIA_RUNBOOK.md)); set `CREG_TESTNET=true` on the node.
2. For shared labs, stand up Vault dev mode, store three keys under separate paths, set `CREG_SECRETS_BACKEND=vault`.
3. Run the rotation procedure in [SECURITY_OPS_RUNBOOK.md](../SECURITY_OPS_RUNBOOK.md) after any exposure.

## Mainnet gate

Before mainnet sign-off:

- [ ] SEC-301b merged with Vault (or chosen cloud KMS) backend
- [ ] `validate_production_security()` rejects `CREG_SECRETS_BACKEND=env` when not testnet
- [ ] k8s manifests use External Secrets Operator or CSI driver — no raw hex in `Secret` data
- [ ] Rotation drill executed and logged (extends SEC-101)

## Consequences

- **Positive:** Clear path from current Sepolia ops to production-grade key handling; one trait for bridge/faucet/relayer.
- **Negative:** Vault (or cloud KMS) operational burden; SEC-301b is non-trivial but bounded.
- **Neutral:** AWS/GCP KMS can be added as additional `SecretsProvider` impls without changing call sites.

## References

- [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](../SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) — Epic 3.4
- [PHASE3_KICKOFF.md](../PHASE3_KICKOFF.md)
- `chain-registry/crates/common/src/hot_key.rs` — SEC-101b warnings
