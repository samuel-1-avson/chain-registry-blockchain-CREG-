// crates/consensus/src/lib.rs
// Practical Byzantine Fault Tolerance (PBFT) consensus engine.
// A block is finalised when ≥ ⌊(2n/3)⌋ + 1 validators sign it.
// This implementation covers the three PBFT phases:
//   PRE-PREPARE → PREPARE → COMMIT

pub mod pbft;
pub mod validator_set;
pub mod vrf;

// anyhow::Result is unused here
// common imports are handled within submodules
pub use pbft::PbftEngine;
pub use validator_set::ValidatorSet;

pub mod vote_accumulator;
pub use vote_accumulator::{CommitOutcome, IncomingVote, VoteAccumulator};
