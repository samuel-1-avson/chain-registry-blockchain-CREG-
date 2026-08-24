# Witnessed transparency log

The log is an append-only Merkle history of submissions, ownership events,
evidence references, policy decisions, revocations, disputes, supersessions,
and signed tree heads. It supports inclusion proofs, consistency proofs, and
checkpoint freshness.

Witnesses independently fetch/gossip tree heads, compare consistency, and
publish equivocation alerts. A checkpoint contract stores only a root, tree
size, epoch, witness threshold, and metadata digest. It does not interpret
package safety.

## Client checks

1. Verify the log signature and inclusion proof.
2. Verify consistency from the pinned root/checkpoint.
3. Verify witness threshold and control-domain diversity.
4. Reject stale, expired, split, or inconsistent metadata.
5. Verify the event subject and exact artifact digest locally.

Tombstones and superseding events preserve history. Private data is represented
only by the minimum necessary commitment.

## Implemented protocol baseline

`common::transparency_log` implements version 1 of the wire-independent
commitment format. It is intentionally distinct from the legacy block Merkle
tree: an RFC 6962 split tree uses SHA-256 over `0x00 || event-bytes` for leaves
and `0x01 || left-hash || right-hash` for internal nodes. Events are
length-delimited and commit to a sequence number, kind, canonical subject,
payload digest, source label, and timestamp.

The node store persists events, leaves, and tree heads atomically and serves
tree heads, inclusion proofs, and consistency proofs. A tree head is only
install-eligible after an Ed25519 log signature and the configured threshold
of distinct trusted witness signatures. Client verification rejects stale or
future heads, invalid inclusion, and any update lacking a consistency proof
from its persisted head.

This is a migration foundation, not a production witness deployment.

When `CREG_TRANSPARENCY_LOG_KEY` is set, `GET /v1/public/transparency/:log_id/signed-head`
returns a `SignedTreeHead`. Independent operators run `creg witness run`, which
refuses unsigned `/head` bodies, checks consistency against the last accepted
head, and writes `WitnessedTreeHead` JSON for clients to fetch from the
witness’s own URL (`CREG_WITNESS_HEAD_URLS`). Without the log key, signed-head
is HTTP 503. Unsigned `GET /head` stays monitoring-only.

`GET /v1/public/transparency/:log_id/latest-event?subject=` returns the latest
event for an exact canonical subject plus an inclusion proof and the unsigned
head. The JSON sets `install_eligible: false`. `creg install` and PATH shims
always apply the local anti-rollback journal; they call
`verify_witnessed_inclusion` only when a witness policy is configured, and
they reject unsigned heads in that mode.

Operator recruitment (Track B) is [WITNESS_OPERATOR_ONBOARDING.md](../WITNESS_OPERATOR_ONBOARDING.md).
The pubkey-only roster at `testnet/witnesses/roster.json` starts empty; filling
it with keys generated on one laptop is not a witnessed network. The daemon
existing does not close WITNESSED-LOG.
