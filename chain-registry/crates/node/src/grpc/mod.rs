// crates/node/src/grpc/mod.rs
pub mod server;

#[allow(unused_imports)] // Re-exported for creg-node main binary gRPC server assembly.
pub use server::{MyExplorer, MyRegistry, MyWatcher};
