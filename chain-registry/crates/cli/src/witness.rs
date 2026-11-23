//! Independent transparency-log witness. This is not a validator and does not
//! join PBFT. It polls a log operator's *signed* tree head, checks consistency
//! against the last accepted head, and writes a `WitnessedTreeHead` for clients
//! to fetch from this operator's own URL.
//!
//! Unsigned `GET /v1/public/transparency/.../head` bodies are monitoring data
//! and must not be signed. Running this process twice with two ids on one host
//! does not create independent witnesses.

use anyhow::{bail, Context, Result};
use common::crypto::SigningKey;
use common::{
    sign_witnessed_tree_head, verify_signed_tree_head, ConsistencyProof, SignedTreeHead, TreeHead,
    WitnessedTreeHead,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub const WITNESS_ID_ENV: &str = "CREG_WITNESS_ID";
#[allow(dead_code)]
pub const WITNESS_KEY_ENV: &str = "CREG_WITNESS_KEY";
pub const WITNESS_KEY_PASSPHRASE_ENV: &str = "CREG_WITNESS_KEY_PASSPHRASE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WitnessState {
    pub log_id: String,
    pub accepted_head: Option<TreeHead>,
    pub split_view: Option<SplitViewAlert>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitViewAlert {
    pub accepted: TreeHead,
    pub observed: TreeHead,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessTick {
    Signed(WitnessedTreeHead),
    Unchanged,
    SplitView(SplitViewAlert),
    Rollback(SplitViewAlert),
}

/// One observation of a log-operator signed head. `consistency` is required
/// when `previous` exists and the tree grew.
pub fn tick(
    signed: &SignedTreeHead,
    log_public_key: &str,
    witness_id: &str,
    witness_key: &SigningKey,
    previous: Option<&TreeHead>,
    consistency: Option<&ConsistencyProof>,
) -> Result<WitnessTick> {
    let witness_id = witness_id.trim();
    if witness_id.is_empty() {
        bail!("CREG_WITNESS_ID is required");
    }
    if witness_id == signed.log_signer_id {
        bail!(
            "witness_id '{witness_id}' must not equal the log signer id; the log operator cannot witness itself"
        );
    }
    verify_signed_tree_head(signed, log_public_key)
        .context("log tree-head signature is invalid")?;
    let observed = &signed.head;

    if let Some(previous) = previous {
        if previous.log_id != observed.log_id {
            bail!(
                "log_id mismatch: accepted {} vs observed {}",
                previous.log_id,
                observed.log_id
            );
        }
        if observed.tree_size < previous.tree_size {
            return Ok(WitnessTick::Rollback(SplitViewAlert {
                accepted: previous.clone(),
                observed: observed.clone(),
                reason: "observed tree_size is smaller than the accepted head (rollback)".into(),
            }));
        }
        if observed.tree_size == previous.tree_size {
            if observed.root_hash != previous.root_hash {
                return Ok(WitnessTick::SplitView(SplitViewAlert {
                    accepted: previous.clone(),
                    observed: observed.clone(),
                    reason: "two incompatible roots at the same tree size (split view)".into(),
                }));
            }
            if observed.issued_at == previous.issued_at {
                return Ok(WitnessTick::Unchanged);
            }
        }
        if observed.tree_size > previous.tree_size {
            let proof = consistency.ok_or_else(|| {
                anyhow::anyhow!("missing consistency proof from the accepted tree head")
            })?;
            if proof.log_id != observed.log_id
                || proof.first_tree_size != previous.tree_size
                || proof.first_root_hash != previous.root_hash
                || proof.second_tree_size != observed.tree_size
                || proof.second_root_hash != observed.root_hash
                || !proof.verify()?
            {
                return Ok(WitnessTick::SplitView(SplitViewAlert {
                    accepted: previous.clone(),
                    observed: observed.clone(),
                    reason: "new head is not consistent with the accepted head".into(),
                }));
            }
        }
    }

    Ok(WitnessTick::Signed(sign_witnessed_tree_head(
        signed.clone(),
        witness_id,
        witness_key,
    )))
}

pub fn load_state(path: &Path) -> Result<WitnessState> {
    if !path.exists() {
        return Ok(WitnessState {
            log_id: String::new(),
            accepted_head: None,
            split_view: None,
        });
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read witness state {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid witness state {}", path.display()))
}

pub fn save_state(path: &Path, state: &WitnessState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    // Windows cannot rename over an existing file.
    if path.exists() {
        std::fs::remove_file(path).with_context(|| format!("cannot replace {}", path.display()))?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("failed to persist {}", path.display()))?;
    Ok(())
}

pub fn write_witnessed_head(path: &Path, head: &WitnessedTreeHead) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(head)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn load_witness_signing_key(path: &Path) -> Result<SigningKey> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read witness key {}", path.display()))?;
    let hex_secret = if content.starts_with("CREG-ENC-V1") {
        let passphrase = std::env::var(WITNESS_KEY_PASSPHRASE_ENV).map_err(|_| {
            anyhow::anyhow!(
                "witness key is encrypted; set {WITNESS_KEY_PASSPHRASE_ENV} for non-interactive run"
            )
        })?;
        crate::keygen::decrypt_key_file(path, &passphrase)?
    } else {
        content.trim().to_string()
    };
    let secret = hex::decode(hex_secret.trim().trim_start_matches("0x"))
        .context("witness key is not valid hex")?;
    let bytes: [u8; 32] = secret
        .try_into()
        .map_err(|_| anyhow::anyhow!("witness key must be 32 bytes"))?;
    Ok(SigningKey::from_ed25519_seed(&bytes))
}

pub fn default_witness_key_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".creg")
        .join("witness.key")
}

pub fn default_state_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".creg")
        .join("witness-state.json")
}

