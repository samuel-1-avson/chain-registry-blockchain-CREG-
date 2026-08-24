# Witness and log-signer key ceremony (WITNESSED-LOG Track B)

**Updated:** 2026-08-24  
**Audience:** Log operator (one) and independent witness operators (N)  
**Status:** ceremony specified. No production keys live in this repository.

This ceremony produces **public keys only** for the candidate roster. Private keys never enter git, CI, or chat.

## Roles (do not mix)

| Role | CLI | Signs | Must not |
|------|-----|-------|----------|
| Log signer | `creg keygen log-signer` | Tree heads (`sign_tree_head`) | Also be a witness; use a validator key |
| Witness | `creg keygen witness` | Witnessed heads (`sign_witnessed_tree_head`) | Share the log-signer host or key |

The verifier ignores a witness whose `witness_id` equals `log_signer_id`. Generating both keys on one laptop does not create independence.

## Log operator (one machine)

On a host that will later run the log (not required to be the paused GCP fleet):

```bash
creg keygen log-signer --key-path ~/.creg/log-signer.key
```

Record **only** the printed public key. Clients will set `CREG_TRANSPARENCY_LOG_PUBKEY` to that hex. The secret stays on this host. The live signer process is **not on main yet**; keep the key offline until that ships.

Do not add this pubkey to `CREG_TRANSPARENCY_WITNESS_KEYS`.

## Each witness operator (separate machines)

Each org, on **their** machine:

```bash
creg keygen witness --key-path ~/.creg/witness.key
```

The CLI refuses to write the secret inside a git worktree. Submit the public key through [WITNESS_OPERATOR_ONBOARDING.md](./WITNESS_OPERATOR_ONBOARDING.md). Status in the roster starts as `candidate_unapproved`.

## What maintainers commit

Pubkey-only rows in [`chain-registry/testnet/witnesses/roster.json`](../chain-registry/testnet/witnesses/roster.json). Empty `candidates` is the honest default until someone independent submits.

Never:

- Commit `*.key` or mnemonics
- Invent placeholder hex keys
- Mark WITNESSED-LOG complete because keygen exists
- Set `CREG_REQUIRE_WITNESSED_HEAD=true` on unsigned node `/head`

## After N ≥ 2 approved pubkeys

1. Publish the pinned client env (`CREG_TRANSPARENCY_LOG_PUBKEY`, `CREG_TRANSPARENCY_WITNESS_KEYS`, `CREG_TRANSPARENCY_MIN_WITNESSES`).
2. Ship Track A (log signer + witness binary + signed-head URLs) so those keys can be used.
3. Run the split-view drill in the onboarding doc.

That sequence closes Track B. Keygen alone does not.
