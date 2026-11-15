# ADR-016: Equivocation slashing (detect, prove, penalize)

- **Status:** Proposed. Increment 1 (detector and evidence) is implemented but not wired; the rest needs the decisions in section 9.
- **Context/problem (AUD-04):** validators bond 100 tCREG, and `Staking.sol` can slash, but nothing connects a double-sign to a slash. The node has no code path that calls a slashing function, so a validator that signs two conflicting blocks today loses nothing and leaves no record. Economic security is decorative until this exists.
- **Options:** (a) node auto-slashes with a hot key; (b) detect and publish evidence, penalize through Governance; (c) detect and publish evidence, penalize through a validator-confirmed slasher contract; (d) prove the offense on-chain (Ed25519 verifier or ZK).
- **Decision:** (b) now, (c) with the batched contract migration, (d) only if mainnet needs trustless proof. Detection is permissionless and runs on every node, observers included; evidence is self-contained and independently verifiable; penalties stay a governed decision until a contract can enforce them mechanically.
- **Security implications:** see section 7. A signature alone cannot frame a validator, and legitimate view changes cannot be punished.
- **Operational implications:** operators must never run two instances with one validator key and must not restore a validator from a stale backup. Both are real double-signs.
- **Migration implications:** (b) needs no contract change. (c) needs a redesigned `SlashingEvidence.sol` and a `Staking.setSlasher` call, which rides the single contract migration in the audit's phase 2.
- **Rejected alternatives:** see section 8.
- **Revisit conditions:** independent operators exist (slashing is symbolic while one party runs every validator), or the validator set grows enough that governed execution becomes a bottleneck.

## 1. What the existing code gets wrong

`consensus/src/slashing_relay.rs` already contained an `EquivocationDetector`. It was dead code (nothing outside its own test used it) and had defects that would have made it unusable:

| Defect | Consequence |
|---|---|
| The evidence id hashed the two block hashes in the order they were observed | Two honest nodes compute different ids for the same offense, so evidence cannot be deduplicated or referenced. |
| The id omitted the phase | A PREPARE offense and a COMMIT offense at one height and view collide. |
| Evidence held only the votes, not the block headers | The signed message is `creg-pbft-v1:<phase>:<block_hash>`. It binds neither height nor view, so two signatures do not prove they were cast in the same slot. A third party could not check the claim. |
| Signatures were never verified | A forged vote could frame a validator or fill a slot so that a genuine conflict was missed. |
| Every vote was kept forever | Unbounded memory growth. |

Increment 1 replaces it. Details in section 3.

## 2. What is slashable

The node's own signing journal (`node/src/pbft_safety.rs`) already defines what an honest validator never does. Evidence rules mirror it exactly, so anything the journal allows can never be evidence.

| Phase | Slot the journal locks | Conflict (evidence) |
|---|---|---|
| PREPARE | height, parent hash, view | two blocks in the **same height, parent and view** |
| COMMIT | height (`validate_commit_lock` refuses any second COMMIT at a height, even on another fork) | two blocks at the **same height** |

Not evidence: PREPAREs in different views (normal after a view change), PREPAREs on different parents, votes at different heights, and a PREPARE that contradicts the validator's own COMMIT (a possible extension; left out of version 1 to stay conservative).

Real-world hazards that are slashable by this rule and should be in the operator runbook: a hot-standby instance sharing the key, restoring a validator from an old data directory, and wiping the signing journal. The node already refuses to sign without its original journal; operators must not defeat that.

## 3. Evidence (implemented in increment 1)

`consensus::slashing_relay::EquivocationEvidence` is self-contained and portable (JSON):

- the accused public key and validator id, the phase and the height;
- two signed votes, each with the **full block header** it names, ordered by block hash;
- a deterministic `evidence_id = sha256("creg-equivocation-v1|phase|pubkey|height|parent|view|hash_a|hash_b")`, where parent and view are the PREPARE slot coordinates and `any` for COMMIT, and the two hashes are sorted. Every observer derives the same id.

`verify()` checks, with no node state: version; both signatures strictly (`verify_strict`) under the accused key over `creg-pbft-v1:<phase>:<hash>`; each header re-hashes to the signed block hash (the hash commits to height, parent and view); the slot rule above; ordering; and that the id matches the contents.

`EquivocationDetector` verifies each vote's signature before storing it, keeps the first vote per slot, and returns evidence when a second, different vote arrives. It is bounded (`prune_below(height)` plus a capacity backstop that evicts the lowest heights); evidence is kept. `import_evidence` verifies and deduplicates evidence from peers or disk. `slash_reason()` yields `equivocation:v1:<phase>:<evidence_id>`, the string passed to `Staking` so the on-chain `Slashed` event names the evidence.

## 4. Detection in the node (increment 2)

