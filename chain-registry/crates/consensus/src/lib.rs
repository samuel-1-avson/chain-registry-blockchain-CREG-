// crates/consensus/src/lib.rs
// Practical Byzantine Fault Tolerance (PBFT) consensus engine.
// A block is finalised when it reaches a PBFT quorum. Validator sets smaller
// than four use the standard ≥ ⌊(2n/3)⌋ + 1 threshold by default. Local/dev
// three-validator clusters can explicitly opt into a 2-of-3 majority via
// `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true`.
// This implementation covers the three PBFT phases:
//   PRE-PREPARE → PREPARE → COMMIT

pub mod forced_inclusion;
pub mod pbft;
pub mod validator_set;
pub mod vrf;

// anyhow::Result is unused here
// common imports are handled within submodules
pub use pbft::{PbftConfig, PbftEngine, ViewChangeSignal};
pub use validator_set::ValidatorSet;

pub mod vote_accumulator;
pub use vote_accumulator::{CommitOutcome, IncomingVote, VoteAccumulator};
