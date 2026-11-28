//! ADR-017 decision 4: every ML-DSA-44 signature is sent once per gossip
//! message.
//!
//! From hybrid activation a PBFT view-change certificate repeats the same
//! PREPARE signatures inside every vote's prepared certificate, and each
//! ML-DSA-44 component is 4,840 hex characters (4,968 for a single-field
//! hybrid signature). A full certificate at the spec's 50-validator maximum
//! would exceed the 4 MiB limit for block and view-change gossip.
//!
//! Before such a message is published, each distinct signature string is
//! moved into a table carried once in the envelope and every occurrence is
//! replaced by a reference. Receivers restore the exact message before
//! parsing it, so consensus code and every signature check see the original
//! value. Messages that carry no ML-DSA-44 material, which is every message
//! before activation, are sent unchanged and parse as before.

use anyhow::{bail, ensure, Context, Result};
use common::vote_v4::{HYBRID_SIGNATURE_HEX_LEN, MLDSA44_SIGNATURE_HEX_LEN};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

const ENVELOPE_KEY: &str = "creg_pq_compact_v1";
/// Never a prefix of lowercase hex, so a reference cannot be confused with a
/// pooled value.
const REFERENCE_PREFIX: &str = "#pq:";
/// Upper bound on a restored message. A compact message is at most 4 MiB on
/// the wire; this caps how far a hostile one can expand.
pub const MAX_EXPANDED_BYTES: usize = 32 * 1024 * 1024;
/// A 4 MiB envelope cannot hold more distinct signatures than this.
const MAX_POOL_ENTRIES: usize = 1_024;

/// Topics whose messages can carry repeated view-change certificates.
pub fn topic_uses_compaction(topic: &str) -> bool {
    topic.contains("blocks") || topic.contains("view-change")
}

fn is_signature_material(value: &str) -> bool {
    (value.len() == MLDSA44_SIGNATURE_HEX_LEN || value.len() == HYBRID_SIGNATURE_HEX_LEN)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn pool_signatures(
    value: &mut Value,
    pool: &mut Vec<String>,
    index: &mut HashMap<String, usize>,
    unsafe_to_compact: &mut bool,
) {
    match value {
        Value::String(text) => {
            if text.starts_with(REFERENCE_PREFIX) {
                // A literal string that looks like a reference would be
                // rewritten on restore; send this message unchanged instead.
                *unsafe_to_compact = true;
            } else if is_signature_material(text) {
                let position = match index.get(text.as_str()) {
                    Some(&position) => position,
                    None => {
                        let position = pool.len();
                        pool.push(text.clone());
                        index.insert(text.clone(), position);
                        position
                    }
                };
                *text = format!("{REFERENCE_PREFIX}{position}");
            }
        }
        Value::Array(items) => {
            for item in items {
                pool_signatures(item, pool, index, unsafe_to_compact);
            }
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                pool_signatures(item, pool, index, unsafe_to_compact);
            }
        }
        _ => {}
    }
}

/// Compact a JSON gossip payload. Returns the original bytes when it holds no
/// ML-DSA-44 material or compaction would not make it smaller.
pub fn compact(data: Vec<u8>) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(&data) else {
        return data;
    };
    let mut pool = Vec::new();
    let mut index = HashMap::new();
    let mut unsafe_to_compact = false;
    pool_signatures(&mut value, &mut pool, &mut index, &mut unsafe_to_compact);
    if unsafe_to_compact || pool.is_empty() || pool.len() > MAX_POOL_ENTRIES {
        return data;
    }
    let mut body = serde_json::Map::new();
    body.insert(
        "pool".to_owned(),
        Value::Array(pool.into_iter().map(Value::String).collect()),
    );
    body.insert("message".to_owned(), value);
    let mut envelope = serde_json::Map::new();
    envelope.insert(ENVELOPE_KEY.to_owned(), Value::Object(body));
    match serde_json::to_vec(&Value::Object(envelope)) {
        Ok(bytes) if bytes.len() < data.len() => bytes,
        _ => data,
    }
}

/// [`compact`] for messages that can benefit: only payloads carrying a
/// separate `pq_signature` field hold PREPARE signatures that repeat across a
/// certificate. Every other message, including all classical traffic, is
/// returned without being parsed.
pub fn compact_if_hybrid(data: Vec<u8>) -> Vec<u8> {
    let carries_pq_signature = std::str::from_utf8(&data)
        .map(|text| text.contains("\"pq_signature\":\""))
        .unwrap_or(false);
    if carries_pq_signature {
        compact(data)
    } else {
        data
    }
}

