//! Consensus payload encoding selected by the chain's build profile.
//!
//! Phase C uses `creg-sorted-json-v1`: recursively sort object keys using Rust
//! string order, preserve array order, and use serde_json scalar encoding.
//! This is deliberately not a claim of RFC 8785 compatibility. Legacy builds
//! keep their original direct serde_json encoding and signature payloads.

use serde::Serialize;

pub const PHASE_C_CONSENSUS_ENCODING: &str = "creg-sorted-json-v1";

pub fn compiled_consensus_encoding() -> Option<&'static str> {
    if cfg!(feature = "phase-c-canonical-transactions") {
        Some(PHASE_C_CONSENSUS_ENCODING)
    } else {
        None
    }
}

pub fn consensus_json_bytes<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<Vec<u8>> {
    #[cfg(feature = "phase-c-canonical-transactions")]
    {
        let value = serde_json::to_value(value)?;
        let mut output = Vec::new();
        write_sorted_value(&mut output, &value)?;
        Ok(output)
    }
    #[cfg(not(feature = "phase-c-canonical-transactions"))]
    serde_json::to_vec(value)
}

#[cfg(feature = "phase-c-canonical-transactions")]
fn write_sorted_value(output: &mut Vec<u8>, value: &serde_json::Value) -> serde_json::Result<()> {
    match value {
        serde_json::Value::Object(map) => {
            output.push(b'{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                // serde_json escapes every control character, including
                // U+0010..U+001F; do not reuse the historical spec writer.
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                write_sorted_value(output, &map[key])?;
            }
            output.push(b'}');
        }
        serde_json::Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_sorted_value(output, item)?;
            }
            output.push(b']');
        }
        _ => serde_json::to_writer(output, value)?,
    }
    Ok(())
}
