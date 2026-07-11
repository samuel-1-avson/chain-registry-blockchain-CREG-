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