- **Hook point.** `node/src/p2p.rs`, the `PbftPrepare` and `PbftCommit` handlers, immediately after `validate_pbft_phase_message` succeeds and before `pbft_engine.prepare/commit` (which may suppress a duplicate vote, which is exactly the case to catch).
- **Header resolution.** The gossip message carries only `{block_hash, validator_id, signature}`. The node must map the hash to a header: from the PBFT round if known, otherwise from a bounded cache of every proposal seen, **including conflicting PRE-PREPAREs the engine rejects** (a Byzantine leader's second proposal is usually refused). A vote for a hash with no known header is held for a short window and resolved when the header arrives, or dropped.
- **Observers detect too.** They see the same gossip and need no journal, so detection can be canaried on observer-3 before any validator runs it, and adds no signing risk.
- **Persistence.** A `equivocation_evidence` column family in `chain.rocksdb`, keyed by `evidence_id` (a few KB per record). On restart, reload with `import_evidence`.
- **Pruning.** Call `prune_below(finalized_height - window)` when blocks finalize.
- **Surface.** `GET /v1/public/evidence/equivocation` and `/{evidence_id}`; metrics `creg_equivocation_evidence_total` and `creg_equivocation_tracked_votes`; a critical alert on any new evidence; a `creg evidence verify <file>` CLI command so signers can check evidence without trusting a node.

## 5. Penalty paths

| Phase | Mechanism | Needs |
|---|---|---|
| **1, now** | A Governance signer verifies the evidence, then submits a proposal calling `Staking.slashSeverity(validatorEvm, Critical, evidence.slash_reason())`. Two signers approve. | Nothing on-chain. The validator's EVM address comes from the signed spec or `Staking` events. |
| **2, contract migration** | Deploy a redesigned `SlashingEvidence` and authorize it with `Staking.setSlasher`. Validator nodes that independently verified the evidence submit a confirmation; at the quorum (3 today), after the window (3 days) and a `MIN_EXECUTE_DELAY` (1 day), the contract slashes and pays the reporter 10%. Fully mechanical, with a built-in delay instead of a separate timelock. | The existing contract defines `DOUBLE_SIGN` for package votes, not PBFT block votes, and cannot check Ed25519 itself. It must be redefined around this evidence and independently audited. |
| **3, if mainnet needs it** | On-chain proof through an Ed25519 verifier or the retired `DoubleSignProof` circuit. | Large gas or ZK cost; only for trustless penalties. |

**Timing.** `Staking.UNBONDING_PERIOD` is 14 days. A validator can be slashed while unbonding but not after withdrawing, so detection to execution must finish well inside that window. Phase 1 budget: seconds to detect, under a day to verify and propose, up to three days to execute (more once a Governance timelock exists), leaving about ten days of margin.

**Effect on the validator.** `slashSeverity` takes the severity percentage (Critical is 30%) of the active validator stake, increments `slashCount`, and ejects to unbonding at the third slash or when the stake reaches zero. Slashed funds go to `slashPool`, not a burn.

## 6. Testing

- Done: 18 unit tests on the detector and evidence (slot rules for both phases, order-independent ids, forged votes, tampering, JSON round trip, pruning, capacity).
- Increment 2: a node test with four validators (blocked by the missing four-validator fixture noted in AUD-03), including a hostile case that sends conflicting PRE-PREPAREs.
- Drill: in staging, run two instances with one key and confirm that every observer records identical evidence within seconds and that a governed slash executes on a Sepolia fork.

## 7. Security analysis

| Concern | Position |
|---|---|
| Framing an honest validator | Requires its signature on two conflicting blocks. Detector and verifier both check signatures strictly, so forged votes are rejected. |
| Punishing a legitimate view change | Impossible by construction: the rules are the journal's own, and different-view PREPAREs are excluded. |
| Evidence spam | Verification is two Ed25519 checks. Ids are deterministic, so duplicates collapse; storage is a few KB per distinct offense. |
| Stolen validator key | An attacker who equivocates with a stolen key gets the victim slashed. Evidence cannot tell the two apart. Mitigation is key custody (KMS/hot-key handling) and fast rotation, not this mechanism. |
| Detection gaps | A vote whose block header the node never saw cannot become evidence. Observing from several nodes and caching rejected proposals narrows this; it cannot be closed completely. |
| Reorgs and rollbacks | The journal forbids a second COMMIT at a height even across forks, so a legitimate rollback does not make an honest validator slashable. |
| Alpha reality | One operator runs every validator, so a slash today is symbolic. The value of building it now is the evidence trail and the rehearsed process. |

## 8. Rejected alternatives

- **Node auto-slashes.** Would give a validator's hot key or the node an L1 slashing authority, enlarging the blast radius of a node compromise. Slashing authority stays with Governance or a governed contract.
- **Slash on any two different votes.** Ignores slot rules and would punish normal view changes.
- **Put evidence in a CREG block as `Transaction::Slash` first.** Worth doing later for ordering and transparency (the variant exists and carries a reason string), but it adds a consensus-visible state change that needs its own rules; not required for detection or for phase 1.
- **ZK proof only.** The retired `DoubleSignProof` circuit and `ZKSlashingVerifier` add trusted-setup and audit cost for no testnet benefit.

## 9. Decisions needed

| # | Decision | Recommendation |
|---|---|---|
| 1 | Penalty per proven offense | Critical (30%) through the existing `slashSeverity`; an ejection policy for repeat offenders beyond the contract's three-strike rule. |
| 2 | Who may publish evidence | Anyone, automatically (detection is permissionless); only Governance executes in phase 1. |
| 3 | Reporter reward | None in phase 1; the 10% in the redesigned contract for phase 2. |
| 4 | Evidence retention | Keep indefinitely (a few KB each). |
| 5 | Include PREPARE-versus-own-COMMIT evidence in version 2 | Yes, after the version 1 rules have run on a canary without false positives. |
| 6 | Who verifies in the alpha | The maintainer, using `creg evidence verify`, until independent operators exist. |

## 10. Increments

1. **Done:** detector and evidence in `consensus`, with tests (not wired into the node).
2. Wire into the node (proposal cache, hooks, persistence, metrics, API), detection only, canaried on observer-3, then validators.
3. `creg evidence` CLI, alert rule, runbook, explorer view.
4. Governed-slash drill on a Sepolia fork; publish the operator guidance in section 2.
5. With the batched contract migration: redesign `SlashingEvidence`, authorize it as a slasher, and run the confirmation flow.
