//! Helpers for simulations, stress tests and `creg stress`.
//!
//! Nothing here is consensus code or admission evidence. The Merkle helpers
//! moved from the retired `cross-chain` crate (ADR-019 Increment 6, `ECA-26`):
//! real L1 state-root anchoring lives in `node::bridge`, and block inclusion
//! proofs use `resolver::light_client::build_merkle_proof`.

pub mod merkle_proof;
