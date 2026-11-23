//! Export a sled-era `creg-relayer` journal (`<RELAYER_DATA_DIR>/relayer-state.db`)
//! to the JSON format the PostgreSQL relayer imports once
//! (`RELAYER_LEGACY_JOURNAL_EXPORT`, ADR-019 Increment 6).
//!
//! Stop the old relayer first: sled holds an exclusive lock on the database.
//!
//! ```text
//! cargo run --release --manifest-path tools/relayer-sled-export/Cargo.toml -- \
//!     <RELAYER_DATA_DIR> <output.json>
//! ```

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

const FORMAT: &str = "creg-relayer-journal-export-v1";

#[derive(Deserialize, Serialize)]
struct DailyCounter {
    day: String,
    count: u64,
}

#[derive(Serialize)]
struct Export {
    format: &'static str,
    nonces: BTreeMap<String, u64>,
    quotas: BTreeMap<String, DailyCounter>,
    requests: Vec<serde_json::Value>,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(data_dir), Some(output)) = (args.next(), args.next()) else {
        bail!("usage: creg-relayer-sled-export <RELAYER_DATA_DIR> <output.json>");
    };
    let output = PathBuf::from(output);
    if output.exists() {
        bail!("{} already exists; refusing to overwrite", output.display());
    }

    let path = PathBuf::from(data_dir).join("relayer-state.db");
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    let db = sled::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let journal = db.open_tree("state")?;
    match journal.get(b"meta:schema")? {
        Some(version) if version.as_ref() == b"1" => {}
        Some(_) => bail!("unsupported relayer journal schema"),
        None => bail!("the database has no relayer journal schema marker"),
    }

    let mut export = Export {
        format: FORMAT,
        nonces: BTreeMap::new(),
        quotas: BTreeMap::new(),
        requests: Vec::new(),
    };
    for entry in journal.iter() {
        let (key, value) = entry?;
        let key = String::from_utf8(key.to_vec()).context("non-UTF-8 journal key")?;
        if let Some(nonce_key) = key.strip_prefix("nonce:") {
            let bytes: [u8; 8] = value
                .as_ref()
                .try_into()
                .with_context(|| format!("nonce {nonce_key} has an invalid encoding"))?;
            export
                .nonces
                .insert(nonce_key.to_string(), u64::from_be_bytes(bytes));
        } else if let Some(quota_key) = key.strip_prefix("quota:") {
            let counter: DailyCounter = serde_json::from_slice(&value)
                .with_context(|| format!("quota {quota_key} is unreadable"))?;
            export.quotas.insert(quota_key.to_string(), counter);
        } else if key.starts_with("request:") {
            export.requests.push(
                serde_json::from_slice(&value).with_context(|| format!("{key} is unreadable"))?,
            );
        }
    }

    let json = serde_json::to_vec_pretty(&export)?;
    std::fs::write(&output, json).with_context(|| format!("cannot write {}", output.display()))?;
    eprintln!(
        "exported {} nonces, {} quota windows and {} requests to {}",
        export.nonces.len(),
        export.quotas.len(),
        export.requests.len(),
        output.display()
    );
    Ok(())
}
