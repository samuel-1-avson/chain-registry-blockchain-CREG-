# WITNESSED-LOG Track B — witness operator outreach

**When someone replies:** record a **pubkey-only** `candidate_unapproved` row in [`chain-registry/testnet/witnesses/roster.json`](../chain-registry/testnet/witnesses/roster.json). Do not mark WITNESSED-LOG done. Do not accept a key generated on the log-operator laptop.

**Related:** [WITNESS_OPERATOR_ONBOARDING.md](./WITNESS_OPERATOR_ONBOARDING.md), [WITNESS_KEY_CEREMONY.md](./WITNESS_KEY_CEREMONY.md).

---

## Email subject

`CREG transparency witness — independent operator invite (public alpha, not mainnet)`

---

## Body (copy and customize)

Hello {{name}},

CREG is a public-alpha package registry (Rust node + `creg` CLI + Sepolia). We need **independent transparency-log witnesses**: organizations that fetch tree heads, check consistency, and sign with a key we do not control.

This is **not** a validator seat. You do not join PBFT or stake as a committee member. The public API (`api.testnet.cregnet.dev`) is paused; you do not need it to generate a key or later run a witness against a self-hosted log.

**What we are asking now**

1. Read https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/WITNESS_OPERATOR_ONBOARDING.md
2. On your own machine (not our repo checkout used as a key store): `creg keygen witness`
3. Send **only** the Ed25519 public key, a `witness_id` (org slug), and a short control-domain statement (legal entity, key host, incident owner). Never send the private key.

**What this is not**

- Mainnet, BFT, or a malware-proof claim
- A request to resume our GCP fleet
- Permission to run two witness identities on one host and call the network independent

`creg witness run` exists. Holding a key is still recruitment until you run it on a host we do not control and serve `witnessed-head.json` from **your** HTTPS URL. We will not flip client `CREG_REQUIRE_WITNESSED_HEAD` against unsigned node heads.

Lab bar is two independent witnesses plus a split-view drill. Public-testnet bar is four independent witness organizations.

Please reply with interest, earliest date you can generate a key, and whether your org can run a small Linux host later.

Thank you,  
{{maintainer}}

---

## Maintainer checklist after a reply

- [ ] Pubkey is 32-byte Ed25519 hex and was **not** produced in this git worktree
- [ ] `witness_id` is not the log-signer id
- [ ] Control-domain statement names a distinct entity, host, and incident path
- [ ] Roster row is `candidate_unapproved` (pubkey only)
- [ ] WITNESSED-LOG in [NEXT_WORK.md](./NEXT_WORK.md) stays open until N≥2 approved **and** the split-view drill
