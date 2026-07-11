# Legacy-to-target migration

1. Freeze unsupported claims and export legacy blocks, package records, pending
   receipts, contract addresses, signer inventories, and retention metadata.
2. Map each package to immutable artifact/source/manifest subjects; report
   unmappable records instead of silently dropping them.
3. Emit `legacy_imported` events that reference the original record and its
   confidence/limitations.
4. Establish ownership and identity proofs for new submissions; do not infer
   authority from historical presence alone.
5. Run the target evidence/policy pipeline and publish witnessed tree heads.
6. Switch clients to target freshness/inclusion verification only after a
   conformance window; retain the legacy chain as read-only migration evidence.

Rollback means returning clients to the last witnessed target checkpoint, not
rewriting or replacing finalized history.
