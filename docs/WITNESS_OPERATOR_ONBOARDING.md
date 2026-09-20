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
| Log signed-head + `creg witness run` | In-repo. Log operator sets `CREG_TRANSPARENCY_LOG_KEY`. Each witness runs the daemon on **their** host |
| Independent witness organizations | **0** — roster is empty |
| Public GCP API | Paused. Witnesses do not need `api.testnet.cregnet.dev` |

Do not generate `witness-a` and `witness-b` on one laptop and call this item done. The log signer id cannot count as a witness (`common::transparency_log`). Shipping the daemon does not close WITNESSED-LOG.

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

## 3. Run the witness daemon (your VPS)

The log operator (not you) must set `CREG_TRANSPARENCY_LOG_KEY` on the log node so `GET /v1/public/transparency/<log_id>/signed-head` returns a `SignedTreeHead`. Unsigned `GET /head` must not be witnessed.

On **your** host:

```bash
creg witness run \
  --node-url https://<log-host> \
  --log-id <id> \
  --log-pubkey <log-operator-ed25519-hex> \
  --witness-id <your-org-slug> \
  --key-path ~/.creg/witness.key \
  --out-path ~/.creg/witnessed-head.json
```

- Default poll interval is 30 seconds. `--once` fetches, signs, and exits.
- Encrypted keys need `CREG_WITNESS_KEY_PASSPHRASE`.
- State lives in `~/.creg/witness-state.json`. After split-view or rollback the process fail-closes until an operator inspects that file and removes `split_view`.
- Serve `witnessed-head.json` over **your** HTTPS origin (nginx, Caddy, object storage). Do not publish it from the log operator’s domain.

Clients with a witness policy set `CREG_WITNESS_HEAD_URLS` to a comma-separated list of those independent URLs. They must not treat unsigned node `/head` as install-eligible. Do not flip `CREG_REQUIRE_WITNESSED_HEAD` until at least two independent URLs actually serve matching signed heads.

Do not put this VM in the CREG GCP project or on CREG billing. Own the cloud account; invoice for reimbursement if a stipend is agreed.

## 4. Split-view drill (closes Track B)

Two processes, two control domains. The log (or a test fixture) serves two incompatible heads. Each witness must refuse or alert. Clients with both pubkeys pinned must fail closed. A unit test that signs twice in one process is not this drill.

## Related

| Document | Role |
|----------|------|
| [WITNESS_KEY_CEREMONY.md](./WITNESS_KEY_CEREMONY.md) | Log-signer vs witness keys |
| [WITNESS_OPERATOR_OUTREACH.md](./archive/WITNESS_OPERATOR_OUTREACH.md) (archived 2026-09-19) | Invite text |
| [CLIENT_VERIFICATION.md](./protocol/CLIENT_VERIFICATION.md) | What `creg install` already checks |
| [TRANSPARENCY_LOG.md](./protocol/TRANSPARENCY_LOG.md) | Log semantics |
| [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md) | Different role — do not mix |
