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

This is a migration foundation, not a production witness deployment. The
current node endpoints return unsigned heads/proofs for monitoring and test
integration; no client may treat them as qualified without independently
distributed signed heads, configured witness keys, freshness policy, and a
persisted prior head.
