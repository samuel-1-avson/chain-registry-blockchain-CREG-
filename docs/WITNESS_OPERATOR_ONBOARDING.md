# Witness operator onboarding (WITNESSED-LOG Track B)

**Updated:** 2026-08-24  
**Audience:** Organizations willing to run an independent CREG transparency-log witness  
**Status:** recruiting. Empty roster is expected. This is not a live witness network.

A witness is **not** a validator. You do not stake tCREG, join PBFT, or run `creg-node` as a committee member. You independently fetch tree heads, check consistency against the last head you accepted, and sign with a key that is **not** the log operator’s key.

The install client already fail-closes without those signatures (`CREG_TRANSPARENCY_LOG_PUBKEY` + `CREG_TRANSPARENCY_WITNESS_KEYS`). Closing WITNESSED-LOG means **you** actually run that check on a host the log operator does not control.

## What is true today

| Item | State |
|------|--------|
| Merkle log + inclusion/consistency proofs | In the node (unsigned `/head` is monitoring only) |
| Client `verify_witnessed_inclusion` | In `creg install` when witness keys are configured |
| Log-signer process / witness daemon | **Not shipped** (Track A). Generating a key is recruitment, not witnessing |
| Independent witness organizations | **0** — roster is empty |
| Public GCP API | Paused. Witnesses do not need `api.testnet.cregnet.dev` |

Do not generate `witness-a` and `witness-b` on one laptop and call this item done. The log signer id cannot count as a witness (`common::transparency_log`).

## Independence bar

You count as independent only when **all** of these are not controlled by the log operator or another already-listed witness:

- Legal entity / governance
- Ed25519 witness key (generated on your machine)
- Host or cloud account
- Monitoring and incident decision path

Lab close for WITNESSED-LOG: **N ≥ 2** independent operators plus a split-view drill. Public-testnet gate: **four** independent witness organizations ([ACCEPTANCE_GATES.md](./testing/ACCEPTANCE_GATES.md)).

## 1. Generate a key (on your machine)

```bash
creg keygen witness --key-path ~/.creg/witness.key
```

- Default path is `~/.creg/witness.key`. The CLI **refuses** to write a private key inside a git worktree.
- Send **only** the printed Ed25519 public key (64 hex chars) plus the control-domain statement below.
- Never commit `*.key`, paste the secret into chat, or store it on the log-operator host.

Choose a stable `witness_id` (your org slug). Clients pin:

```bash
export CREG_TRANSPARENCY_WITNESS_KEYS='{"your-org-id":"<public-key-hex>"}'
```

Ceremony detail: [WITNESS_KEY_CEREMONY.md](./WITNESS_KEY_CEREMONY.md).

## 2. Control-domain statement

Email or PR a **pubkey-only** candidate. Include:

- Organization legal name
- `witness_id` you will use
- Ed25519 public key hex
- Where the key was generated (machine class; not a path into this repo)
- Who owns the host you will use (cloud account / DC)
- Who decides incidents if two heads disagree
- Confirmation you are not the log operator and do not share that host or key

Maintainer records this as `candidate_unapproved` in [`chain-registry/testnet/witnesses/roster.json`](../chain-registry/testnet/witnesses/roster.json). Approval is not automatic.

## 3. After Track A ships (not yet)

You will run a witness binary that:

1. Polls the log’s tree head for a published `log_id`
2. Verifies consistency against your last accepted head
3. Signs with your witness key
4. Publishes the signature on **your** URL (or a file clients fetch via `CREG_WITNESSED_EVIDENCE_PATH`)
5. Alerts if the log presents two incompatible heads of the same size

Until that binary exists, keep the key offline. Do not flip `CREG_REQUIRE_WITNESSED_HEAD` against unsigned `/head` — clients will correctly fail closed and nothing independent is witnessing.

## 4. Split-view drill (closes Track B)

Two processes, two control domains. The log (or a test fixture) serves two incompatible heads. Each witness must refuse or alert. Clients with both pubkeys pinned must fail closed. A unit test that signs twice in one process is not this drill.

## Related

| Document | Role |
|----------|------|
| [WITNESS_KEY_CEREMONY.md](./WITNESS_KEY_CEREMONY.md) | Log-signer vs witness keys |
| [WITNESS_OPERATOR_OUTREACH.md](./WITNESS_OPERATOR_OUTREACH.md) | Invite text |
| [CLIENT_VERIFICATION.md](./protocol/CLIENT_VERIFICATION.md) | What `creg install` already checks |
| [TRANSPARENCY_LOG.md](./protocol/TRANSPARENCY_LOG.md) | Log semantics |
| [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md) | Different role — do not mix |
