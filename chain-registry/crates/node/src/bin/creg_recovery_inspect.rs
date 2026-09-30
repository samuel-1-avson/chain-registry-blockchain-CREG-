use anyhow::{Context, Result};
use std::path::Path;

fn main() -> Result<()> {
    let data_dir = std::env::args_os()
        .nth(1)
        .context("usage: creg-recovery-inspect <copied-node-data-directory>")?;
    let store = node::chain_store::ChainStore::open_read_only(Path::new(&data_dir))?;
    let report = store.finalized_work_diagnostics()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