pub fn default_out_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".creg")
        .join("witnessed-head.json")
}

pub struct WitnessRunArgs {
    pub node_url: String,
    pub log_id: String,
    pub log_public_key: String,
    pub witness_id: String,
    pub key_path: PathBuf,
    pub state_path: PathBuf,
    pub out_path: PathBuf,
    pub once: bool,
    pub interval_secs: u64,
}

pub async fn run(args: WitnessRunArgs) -> Result<()> {
    let key = load_witness_signing_key(&args.key_path)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let base = args.node_url.trim_end_matches('/');
    let encoded_log = urlencoding::encode(&args.log_id);
    let signed_url = format!("{base}/v1/public/transparency/{encoded_log}/signed-head");

    loop {
        let mut state = load_state(&args.state_path)?;
        if let Some(alert) = &state.split_view {
            bail!(
                "witness is fail-closed after split-view/rollback (accepted tree_size={}, observed tree_size={}). Inspect {} and remove split_view only after an operator review. Reason: {}",
                alert.accepted.tree_size,
                alert.observed.tree_size,
                args.state_path.display(),
                alert.reason
            );
        }
        if !state.log_id.is_empty() && state.log_id != args.log_id {
            bail!(
                "state log_id {} does not match --log-id {}",
                state.log_id,
                args.log_id
            );
        }

        let response = client
            .get(&signed_url)
            .send()
            .await
            .with_context(|| format!("failed to fetch signed tree head from {signed_url}"))?;
        if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE
            || response.status() == reqwest::StatusCode::NOT_FOUND
        {
            bail!(
                "log at {signed_url} has not enabled signed tree heads (HTTP {}). Unsigned GET /head must not be witnessed. The log operator must set CREG_TRANSPARENCY_LOG_KEY.",
                response.status()
            );
        }
        if !response.status().is_success() {
            bail!("{signed_url} returned HTTP {}", response.status());
        }
        let signed: SignedTreeHead = response
            .json()
            .await
            .context("signed-head body is not a SignedTreeHead")?;
        if signed.head.log_id != args.log_id {
            bail!(
                "signed head log_id {} does not match {}",
                signed.head.log_id,
                args.log_id
            );
        }

        let mut consistency = None;
        if let Some(previous) = &state.accepted_head {
            if signed.head.tree_size > previous.tree_size {
                let url = format!(
                    "{base}/v1/public/transparency/{encoded_log}/consistency/{}",
                    previous.tree_size
                );
                let proof: ConsistencyProof = client
                    .get(&url)
                    .send()
                    .await
                    .with_context(|| format!("failed to fetch {url}"))?
                    .error_for_status()
                    .with_context(|| format!("{url} returned an error"))?
                    .json()
                    .await
                    .context("consistency body is invalid")?;
                consistency = Some(proof);
            }
        }

        match tick(
            &signed,
            &args.log_public_key,
            &args.witness_id,
            &key,
            state.accepted_head.as_ref(),
            consistency.as_ref(),
        )? {
            WitnessTick::Signed(head) => {
                write_witnessed_head(&args.out_path, &head)?;
                state.log_id = args.log_id.clone();
                state.accepted_head = Some(head.signed_head.head.clone());
                save_state(&args.state_path, &state)?;
                tracing::info!(
                    tree_size = head.signed_head.head.tree_size,
                    out = %args.out_path.display(),
                    "wrote witnessed tree head"
                );
            }
            WitnessTick::Unchanged => {
                tracing::debug!("signed tree head unchanged");
            }
            WitnessTick::SplitView(alert) | WitnessTick::Rollback(alert) => {
                state.log_id = args.log_id.clone();
                state.split_view = Some(alert.clone());
                save_state(&args.state_path, &state)?;
                bail!(
                    "{}: accepted root {} size {} vs observed root {} size {}. Witness will not sign. State: {}",
                    alert.reason,
                    alert.accepted.root_hash,
                    alert.accepted.tree_size,
                    alert.observed.root_hash,
                    alert.observed.tree_size,
                    args.state_path.display()
                );
            }
        }

        if args.once {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(args.interval_secs.max(1))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{
        consistency_proof, leaf_hash, sign_tree_head, tree_root, TRANSPARENCY_LOG_VERSION,
    };

    fn head(log_id: &str, size: u64, root: &str, issued: &str) -> TreeHead {
        TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: log_id.into(),
            tree_size: size,
            root_hash: root.into(),
            issued_at: issued.into(),
        }
    }

    fn keys() -> (SigningKey, SigningKey) {
        (
            SigningKey::from_ed25519_seed(&[11u8; 32]),
            SigningKey::from_ed25519_seed(&[12u8; 32]),
        )
    }

    #[test]
    fn first_signed_head_is_witnessed() {
        let (log, witness) = keys();
        let tree = head("log-1", 1, &leaf_hash(b"a"), "2026-08-24T00:00:00Z");
        let signed = sign_tree_head(tree, "log-operator", &log);
        let tick = tick(
            &signed,
            &log.public_key().to_hex(),
            "org-a",
            &witness,
            None,
            None,
        )
        .unwrap();
        match tick {
            WitnessTick::Signed(head) => {
                assert_eq!(head.witness_signatures.len(), 1);
                assert_eq!(head.witness_signatures[0].witness_id, "org-a");
            }
            other => panic!("expected signed, got {other:?}"),
        }
    }

    #[test]
    fn log_operator_cannot_witness_self() {
        let (log, _) = keys();
        let tree = head("log-1", 1, &leaf_hash(b"a"), "2026-08-24T00:00:00Z");
        let signed = sign_tree_head(tree, "log-operator", &log);
        let err = tick(
            &signed,
            &log.public_key().to_hex(),
            "log-operator",
            &log,
            None,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("must not equal the log signer"));
    }

    #[test]
    fn split_view_same_size_different_root() {
        let (log, witness) = keys();
        let accepted = head("log-1", 1, &leaf_hash(b"a"), "2026-08-24T00:00:00Z");
        let observed = head("log-1", 1, &leaf_hash(b"b"), "2026-08-24T00:00:01Z");
        let signed = sign_tree_head(observed, "log-operator", &log);
        let tick = tick(
            &signed,
            &log.public_key().to_hex(),
            "org-a",
            &witness,
            Some(&accepted),
            None,
        )
        .unwrap();
        match tick {
            WitnessTick::SplitView(alert) => {
                assert!(alert.reason.contains("split view"));
            }
            other => panic!("expected split view, got {other:?}"),
        }
    }

    #[test]
    fn rollback_is_refused() {
        let (log, witness) = keys();
        let accepted = head("log-1", 2, &leaf_hash(b"ab"), "2026-08-24T00:00:00Z");
        let observed = head("log-1", 1, &leaf_hash(b"a"), "2026-08-24T00:00:01Z");
        let signed = sign_tree_head(observed, "log-operator", &log);
        let tick = tick(
            &signed,
            &log.public_key().to_hex(),
            "org-a",
            &witness,
            Some(&accepted),
            None,
        )
        .unwrap();
        assert!(matches!(tick, WitnessTick::Rollback(_)));
    }

    #[test]
    fn growth_with_valid_consistency_is_signed() {
        let (log, witness) = keys();
        let leaf_a = leaf_hash(b"a");
        let leaf_b = leaf_hash(b"b");
        let leaves = vec![leaf_a.clone(), leaf_b.clone()];
        let first = head("log-1", 1, &leaf_a, "2026-08-24T00:00:00Z");
        let second = head(
            "log-1",
            2,
            &tree_root(&leaves).unwrap(),
            "2026-08-24T00:00:01Z",
        );
        let proof = consistency_proof("log-1", &leaves, 1).unwrap();
        let signed = sign_tree_head(second, "log-operator", &log);
        let tick = tick(
            &signed,
            &log.public_key().to_hex(),
            "org-a",
            &witness,
            Some(&first),
            Some(&proof),
        )
        .unwrap();
        assert!(matches!(tick, WitnessTick::Signed(_)));
    }

    #[test]
    fn persist_split_view_fail_closes_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let accepted = head("log-1", 1, &leaf_hash(b"a"), "2026-08-24T00:00:00Z");
        let observed = head("log-1", 1, &leaf_hash(b"b"), "2026-08-24T00:00:01Z");
        let state = WitnessState {
            log_id: "log-1".into(),
            accepted_head: Some(accepted.clone()),
            split_view: Some(SplitViewAlert {
                accepted,
                observed,
                reason: "split".into(),
            }),
        };
        save_state(&path, &state).unwrap();
        save_state(&path, &state).unwrap();
        let loaded = load_state(&path).unwrap();
        assert!(loaded.split_view.is_some());
    }

    #[test]
    fn invalid_consistency_is_split_view() {
        let (log, witness) = keys();
        let leaf_a = leaf_hash(b"a");
        let leaf_b = leaf_hash(b"b");
        let leaves = vec![leaf_a.clone(), leaf_b.clone()];
        let first = head("log-1", 1, &leaf_a, "2026-08-24T00:00:00Z");
        let second = head(
            "log-1",
            2,
            &tree_root(&leaves).unwrap(),
            "2026-08-24T00:00:01Z",
        );
        let mut proof = consistency_proof("log-1", &leaves, 1).unwrap();
        proof.second_root_hash = leaf_hash(b"zzz");
        let signed = sign_tree_head(second, "log-operator", &log);
        let tick = tick(
            &signed,
            &log.public_key().to_hex(),
            "org-a",
            &witness,
            Some(&first),
            Some(&proof),
        )
        .unwrap();
        match tick {
            WitnessTick::SplitView(alert) => {
                assert!(alert.reason.contains("not consistent"));
            }
            other => panic!("expected split view, got {other:?}"),
        }
    }
}