/// Whether `data` is a compact envelope. Ordinary gossip messages are
/// externally tagged enums or plain objects and never use this key.
pub fn is_compact(data: &[u8]) -> bool {
    let trimmed = data.trim_ascii_start();
    trimmed.starts_with(b"{\"") && trimmed[2..].starts_with(ENVELOPE_KEY.as_bytes())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    creg_pq_compact_v1: Body,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    pool: Vec<String>,
    message: Value,
}

fn restore_signatures(
    value: &mut Value,
    pool: &[String],
    used: &mut [bool],
    budget: &mut usize,
) -> Result<()> {
    match value {
        Value::String(text) => {
            if let Some(position) = text.strip_prefix(REFERENCE_PREFIX) {
                let position: usize = position
                    .parse()
                    .context("compact gossip reference is not an index")?;
                if text.len() != REFERENCE_PREFIX.len() + position.to_string().len() {
                    bail!("compact gossip reference is not canonical");
                }
                let entry = pool
                    .get(position)
                    .context("compact gossip reference is out of range")?;
                *budget = budget
                    .checked_sub(entry.len())
                    .context("compact gossip message expands past its size limit")?;
                used[position] = true;
                *text = entry.clone();
            }
        }
        Value::Array(items) => {
            for item in items {
                restore_signatures(item, pool, used, budget)?;
            }
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                restore_signatures(item, pool, used, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Restore the original JSON payload from a compact envelope. The envelope
/// must be canonical: every pool entry is signature-shaped, distinct and
/// referenced, and every reference is in range.
pub fn expand(data: &[u8]) -> Result<Vec<u8>> {
    let Envelope {
        creg_pq_compact_v1: Body { pool, mut message },
    } = serde_json::from_slice(data).context("malformed compact gossip envelope")?;
    ensure!(
        !pool.is_empty() && pool.len() <= MAX_POOL_ENTRIES,
        "compact gossip pool size is out of range"
    );
    let mut distinct = std::collections::HashSet::with_capacity(pool.len());
    for entry in &pool {
        ensure!(
            is_signature_material(entry),
            "compact gossip pool holds a non-signature value"
        );
        ensure!(
            distinct.insert(entry.as_str()),
            "compact gossip pool repeats a value"
        );
    }
    let mut used = vec![false; pool.len()];
    let mut budget = MAX_EXPANDED_BYTES;
    restore_signatures(&mut message, &pool, &mut used, &mut budget)?;
    ensure!(
        used.iter().all(|used| *used),
        "compact gossip pool holds an unreferenced value"
    );
    let restored = serde_json::to_vec(&message)?;
    ensure!(
        restored.len() <= MAX_EXPANDED_BYTES,
        "compact gossip message expands past its size limit"
    );
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{
        Block, BlockSignature, GossipMessage, PreparedBlockHeader, PreparedCertificate,
        ViewChangeVote,
    };

    fn hex_of_len(seed: &str, len: usize) -> String {
        let mut out = String::with_capacity(len + 64);
        let mut counter = 0u64;
        while out.len() < len {
            out.push_str(&common::sha256_hex(format!("{seed}:{counter}").as_bytes()));
            counter += 1;
        }
        out.truncate(len);
        out
    }

    /// A re-proposal carrying a hybrid view-change certificate from `n`
    /// validators: `q` votes, each with a prepared certificate holding the
    /// same `q` PREPARE signatures.
    fn hybrid_reproposal(n: usize) -> GossipMessage {
        let q = 2 * n / 3 + 1;
        let block = Block::genesis();
        let prepare_signatures: Vec<BlockSignature> = (0..q)
            .map(|i| BlockSignature {
                validator_id: format!("validator-{i}"),
                pubkey: hex_of_len(&format!("pk{i}"), 64),
                signature: hex_of_len(&format!("ed{i}"), 128),
                pq_signature: Some(hex_of_len(&format!("pq{i}"), MLDSA44_SIGNATURE_HEX_LEN)),
            })
            .collect();
        let prepared = PreparedCertificate {
            header: PreparedBlockHeader::from(&block.header),
            prepared_view: 0,
            original_view_change_certificate: Vec::new(),
            reproposal_proof: None,
            transactions: Vec::new(),
            prepare_signatures,
        };
        let certificate = (0..q)
            .map(|i| ViewChangeVote {
                height: 1,
                prev_hash: "11".repeat(32),
                new_view: 1,
                validator_id: format!("validator-{i}"),
                prepared: Some(prepared.clone()),
                signature: hex_of_len(&format!("vote{i}"), HYBRID_SIGNATURE_HEX_LEN),
            })
            .collect();
        GossipMessage::PbftRepropose {
            block,
            target_view: 1,
            leader_id: "validator-1".into(),
            view_change_certificate: certificate,
            leader_signature: hex_of_len("leader", HYBRID_SIGNATURE_HEX_LEN),
        }
    }

    #[test]
    fn a_fifty_validator_hybrid_certificate_fits_the_gossip_limit_and_restores_exactly() {
        let message = hybrid_reproposal(50);
        let original = serde_json::to_vec(&message).unwrap();
        assert!(
            original.len() > 4 * 1024 * 1024,
            "premise: the uncompacted certificate exceeds 4 MiB ({} bytes)",
            original.len()
        );
        let compacted = compact_if_hybrid(original.clone());
        assert!(is_compact(&compacted));
        assert!(
            compacted.len() < 1024 * 1024,
            "compacted certificate is {} bytes",
            compacted.len()
        );
        let restored = expand(&compacted).unwrap();
        let restored_value: Value = serde_json::from_slice(&restored).unwrap();
        let original_value: Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(restored_value, original_value);
        let parsed: GossipMessage = serde_json::from_slice(&restored).unwrap();
        assert!(matches!(parsed, GossipMessage::PbftRepropose { .. }));
    }

    #[test]
    fn classical_messages_are_sent_unchanged() {
        let message = GossipMessage::PbftPrepare {
            block_hash: "ab".repeat(32),
            validator_id: "validator-1".into(),
            signature: hex_of_len("ed", 128),
            pq_signature: None,
        };
        let original = serde_json::to_vec(&message).unwrap();
        assert_eq!(compact(original.clone()), original);
        assert_eq!(compact_if_hybrid(original.clone()), original);
        assert!(!is_compact(&original));
        // Not JSON at all: unchanged.
        assert_eq!(compact(b"not json".to_vec()), b"not json".to_vec());
    }

    #[test]
    fn a_literal_reference_string_disables_compaction() {
        let message = serde_json::json!({
            "note": "#pq:0",
            "a": hex_of_len("x", MLDSA44_SIGNATURE_HEX_LEN),
            "b": hex_of_len("x", MLDSA44_SIGNATURE_HEX_LEN),
        });
        let original = serde_json::to_vec(&message).unwrap();
        assert_eq!(compact(original.clone()), original);
    }

    #[test]
    fn malformed_envelopes_are_refused() {
        let signature = hex_of_len("x", MLDSA44_SIGNATURE_HEX_LEN);
        let other = hex_of_len("y", MLDSA44_SIGNATURE_HEX_LEN);
        let envelope = |pool: Vec<String>, message: Value| {
            serde_json::to_vec(&serde_json::json!({
                "creg_pq_compact_v1": { "pool": pool, "message": message }
            }))
            .unwrap()
        };
        // A valid one first.
        assert!(expand(&envelope(
            vec![signature.clone()],
            serde_json::json!({ "s": "#pq:0", "t": "#pq:0" })
        ))
        .is_ok());
        for (pool, message) in [
            // Out of range, non-canonical and non-numeric references.
            (vec![signature.clone()], serde_json::json!({ "s": "#pq:1" })),
            (
                vec![signature.clone()],
                serde_json::json!({ "s": "#pq:00" }),
            ),
            (vec![signature.clone()], serde_json::json!({ "s": "#pq:x" })),
            // Unreferenced, repeated and non-signature pool entries.
            (
                vec![signature.clone(), other.clone()],
                serde_json::json!({ "s": "#pq:0" }),
            ),
            (
                vec![signature.clone(), signature.clone()],
                serde_json::json!({ "s": "#pq:0", "t": "#pq:1" }),
            ),
            (vec!["abcd".into()], serde_json::json!({ "s": "#pq:0" })),
            (Vec::new(), serde_json::json!({})),
        ] {
            assert!(expand(&envelope(pool, message)).is_err());
        }
        // Unknown envelope fields.
        let extra = serde_json::to_vec(&serde_json::json!({
            "creg_pq_compact_v1": { "pool": [signature], "message": { "s": "#pq:0" }, "x": 1 }
        }))
        .unwrap();
        assert!(expand(&extra).is_err());
    }

    #[test]
    fn expansion_is_bounded() {
        let signature = hex_of_len("x", HYBRID_SIGNATURE_HEX_LEN);
        let references = MAX_EXPANDED_BYTES / HYBRID_SIGNATURE_HEX_LEN + 1;
        let message = Value::Array(vec![Value::String("#pq:0".into()); references]);
        let bytes = serde_json::to_vec(&serde_json::json!({
            "creg_pq_compact_v1": { "pool": [signature], "message": message }
        }))
        .unwrap();
        assert!(bytes.len() < 4 * 1024 * 1024);
        let error = expand(&bytes).unwrap_err();
        assert!(error.to_string().contains("size limit"), "{error}");
    }
}
