# Wallet key derivation (CREG CLI)

**SEC-105** — operators must understand how CREG keys relate to Ethereum addresses.

## Two different identities

| Purpose | Key type | Used for |
|---------|----------|----------|
| CREG consensus / publish signatures | **Ed25519** (32-byte secret, 64 hex chars) | `CREG_VALIDATOR_KEY`, `creg publish --key`, PBFT votes |
| On-chain ETH / staking / `cast send` | **secp256k1 EOA** (standard Ethereum wallet) | Paying gas, `stakeAsPublisher`, `joinAsValidator` |

Generate Ed25519 keys with:

```bash
creg keygen publisher
creg keygen validator
```

## Derived `0x` address shown by `creg keygen`

After keygen, the CLI may print an **ETH address (derived)**. That value is computed by:

1. Taking your **Ed25519** secret (32 bytes)
2. Interpreting those bytes as a **secp256k1** private key
3. Applying the usual `keccak256(uncompressed_pubkey)[12..]` Ethereum address formula

This is a **convenience hint only**. It is **not**:

- A BIP-39 / BIP-44 path (`m/44'/60'/0'/0/0`)
- The address MetaMask or Ledger show for the same mnemonic
- Safe to use as “the wallet for this validator” without verifying on-chain

**Do not send Sepolia/mainnet ETH to the derived address unless you have verified you control it with a real secp256k1 key.**

## Validator registration on-chain

Validators register a **separate** `evm_address` (your real staking wallet) plus Ed25519 pubkey via the node API. The chain spec bootstrap validator `eth_address` in `chain-spec.sepolia.json` is an example EOA — not auto-derived from your keygen output.

## `creg stake` and Foundry `cast`

`creg stake --key-file` runs `cast send` with the file contents as `--private-key`. That flag expects a **standard Ethereum private key** (64 hex chars of secp256k1 scalar, often with `0x` prefix).

If the file is from `creg keygen`, the command **refuses** to run and points here.

## Recommended operator flow (Sepolia)

1. `creg keygen validator` → save Ed25519 key; set `CREG_VALIDATOR_KEY`.
2. Use a normal wallet (MetaMask, `cast wallet new`, etc.) for Sepolia ETH and staking txs.
3. Register validator identity on the node with your **real** `evm_address` and Ed25519 pubkey.
4. Wait for `validator_set_sync.state = synced` on `/v1/health`.

## Related docs

- [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) — hot keys and env vars
- [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) — Sepolia deploy and node boot
- [PHASE2_SEPOLIA_KICKOFF.md](./PHASE2_SEPOLIA_KICKOFF.md) — Option A reuse checklist
