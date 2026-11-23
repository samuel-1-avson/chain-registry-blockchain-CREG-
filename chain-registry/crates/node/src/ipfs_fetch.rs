// crates/node/src/ipfs_fetch.rs
//! Bounded, injection-safe IPFS fetcher (`VNA-11`, `VNA-13`). The
//! implementation lives in `validator::ipfs_fetch` so `creg-agent-runner`
//! uses the same bounds for Lane C (ADR-019).

pub use validator::ipfs_fetch::{fetch_ipfs_bounded, validate_cid_for_node, IpfsFetchError};
