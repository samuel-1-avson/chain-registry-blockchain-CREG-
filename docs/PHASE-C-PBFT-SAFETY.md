# Phase C PBFT safety gate (2026-09-25)

The four-validator target requires three distinct package approvals, three
COMMIT signatures and three signed votes for a certified new view. The four
observers never vote. All eight actual R7 candidates currently run R18; public
routing remains on the old h0 chain.

## Latest real evidence (2026-09-26, h6)

All eight hosts share independently verified h1–h6 hashes, transaction roots
and at least three distinct Ed25519 COMMIT signatures per block. V3 terminal
work and V4/all-eight committed-state restarts passed on earlier images. R18
replacement/startup passed on every host; stale same-canonical rounds cleared.
The actual h6 block reached PREPARE quorum at view 0, then finalized its exact
original hash after a three-vote certified view change at view 1.

This natural prepared reproposal does not pass an in-flight crash gate. The
first planned h7 crash probe timed out before a prepared round appeared: the
actual owner revocation was retained on V1 and not propagated. No process was
stopped by that probe. R20 delivery/retirement source is compiling; it is not
yet deployed. Prepared/local-COMMIT recovery and two-online-validator refusal
plus quorum restoration remain open. No signing journal was copied or edited.
Current evidence is in `testnet-heal-evidence/2026-09-26-phase-c-events-r16/`.

## The prepared-block problem

`Block::hash()` binds the header's proposer, timestamp, and view as well as
the transaction root. The local view-change code previously required a new
leader to copy only the highest prepared transaction root. A replacement
header therefore had a different block hash even with identical transactions.

This can split a four-validator chain. A, B, and C may sign COMMIT for hash X;
A may see the quorum and finalize X while B and C have not yet seen enough
COMMIT messages. A 3-of-4 new-view certificate from B, C, and D can then
authorize a replacement header with the same transactions but hash Y. If B
and C sign Y, Y can also collect three COMMIT signatures. In-memory locks
that clear when the earlier round is marked failed do not prevent this.

The local repair now rejects any new-view proposal that changes the highest
prepared block hash. Ordinary builds keep the release gate closed; only an
explicit `phase-c-real-fleet` feature build enables certified view change.
Backups retain work while the primary is unavailable in ordinary builds. The repair
also suppresses local PREPARE, COMMIT, and view-change broadcasts when the PBFT
engine rejects their local registration. The engine retains duplicate-round
votes and rejects a second proposal in the same view, repeated PREPARE in one
view, or repeated COMMIT for a different hash at one height while its state
remains in memory. These are fail-closed guards, not completed proposer
failover: a stalled round may remain stalled.

Local signed decisions are recorded in a separate RocksDB database at
`<data_dir>/pbft-safety.rocksdb`. Each write requests synchronous persistence.
The database is bound to the canonical genesis hash, validator ID, and public
key; startup fails if those differ or a recovered signature does not verify.
PREPARE locks are scoped to height/parent/view, COMMIT locks to height/parent
across views, and view-change locks to height/parent/target-view. A local
view-change vote also advances a durable local view lock, which blocks an
older-view PREPARE after restart. The journal must be included in validator
backups. For a new signed genesis, archive the old journal with the old chain
and start a separate empty journal bound to the new genesis. Never reset a
journal while continuing the same genesis.

The current staged repair additionally syncs the full prepared block and its
PREPARE quorum before signing COMMIT. It syncs each verified signed view-change
vote before applying it in memory. Startup reconstructs the active prepared
round, restores its own signed COMMIT, and replays view-change votes against
the authenticated validator set; it refuses a prepared value or COMMIT that
conflicts with finalized history. This code still needs real fleet acceptance
before the candidate feature build can replace the live image.

## Required implementation before deployment

1. Separate the immutable proposed block value from the view/leader envelope.
   A new leader must be able to carry the exact prepared block hash forward
   with a 3-of-4 new-view certificate. The receiver must verify the leader,
   full certificate, highest prepared value, and exact block hash before it
   signs. A fresh value is allowed only when the certificate carries no
   prepared value.

   The implementation path is an explicit signed `REPROPOSE` message carrying
   the original full block unchanged, target view, replacement leader ID, and
   the verified 3-of-4 view-change certificate. Its signature must bind the
   original block hash and certificate. The original block header, timestamp,
   proposer, and any prior-view proof remain byte-for-byte intact. A prepared
   certificate must separately report the view in which PREPARE quorum formed;
   that may be later than the view embedded in the original block hash. The
   receiver must reactivate the same-hash round without discarding PREPARE or
   COMMIT votes and permit the original timestamp when this certified proof
   is valid. Block sync must verify the finalized original block and COMMIT
   quorum. The signed envelope and exact-hash reactivation are now staged in
   code behind the candidate feature. They still require real eight-host
   validation before cutover.
2. Finish and review crash recovery of the prepared block and certified view.
   The staged journal records the full prepared block, PREPARE quorum, and
   signed view-change inputs, then reconstructs the current tip at startup.
   Verify the path through a real validator restart during a prepared round.
3. Prune safety records only after the finalized block and its certificate are
   durable. Restarts and reorgs must not restore an earlier signing state or
   permit a conflicting vote at the same height.
4. Review both directions of P2P handling and block sync against these rules.
   Validate on the eight real independent hosts: four validators must agree
   on h1 and h2 with 3-of-4 certificates, and all four observers must sync.
   Exercise a real validator restart during a prepared round before declaring
   failover complete. Development-only checks do not count as acceptance.

Until all four items are complete, backups keep finalized work buffered and
Phase C stays undeployed. Preserve the divergent current-chain evidence and
take stopped-node data backups before any coordinated reset.

## Development checks on 2026-09-25

The consensus library checks passed (55/55), including duplicate proposal and
prepared-hash regression checks. The Linux node binary passed
`cargo check -p chain-registry-node --bin creg-node` in the local builder
container. The PBFT signing journal checks passed (3/3) against durable local
storage behavior. These checks do not establish four-validator
finality, crash recovery of prepared values, or observer sync.
