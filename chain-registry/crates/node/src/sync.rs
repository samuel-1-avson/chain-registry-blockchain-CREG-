// crates/node/src/sync.rs
// Chain synchronisation — brings a lagging node up to the network tip.
//
// On startup, and periodically during operation, this module:
//   1. Asks peers for their chain tip height.
//   2. If we are behind, fetches each missing block in order.
//   3. Validates each block's prev_hash linkage before inserting.
//   4. Applies each block to the publisher index.
//
// This is a simple linear sync. In a production network with thousands of
// blocks, a state-snapshot sync (download a snapshot + apply delta) would
// be more efficient, but for a registry where each block contains a handful
// of package verification transactions, linear sync is perfectly adequate.

use crate::NodeState;
use common::{AuthenticatedValidatorSetSnapshot, Transaction, ValidatorSet, ValidatorVote};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

/// Sync interval — check for new blocks from peers every 10 seconds.
const SYNC_INTERVAL_SECS: u64 = 10;

/// Log peer heights when local tip has not advanced for this many sync polls.
const STALL_LOG_AFTER_POLLS: u32 = 3;

/// Maximum reorg depth this node will automatically recover from. Forks
/// deeper than this require operator intervention (they indicate either a
/// long partition or an attack and should not be auto-adopted).
const MAX_REORG_DEPTH: u64 = 64;

/// Bound untrusted history responses before deserializing them. A checkpointed
/// deployment retains the complete certificate chain, so this is intentionally
/// much larger than the compatibility retention window while still preventing
/// an HTTP peer from forcing unbounded allocation.
const MAX_TRANSITION_HISTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRANSITION_HISTORY_ENTRIES: usize = 16_384;

#[derive(Clone, Debug)]
struct PeerTransitionHistory {
    source: String,
    advertised_tip: u64,
    transitions: Vec<AuthenticatedValidatorSetSnapshot>,
}

struct SyncStallTracker {
    last_height: u64,
    stall_polls: u32,
}

static SYNC_STALL: Mutex<SyncStallTracker> = Mutex::new(SyncStallTracker {
    last_height: 0,
    stall_polls: 0,
});

/// Compute lag between a local tip and the highest peer tip.
pub fn sync_lag_blocks(local_tip: u64, max_peer_tip: u64) -> u64 {
    max_peer_tip.saturating_sub(local_tip)
}

async fn query_peer_heights(
    client: &reqwest::Client,
    peer_urls: &[String],
) -> (u64, HashMap<String, u64>) {
    #[derive(serde::Deserialize)]
    struct Stats {
        tip_height: u64,
    }

    let mut max_tip = 0u64;
    let mut per_peer = HashMap::new();
    for url in peer_urls {
        let full_url = format!("{}/v1/chain/stats", url.trim_end_matches('/'));
        let height = match client.get(&full_url).send().await {
            Ok(resp) if resp.status().is_success() => resp
                .json::<Stats>()
                .await
                .map(|stats| stats.tip_height)
                .unwrap_or(0),
            _ => 0,
        };
        per_peer.insert(url.clone(), height);
        max_tip = max_tip.max(height);
    }
    (max_tip, per_peer)
}

async fn fetch_peer_transition_histories(
    client: &reqwest::Client,
    peer_urls: &[String],
    peer_heights: &HashMap<String, u64>,
) -> Vec<PeerTransitionHistory> {
    let mut histories = Vec::new();
    for url in peer_urls {
        let full_url = format!("{}/v1/validator/set-transitions", url.trim_end_matches('/'));
        let response = match client.get(&full_url).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                tracing::debug!(
                    peer = %url,
                    status = %response.status(),
                    "Peer did not serve validator-set transition history"
                );
                continue;
            }
            Err(error) => {
                tracing::debug!(
                    peer = %url,
                    error = %error,
                    "Could not fetch validator-set transition history"
                );
                continue;
            }
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_TRANSITION_HISTORY_BYTES as u64)
        {
            tracing::warn!(peer = %url, "Rejected oversized validator-set history response");
            continue;
        }
        let bytes = match response.bytes().await {
            Ok(bytes) if bytes.len() <= MAX_TRANSITION_HISTORY_BYTES => bytes,
            Ok(_) => {
                tracing::warn!(peer = %url, "Rejected oversized validator-set history response");
                continue;
            }
            Err(error) => {
                tracing::debug!(peer = %url, error = %error, "Could not read validator-set history response");
                continue;
            }
        };
        let transitions: Vec<AuthenticatedValidatorSetSnapshot> = match serde_json::from_slice(
            &bytes,
        ) {
            Ok(transitions) => transitions,
            Err(error) => {
                tracing::warn!(peer = %url, error = %error, "Rejected malformed validator-set history response");
                continue;
            }
        };
        if transitions.len() > MAX_TRANSITION_HISTORY_ENTRIES {
            tracing::warn!(peer = %url, "Rejected validator-set history with too many entries");
            continue;
        }
        histories.push(PeerTransitionHistory {
            source: url.clone(),
            advertised_tip: peer_heights.get(url).copied().unwrap_or_default(),
            transitions,
        });
    }
    histories
}

/// Validate an untrusted peer history without mutating local authority state.
/// Every certificate is checked sequentially from the configured bootstrap
/// validator set. Anchors at or below `canonical_tip` must name the exact local
/// canonical block, while later anchors are checked immediately before their
/// certificate becomes eligible for persistence.
fn validate_peer_transition_history(
    history: &PeerTransitionHistory,
    bootstrap: &ValidatorSet,
    checkpoint: Option<&AuthenticatedValidatorSetSnapshot>,
    canonical_tip: u64,
    canonical_anchors: &HashMap<u64, String>,
) -> Result<(), String> {
    if history.transitions.len() > MAX_TRANSITION_HISTORY_ENTRIES {
        return Err("validator-set transition history exceeds the entry limit".into());
    }

    let checkpoint_hash = checkpoint.map(common::validator_snapshot_certificate_hash);
    let mut checkpoint_found = checkpoint_hash.is_none();
    let mut previous_height = None;
    let mut authority = bootstrap.clone();

    for transition in &history.transitions {
        if previous_height.is_some_and(|height| transition.effective_from_height <= height) {
            return Err(
                "validator-set transition history is unordered or contains duplicate heights"
                    .into(),
            );
        }
        if transition.effective_from_height > history.advertised_tip.saturating_add(1) {
            return Err(format!(
                "transition at height {} is ahead of the peer's advertised tip {}",
                transition.effective_from_height, history.advertised_tip
            ));
        }
        transition
            .validate_against(&authority)
            .map_err(|error| format!("invalid transition certificate: {error}"))?;

        let anchor_height = transition.effective_from_height.saturating_sub(1);
        if anchor_height <= canonical_tip {
            let canonical_hash = canonical_anchors
                .get(&anchor_height)
                .ok_or_else(|| format!("local canonical block {anchor_height} is unavailable"))?;
            if transition.previous_block_hash.as_str() != canonical_hash.as_str() {
                return Err(format!(
                    "transition at height {} anchors a conflicting fork",
                    transition.effective_from_height
                ));
            }
        }

        let transition_certificate_hash = common::validator_snapshot_certificate_hash(transition);
        if checkpoint_hash.as_deref() == Some(transition_certificate_hash.as_str()) {
            checkpoint_found = true;
        }
        previous_height = Some(transition.effective_from_height);
        authority = transition.validators.clone();
    }

    if !checkpoint_found {
        return Err(
            "peer history does not contain the configured weak-subjectivity checkpoint".into(),
        );
    }
    Ok(())
}

/// Histories may differ only by a suffix. A different certificate or a
/// missing intermediate transition before both histories' tips is a
/// conflicting authority timeline and is rejected rather than peer-voted.
fn ensure_histories_are_prefix_compatible(
    left: &[AuthenticatedValidatorSetSnapshot],
    right: &[AuthenticatedValidatorSetSnapshot],
) -> Result<(), String> {
    let left_tip = left
        .last()
        .map(|transition| transition.effective_from_height)
        .unwrap_or_default();
    let right_tip = right
        .last()
        .map(|transition| transition.effective_from_height)
        .unwrap_or_default();
    let shared_tip = left_tip.min(right_tip);
    let left_prefix: HashMap<u64, String> = left
        .iter()
        .filter(|transition| transition.effective_from_height <= shared_tip)
        .map(|transition| {
            (
                transition.effective_from_height,
                common::validator_snapshot_certificate_hash(transition),
            )
        })
        .collect();
    let right_prefix: HashMap<u64, String> = right
        .iter()
        .filter(|transition| transition.effective_from_height <= shared_tip)
        .map(|transition| {
            (
                transition.effective_from_height,
                common::validator_snapshot_certificate_hash(transition),
            )
        })
        .collect();
    if left_prefix != right_prefix {
        return Err("conflicting validator-set transition histories".into());
    }
    Ok(())
}

fn transition_history_set_at(
    transitions: &[AuthenticatedValidatorSetSnapshot],
    bootstrap: &ValidatorSet,
    height: u64,
) -> ValidatorSet {
    transitions
        .iter()
        .filter(|transition| transition.effective_from_height <= height)
        .max_by_key(|transition| transition.effective_from_height)
        .map(|transition| transition.validators.clone())
        .unwrap_or_else(|| bootstrap.clone())
}

fn record_sync_stall(local_tip: u64, peer_heights: &HashMap<String, u64>) {
    let mut tracker = SYNC_STALL.lock().expect("sync stall mutex poisoned");
    if local_tip > tracker.last_height {
        tracker.last_height = local_tip;
        tracker.stall_polls = 0;
        return;
    }
    tracker.stall_polls = tracker.stall_polls.saturating_add(1);
    if tracker.stall_polls >= STALL_LOG_AFTER_POLLS {
        let peer_summary: Vec<String> = peer_heights
            .iter()
            .map(|(url, height)| format!("{url}={height}"))
            .collect();
        tracing::warn!(
            local_tip,
            stall_polls = tracker.stall_polls,
            peers = %peer_summary.join(", "),
            "Chain sync stalled — no local progress while peers report heights"
        );
    }
}

async fn select_transition_history_for_sync(
    state: &Arc<RwLock<NodeState>>,
    client: &reqwest::Client,
    peer_urls: &[String],
    peer_heights: &HashMap<String, u64>,
) -> anyhow::Result<Option<Vec<AuthenticatedValidatorSetSnapshot>>> {
    let mut candidates = fetch_peer_transition_histories(client, peer_urls, peer_heights).await;
    let (bootstrap, data_dir) = {
        let state = state.read().await;
        (
            state.validator_set_bootstrap.clone(),
            state.config.data_dir.clone(),
        )
    };
    let checkpoint =
        crate::validator_set_history::checkpoint_from_env().map_err(anyhow::Error::msg)?;
    let local_transitions: Vec<AuthenticatedValidatorSetSnapshot> =
        crate::validator_set_history::load(&data_dir)
            .into_iter()
            .filter_map(|entry| entry.certificate)
            .collect();

    let canonical_tip = state.read().await.chain.tip_height()?;
    let required_anchor_heights: HashSet<u64> = candidates
        .iter()
        .flat_map(|candidate| candidate.transitions.iter())
        .chain(local_transitions.iter())
        .map(|transition| transition.effective_from_height.saturating_sub(1))
        .filter(|height| *height <= canonical_tip)
        .collect();
    let canonical_anchors = {
        let state = state.read().await;
        let mut anchors = HashMap::new();
        for height in required_anchor_heights {
            if let Some(block) = state.chain.get_block_by_height(height)? {
                anchors.insert(height, block.hash());
            }
        }
        anchors
    };

    let mut valid = Vec::new();
    for candidate in candidates {
        match validate_peer_transition_history(
            &candidate,
            &bootstrap,
            checkpoint.as_ref(),
            canonical_tip,
            &canonical_anchors,
        ) {
            Ok(()) => valid.push(candidate),
            Err(error) => tracing::warn!(
                source = %candidate.source,
                error = %error,
                "Rejected validator-set transition history"
            ),
        }
    }

    if valid.is_empty() && !local_transitions.is_empty() {
        let local = PeerTransitionHistory {
            source: "local-persisted-history".into(),
            advertised_tip: canonical_tip,
            transitions: local_transitions.clone(),
        };
        validate_peer_transition_history(
            &local,
            &bootstrap,
            checkpoint.as_ref(),
            canonical_tip,
            &canonical_anchors,
        )
        .map_err(anyhow::Error::msg)?;
        return Ok(Some(local_transitions));
    }

    if valid.is_empty() {
        if checkpoint.is_some() {
            anyhow::bail!(
                "no peer supplied a validator-set history containing the configured weak-subjectivity checkpoint"
            );
        }
        return Ok(None);
    }

    for left_index in 0..valid.len() {
        for right_index in (left_index + 1)..valid.len() {
            ensure_histories_are_prefix_compatible(
                &valid[left_index].transitions,
                &valid[right_index].transitions,
            )
            .map_err(|error| {
                anyhow::anyhow!(
                    "{error}: {} conflicts with {}",
                    valid[left_index].source,
                    valid[right_index].source
                )
            })?;
        }
    }

    let selected = valid
        .into_iter()
        .max_by_key(|candidate| {
            (
                candidate
                    .transitions
                    .last()
                    .map(|transition| transition.effective_from_height)
                    .unwrap_or_default(),
                candidate.transitions.len(),
            )
        })
        .expect("non-empty validated candidate list");
    if selected.transitions.is_empty() {
        Ok(None)
    } else {
        tracing::info!(
            source = %selected.source,
            transitions = selected.transitions.len(),
            "Selected independently validated validator-set transition history"
        );
        Ok(Some(selected.transitions))
    }
}

/// Persist only the prefix whose anchors are already present on the local
/// canonical chain. The complete eligible prefix is prevalidated before the
/// first write, preventing an invalid peer suffix from causing partial state
/// mutation.
async fn import_transition_history_through(
    state: &Arc<RwLock<NodeState>>,
    transitions: &[AuthenticatedValidatorSetSnapshot],
    requested_effective_height: u64,
) -> anyhow::Result<usize> {
    let (bootstrap, data_dir, canonical_tip, anchors) = {
        let state = state.read().await;
        let canonical_tip = state.chain.tip_height()?;
        let max_effective_height = requested_effective_height.min(canonical_tip.saturating_add(1));
        let mut anchors = HashMap::new();
        for transition in transitions
            .iter()
            .filter(|transition| transition.effective_from_height <= max_effective_height)
        {
            let anchor_height = transition.effective_from_height.saturating_sub(1);
            let block = state
                .chain
                .get_block_by_height(anchor_height)?
                .ok_or_else(|| {
                    anyhow::anyhow!("local canonical block {anchor_height} is unavailable")
                })?;
            anchors.insert(anchor_height, block.hash());
        }
        (
            state.validator_set_bootstrap.clone(),
            state.config.data_dir.clone(),
            canonical_tip,
            anchors,
        )
    };
    let max_effective_height = requested_effective_height.min(canonical_tip.saturating_add(1));
    let mut authority = bootstrap;
    let mut staged = Vec::new();
    for transition in transitions {
        transition
            .validate_against(&authority)
            .map_err(|error| anyhow::anyhow!("invalid transition certificate: {error}"))?;
        if transition.effective_from_height <= max_effective_height {
            let anchor_height = transition.effective_from_height.saturating_sub(1);
            let canonical_hash = anchors.get(&anchor_height).ok_or_else(|| {
                anyhow::anyhow!("local canonical block {anchor_height} is unavailable")
            })?;
            if transition.previous_block_hash.as_str() != canonical_hash.as_str() {
                anyhow::bail!(
                    "transition at height {} anchors a conflicting fork",
                    transition.effective_from_height
                );
            }
            staged.push((transition.clone(), authority.clone()));
        }
        authority = transition.validators.clone();
    }

    for (transition, previous) in &staged {
        crate::validator_set_history::record_authenticated(
            &data_dir,
            transition.clone(),
            previous,
        )?;
    }
    Ok(staged.len())
}

pub async fn run(state: Arc<RwLock<NodeState>>) {
    // Initial sync at startup.
    if let Err(e) = sync_once(Arc::clone(&state)).await {
        tracing::warn!("Initial chain sync failed: {}", e);
    }

    let mut ticker = interval(Duration::from_secs(SYNC_INTERVAL_SECS));
    tracing::info!("Chain sync running (interval: {}s)", SYNC_INTERVAL_SECS);

    loop {
        ticker.tick().await;
        if let Err(e) = sync_once(Arc::clone(&state)).await {
            tracing::debug!("Chain sync tick failed: {}", e);
        }
    }
}

async fn sync_once(state: Arc<RwLock<NodeState>>) -> anyhow::Result<()> {
    let (our_height, peer_urls) = {
        let s = state.read().await;
        (s.chain.tip_height()?, s.config.peers.clone())
    };

    let client = reqwest::Client::new();
    let (peer_height, peer_heights) = query_peer_heights(&client, &peer_urls).await;
    let transition_history =
        select_transition_history_for_sync(&state, &client, &peer_urls, &peer_heights).await?;

    {
        let mut s = state.write().await;
        s.sync_max_peer_tip = peer_height;
        s.sync_lag_blocks = sync_lag_blocks(our_height, peer_height);
    }

    if peer_height <= our_height {
        // A rebuilt node may already have all canonical blocks while its
        // durable authority history is missing. Recover the entire currently
        // anchored prefix even when there are no blocks to download.
        if let Some(transitions) = transition_history.as_deref() {
            import_transition_history_through(&state, transitions, our_height.saturating_add(1))
                .await?;
        }
        record_sync_stall(our_height, &peer_heights);
        tracing::debug!("Chain is up to date (height {})", our_height);
        return Ok(());
    }

    {
        let mut tracker = SYNC_STALL.lock().expect("sync stall mutex poisoned");
        tracker.last_height = our_height;
        tracker.stall_polls = 0;
    }

    tracing::info!(
        "Chain sync: our height={} peer height={} — fetching {} blocks",
        our_height,
        peer_height,
        peer_height - our_height
    );

    // Fetch each missing block in order and validate the chain linkage.
    let mut prev_hash = {
        let s = state.read().await;
        s.chain.tip_hash()?
    };

    for height in (our_height + 1)..=peer_height {
        let block = match fetch_block(&client, &peer_urls, height).await {
            Some(b) => b,
            None => {
                tracing::warn!("Could not fetch block {} from any peer", height);
                break;
            }
        };

        // Validate chain linkage. A mismatch means our local chain forked
        // from the network's canonical chain — attempt reorg recovery
        // instead of halting sync forever.
        if block.header.prev_hash != prev_hash {
            tracing::warn!(
                "Block {} has wrong prev_hash (expected {}, got {}) — \
                 local chain has diverged from peers, attempting reorg recovery",
                height,
                &prev_hash[..12],
                &block.header.prev_hash[..12]
            );
            match attempt_reorg_recovery(
                &state,
                &client,
                &peer_urls,
                height - 1,
                peer_height,
                transition_history.as_deref(),
            )
            .await
            {
                Ok(true) => {
                    tracing::info!("Reorg recovery succeeded — chain re-aligned with peers");
                }
                Ok(false) => {
                    tracing::warn!("Reorg recovery not applied — will retry on next sync tick");
                }
                Err(e) => {
                    tracing::error!("Reorg recovery failed: {}", e);
                }
            }
            break;
        }

        if block.header.height != height {
            tracing::error!(
                "Block claims height {} but we requested {}",
                block.header.height,
                height
            );
            break;
        }

        // A transition governing `height` is anchored to `height - 1`, which
        // is canonical locally after the linkage check. Import it before
        // selecting the authority used to verify this block.
        if let Some(transitions) = transition_history.as_deref() {
            import_transition_history_through(&state, transitions, height).await?;
        }

        // Verify that every Publish transaction carries a PBFT quorum of valid
        // signatures against the current validator set. Without this check a
        // malicious peer could serve a syntactically-valid block whose payload
        // never reached consensus. (ISSUE-018)
        let (validator_set, data_dir) = {
            let s = state.read().await;
            (s.validator_set.clone(), s.config.data_dir.clone())
        };
        // Verify against the validator set that was active at THIS block's
        // height (ISSUE-050), falling back to the current set when no
        // historical snapshot covers it (no regression).
        let effective_set = match crate::validator_set_history::verification_set_at(
            &data_dir,
            block.header.height,
            &validator_set,
        ) {
            Ok(set) => set,
            Err(error) => {
                tracing::error!(
                    "Block {} lacks authenticated validator authority: {}",
                    height,
                    error
                );
                break;
            }
        };
        if let Err(e) = verify_block_consensus_certificate(&block, &effective_set) {
            tracing::error!(
                "Block {} failed PBFT certificate verification: {} — halting sync",
                height,
                e
            );
            break;
        }
        if let Err(e) = verify_block_signatures(&block, &effective_set) {
            tracing::error!(
                "Block {} failed signature verification: {} — halting sync",
                height,
                e
            );
            break;
        }

        prev_hash = block.hash();

        // Insert and index the block.
        {
            let mut s = state.write().await;
            s.chain.insert_block(&block)?;
            s.publisher_index.apply_block(&block);
            s.on_block_committed(&block);
            s.sync_max_peer_tip = peer_height;
            s.sync_lag_blocks = sync_lag_blocks(height, peer_height);
        }

        tracing::info!("Synced block {} ({})", height, &prev_hash[..12]);
    }

    Ok(())
}

/// Fetch a block at `height` from the first peer that serves it.
async fn fetch_block(
    client: &reqwest::Client,
    peer_urls: &[String],
    height: u64,
) -> Option<common::Block> {
    for url in peer_urls {
        let full_url = format!("{}/v1/blocks/{}", url.trim_end_matches('/'), height);
        if let Ok(resp) = client.get(&full_url).send().await {
            if let Ok(b) = resp.json::<common::Block>().await {
                return Some(b);
            }
        }
    }
    None
}

/// Attempt to recover from a fork between the local chain and the network.
///
/// `local_tip` is the highest local height that conflicts with the peers'
/// branch (the block whose hash the peers' next block does not link to).
///
/// Strategy (longest-valid-chain):
///   1. Walk back from `local_tip` (bounded by `MAX_REORG_DEPTH`) comparing
///      local block hashes with peer block hashes to find the common ancestor.
///   2. Fetch the peers' branch from `ancestor+1..=peer_height`, verifying
///      linkage and the PBFT signature quorum on every block *before*
///      mutating any local state.
///   3. Only adopt the new branch if it is strictly longer than ours.
///   4. Roll back to the ancestor, insert the new branch, rebuild the package
///      and publisher indexes, and record a `ReorgEvent` for /v1/reorgs.
///
/// Returns Ok(true) if the reorg was applied, Ok(false) if it was declined
/// (no ancestor found within depth limit, branch invalid, or not longer).
async fn attempt_reorg_recovery(
    state: &Arc<RwLock<NodeState>>,
    client: &reqwest::Client,
    peer_urls: &[String],
    local_tip: u64,
    peer_height: u64,
    transition_history: Option<&[AuthenticatedValidatorSetSnapshot]>,
) -> anyhow::Result<bool> {
    // ── 1. Find the common ancestor ───────────────────────────────────────
    let floor = local_tip.saturating_sub(MAX_REORG_DEPTH);
    let mut ancestor: Option<(u64, String)> = None;

    for k in (floor..=local_tip).rev() {
        let ours = {
            let s = state.read().await;
            s.chain.get_block_by_height(k)?
        };
        let Some(ours) = ours else { continue };
        let Some(theirs) = fetch_block(client, peer_urls, k).await else {
            tracing::warn!(
                "Reorg recovery: could not fetch peer block {} — aborting",
                k
            );
            return Ok(false);
        };
        if ours.hash() == theirs.hash() {
            ancestor = Some((k, ours.hash()));
            break;
        }
    }

    let Some((fork_height, ancestor_hash)) = ancestor else {
        tracing::error!(
            "Reorg recovery: no common ancestor within {} blocks of height {} — \
             this fork is too deep to auto-recover; operator intervention required",
            MAX_REORG_DEPTH,
            local_tip
        );
        return Ok(false);
    };

    // ── 2. Fetch and fully validate the peers' branch before adopting ────
    let (validator_set, bootstrap_set, data_dir) = {
        let s = state.read().await;
        (
            s.validator_set.clone(),
            s.validator_set_bootstrap.clone(),
            s.config.data_dir.clone(),
        )
    };

    if let Some(transitions) = transition_history {
        let retained_local: Vec<_> = crate::validator_set_history::load(&data_dir)
            .into_iter()
            .filter(|entry| entry.effective_from_height <= fork_height.saturating_add(1))
            .filter_map(|entry| entry.certificate)
            .collect();
        if let Err(error) = ensure_histories_are_prefix_compatible(&retained_local, transitions) {
            tracing::error!(
                "Reorg recovery: peer validator authority conflicts at or before the common ancestor: {}",
                error
            );
            return Ok(false);
        }
    }

    let mut new_branch: Vec<common::Block> = Vec::new();
    let mut link_hash = ancestor_hash;
    for h in (fork_height + 1)..=peer_height {
        let Some(b) = fetch_block(client, peer_urls, h).await else {
            tracing::warn!(
                "Reorg recovery: peer branch incomplete at height {} — aborting",
                h
            );
            return Ok(false);
        };
        if b.header.height != h {
            tracing::warn!(
                "Reorg recovery: block claims height {} at {} — aborting",
                b.header.height,
                h
            );
            return Ok(false);
        }
        if b.header.prev_hash != link_hash {
            tracing::warn!(
                "Reorg recovery: peer branch broke linkage at height {} — aborting",
                h
            );
            return Ok(false);
        }
        if let Some(transitions) = transition_history {
            if let Some(transition) = transitions
                .iter()
                .find(|transition| transition.effective_from_height == h)
            {
                if transition.previous_block_hash != link_hash {
                    tracing::error!(
                        "Reorg recovery: validator transition at height {} anchors a different fork",
                        h
                    );
                    return Ok(false);
                }
            }
        }
        let effective_set = if let Some(transitions) = transition_history {
            transition_history_set_at(transitions, &bootstrap_set, b.header.height)
        } else {
            match crate::validator_set_history::verification_set_at(
                &data_dir,
                b.header.height,
                &validator_set,
            ) {
                Ok(set) => set,
                Err(error) => {
                    tracing::error!(
                        "Reorg branch height {} lacks authenticated validator authority: {}",
                        h,
                        error
                    );
                    return Ok(false);
                }
            }
        };
        if let Err(e) = verify_block_consensus_certificate(&b, &effective_set) {
            tracing::error!(
                "Reorg recovery: peer branch block {} failed PBFT certificate verification: {} — refusing to adopt fork",
                h,
                e
            );
            return Ok(false);
        }
        if let Err(e) = verify_block_signatures(&b, &effective_set) {
            tracing::error!(
                "Reorg recovery: peer branch block {} failed signature verification: {} — \
                 refusing to adopt fork",
                h,
                e
            );
            return Ok(false);
        }
        link_hash = b.hash();
        new_branch.push(b);
    }

    // ── 3. Longest-chain rule: only adopt a strictly longer branch ───────
    let new_tip_height = fork_height + new_branch.len() as u64;
    if new_tip_height <= local_tip {
        tracing::warn!(
            "Reorg recovery: peer branch tip {} is not longer than local tip {} — keeping ours",
            new_tip_height,
            local_tip
        );
        return Ok(false);
    }

    // ── 4. Apply: rewind, adopt, rebuild derived state, record the event ─
    {
        let mut s = state.write().await;
        let abandoned = s.chain.rollback_to_height(fork_height)?;
        crate::validator_set_history::truncate_after_reorg(&data_dir, fork_height)?;
        for b in &new_branch {
            s.chain.insert_block(b)?;
        }
        s.chain.rebuild_package_index()?;

        let tip = s.chain.tip_height()?;
        let mut canonical_blocks = Vec::with_capacity((tip + 1) as usize);
        for h in 0..=tip {
            if let Some(b) = s.chain.get_block_by_height(h)? {
                canonical_blocks.push(b);
            }
        }
        s.publisher_index
            .rebuild_from_chain(canonical_blocks.iter());

        let depth = local_tip - fork_height;
        s.record_reorg(depth, abandoned, link_hash.clone());
    }

    if let Some(transitions) = transition_history {
        import_transition_history_through(state, transitions, new_tip_height.saturating_add(1))
            .await?;
    }

    tracing::info!(
        "Reorg applied: fork at height {}, new tip {} ({})",
        fork_height,
        new_tip_height,
        &link_hash[..link_hash.len().min(12)]
    );
    Ok(true)
}

/// Verify the block-level PBFT COMMIT certificate and every header commitment
/// before any transaction-specific evidence is considered. This covers blocks
/// containing governance or validator lifecycle transactions as well as
/// package publications.
fn verify_block_consensus_certificate(
    block: &common::Block,
    validator_set: &ValidatorSet,
) -> anyhow::Result<()> {
    if block.header.height == 0 {
        return Ok(());
    }
    if validator_set.validators.is_empty() {
        anyhow::bail!("cannot verify block: validator set is empty");
    }

    let expected_set_hash = common::canonical_validator_set_hash(validator_set);
    if block.header.validator_set_hash != expected_set_hash {
        anyhow::bail!("block validator-set hash does not match authenticated authority");
    }
    let expected_merkle_root = common::merkle_root(&block.transactions);
    if block.header.merkle_root != expected_merkle_root {
        anyhow::bail!("block transaction Merkle root is invalid");
    }
    if block.header.vrf_output.is_some() != block.header.vrf_proof.is_some() {
        anyhow::bail!("block must carry both VRF output and proof or neither");
    }

    let mut round = consensus::pbft::PbftRound::new(block.clone(), validator_set.clone().into());
    round
        .pre_prepare(&block.header.proposer_id)
        .map_err(|error| anyhow::anyhow!("invalid PBFT pre-prepare: {error}"))?;

    let allow_small_cluster_quorum = std::env::var("CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let validator_count = validator_set.validators.len();
    let quorum = if validator_count == 3 && allow_small_cluster_quorum {
        2
    } else {
        (2 * validator_count / 3) + 1
    };
    let mut seen = HashSet::new();
    let block_hash = block.hash();
    let mut valid_commits = 0usize;
    for signature in &block.pbft_signatures {
        if !seen.insert(signature.validator_id.clone()) {
            anyhow::bail!("PBFT certificate contains a duplicate validator identity");
        }
        let validator = validator_set
            .validators
            .iter()
            .find(|validator| validator.id == signature.validator_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "PBFT certificate names unknown validator {}",
                    signature.validator_id
                )
            })?;
        if validator.status != "online" && validator.status != "self" {
            anyhow::bail!(
                "PBFT certificate names inactive validator {}",
                signature.validator_id
            );
        }
        if !validator.pubkey.eq_ignore_ascii_case(&signature.pubkey) {
            anyhow::bail!(
                "PBFT certificate pubkey mismatch for validator {}",
                signature.validator_id
            );
        }
        consensus::pbft::verify_pbft_phase_signature(
            "commit",
            &block_hash,
            &validator.pubkey,
            &signature.signature,
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "invalid PBFT COMMIT signature from {}: {error}",
                signature.validator_id
            )
        })?;
        valid_commits += 1;
    }
    if valid_commits < quorum {
        anyhow::bail!(
            "PBFT certificate has {} valid commits, requires {} of {} validators",
            valid_commits,
            quorum,
            validator_count
        );
    }
    Ok(())
}

/// Verify consensus-grade package evidence on every Publish transaction in a block.
///
/// Each `ChainRecord` in the block must carry at least `⌊2n/3⌋ + 1` valid
/// Ed25519 signatures from validators currently in the active set, where each
/// signature is over the domain-separated payload produced by
/// `crate::gossip::canonical_vote_message(...)`.
///
/// Non-Publish transactions are covered by the block-level PBFT certificate
/// above and remain subject to their state-transition validation rules.
///   * Single-validator deployments still require the one validator's signature.
fn verify_block_signatures(
    block: &common::Block,
    validator_set: &ValidatorSet,
) -> anyhow::Result<()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    // Genesis has no signatures by design.
    if block.header.height == 0 {
        return Ok(());
    }

    let n = validator_set.validators.len();
    if n == 0 {
        anyhow::bail!("cannot verify block: local validator set is empty");
    }
    let quorum = (2 * n / 3) + 1;

    // Build a pubkey → validator_id lookup once so per-signature verification
    // is O(1) instead of O(n).
    let known: std::collections::HashMap<String, &common::Validator> = validator_set
        .validators
        .iter()
        .map(|v| (v.pubkey.to_ascii_lowercase(), v))
        .collect();

    for (tx_idx, tx) in block.transactions.iter().enumerate() {
        let (record, publisher_authorization_digest) = match tx {
            Transaction::Publish(record) => (record, None),
            Transaction::PublishAuthorizedV1 {
                record,
                authorization,
            } => (
                record,
                Some(authorization.authorization_digest(&record.id, &record.content_hash)),
            ),
            _ => continue,
        };
        let canonical = record.id.canonical();

        let mut approval_groups: HashMap<String, usize> = HashMap::new();
        let mut seen = HashSet::new();

        for sig in &record.validator_signatures {
            if !matches!(sig.vote, ValidatorVote::Approve) {
                continue;
            }
            if !common::is_consensus_grade_vote(
                &sig.ml_model_version,
                &sig.analysis_bundles,
                &sig.evidence_digest,
            ) {
                tracing::debug!(
                    "sync: tx {} carries consensus-ineligible signature from {} — ignored",
                    tx_idx,
                    sig.validator_id
                );
                continue;
            }

            let pubkey_key = sig.validator_pubkey.to_ascii_lowercase();
            if !known.contains_key(&pubkey_key) {
                tracing::debug!(
                    "sync: tx {} carries signature from unknown validator pubkey {} — ignored",
                    tx_idx,
                    pubkey_key
                );
                continue;
            }
            if !seen.insert(pubkey_key.clone()) {
                // Duplicate signature from the same validator — count only once.
                continue;
            }

            let pubkey_bytes = match hex::decode(&sig.validator_pubkey) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let vk = match VerifyingKey::try_from(pubkey_bytes.as_slice()) {
                Ok(k) => k,
                Err(_) => continue,
            };
            let sig_bytes = match hex::decode(&sig.signature) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let ed_sig = match Signature::try_from(sig_bytes.as_slice()) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let scanner_profile =
                common::scanner_profile_digest(&sig.ml_model_version, &sig.analysis_bundles);
            let reject_reason = match &sig.vote {
                ValidatorVote::Approve => None,
                ValidatorVote::Reject { reason } => Some(reason.as_str()),
            };
            let vote_metadata = crate::gossip::vote_metadata_digest(
                sig.is_mandatory,
                &sig.deterministic_risk,
                reject_reason,
            );
            let message = match publisher_authorization_digest.as_deref() {
                Some(digest) => crate::gossip::canonical_authorized_vote_message(
                    &canonical,
                    &record.content_hash,
                    matches!(sig.vote, ValidatorVote::Approve),
                    &sig.validator_pubkey,
                    &scanner_profile,
                    &sig.evidence_digest,
                    digest,
                    &vote_metadata,
                ),
                None => crate::gossip::canonical_vote_message(
                    &canonical,
                    &record.content_hash,
                    matches!(sig.vote, ValidatorVote::Approve),
                    &sig.validator_pubkey,
                    &scanner_profile,
                    &sig.evidence_digest,
                ),
            };

            if vk.verify(message.as_bytes(), &ed_sig).is_ok() {
                let profile_key = format!(
                    "{}:{}",
                    common::scanner_profile_digest(&sig.ml_model_version, &sig.analysis_bundles),
                    sig.evidence_digest
                );
                *approval_groups.entry(profile_key).or_default() += 1;
            }
        }

        let approvals = approval_groups.values().copied().max().unwrap_or(0);
        if approvals < quorum {
            anyhow::bail!(
                "tx {} ({}): only {} valid signatures, need {} ({}/{} validators)",
                tx_idx,
                canonical,
                approvals,
                quorum,
                approvals,
                n
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_histories_are_prefix_compatible, validate_peer_transition_history,
        verify_block_consensus_certificate, verify_block_signatures, PeerTransitionHistory,
    };
    use chrono::Utc;
    use common::{
        merkle_root, validator_set_transition_message, AnalysisBundleRefs,
        AuthenticatedValidatorSetSnapshot, Block, BlockHeader, ChainRecord,
        DeterministicRiskSummary, PackageId, PackageStatus, Transaction, Validator, ValidatorSet,
        ValidatorSetTransitionSignature, ValidatorSignature, ValidatorVote,
        VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
    };
    use consensus::{aggregate_evidence_votes, EvidenceVoteOutcome};
    use ed25519_dalek::{Signer, SigningKey};
    use std::collections::HashMap;

    #[derive(Clone, Copy)]
    enum VotePayloadFormat {
        Canonical,
        Legacy,
    }

    fn validator(id: &str, signing_key: &SigningKey) -> Validator {
        Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: hex::encode(signing_key.verifying_key().as_bytes()),
            eth_address: String::new(),
            stake: 100,
            reputation: 100,
            status: "online".into(),
            performance_score: 500,
            contribution_count: 0,
        }
    }

    fn validator_set(keys: &[SigningKey]) -> ValidatorSet {
        ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(index, key)| validator(&format!("validator-{index}"), key))
                .collect(),
        )
    }

    fn signed_transition(
        effective_from_height: u64,
        previous_block_hash: &str,
        previous: &ValidatorSet,
        next: &ValidatorSet,
        previous_keys: &[SigningKey],
    ) -> AuthenticatedValidatorSetSnapshot {
        let mut transition = AuthenticatedValidatorSetSnapshot {
            schema_version: VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height,
            previous_block_hash: previous_block_hash.into(),
            previous_validator_set_hash: common::canonical_validator_set_hash(previous),
            validators: next.clone(),
            validator_set_hash: common::canonical_validator_set_hash(next),
            transition_signatures: Vec::new(),
        };
        let message = validator_set_transition_message(&transition);
        let quorum = (2 * previous.validators.len()) / 3 + 1;
        transition.transition_signatures = previous
            .validators
            .iter()
            .zip(previous_keys)
            .take(quorum)
            .map(|(validator, key)| ValidatorSetTransitionSignature {
                validator_id: validator.id.clone(),
                validator_pubkey: validator.pubkey.clone(),
                signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
            })
            .collect();
        transition
    }

    fn signed_approval(
        validator_id: &str,
        signing_key: &SigningKey,
        canonical: &str,
        content_hash: &str,
        format: VotePayloadFormat,
    ) -> ValidatorSignature {
        let validator_pubkey = hex::encode(signing_key.verifying_key().as_bytes());
        let analysis_bundles = AnalysisBundleRefs {
            policy_bundle_id: "policy-v1".into(),
            feature_schema_id: "features-v1".into(),
            expert_bundle_id: "experts-v1".into(),
            embedding_model_id: "embeddings-v1".into(),
            index_epoch: "index-epoch-1".into(),
            threshold_profile_id: "thresholds-v1".into(),
            llm_prompt_profile_id: "llm-prompt-v1".into(),
            osv_snapshot_epoch: "osv-off".into(),
        };
        let evidence_digest =
            common::sha256_hex(format!("{canonical}:{content_hash}:sync-test-evidence").as_bytes());
        let ml_model_version = "creg-detect-v1.0.0".to_string();
        let message = match format {
            VotePayloadFormat::Canonical => crate::gossip::canonical_vote_message(
                canonical,
                content_hash,
                true,
                &validator_pubkey,
                &common::scanner_profile_digest(&ml_model_version, &analysis_bundles),
                &evidence_digest,
            ),
            VotePayloadFormat::Legacy => format!("{}-{}", canonical, content_hash),
        };

        ValidatorSignature {
            validator_id: validator_id.into(),
            validator_pubkey,
            signature: hex::encode(signing_key.sign(message.as_bytes()).to_bytes()),
            vote: ValidatorVote::Approve,
            signed_at: Utc::now(),
            ml_model_version,
            analysis_bundles,
            evidence_digest,
            deterministic_risk: DeterministicRiskSummary::default(),
            is_mandatory: true,
        }
    }

    fn publish_block(
        record: ChainRecord,
        validator_set: &ValidatorSet,
        signing_keys: &[(&str, SigningKey)],
    ) -> Block {
        let tx = Transaction::Publish(record);
        let prev_hash = Block::genesis().hash();
        let active: Vec<_> = validator_set
            .validators
            .iter()
            .map(|validator| consensus::vrf::VrfValidator {
                id: validator.id.clone(),
                pubkey: validator.pubkey.clone(),
                vrf_output: None,
                vrf_proof: None,
            })
            .collect();
        let proposer_id = consensus::vrf::select_proposer_deterministic(&active, &prev_hash)
            .expect("test validator set selects a proposer");
        let mut block = Block {
            header: BlockHeader {
                height: 1,
                prev_hash,
                merkle_root: merkle_root(std::slice::from_ref(&tx)),
                proposer_id,
                timestamp: Utc::now(),
                validator_set_hash: common::canonical_validator_set_hash(validator_set),
                vrf_output: None,
                vrf_proof: None,
            },
            transactions: vec![tx],
            pbft_signatures: vec![],
        };
        let block_hash = block.hash();
        block.pbft_signatures = signing_keys
            .iter()
            .map(|(validator_id, key)| common::BlockSignature {
                validator_id: (*validator_id).to_string(),
                pubkey: hex::encode(key.verifying_key().as_bytes()),
                signature: hex::encode(
                    key.sign(
                        consensus::pbft::pbft_signature_message("commit", &block_hash).as_bytes(),
                    )
                    .to_bytes(),
                ),
            })
            .collect();
        block
    }

    fn publish_record(
        package_id: &PackageId,
        content_hash: &str,
        validator_signatures: Vec<ValidatorSignature>,
    ) -> ChainRecord {
        ChainRecord {
            id: package_id.clone(),
            content_hash: content_hash.into(),
            ipfs_cid: "bafytestcid".into(),
            publisher_pubkey: "publisher-pubkey".into(),
            block_hash: String::new(),
            published_at: Utc::now(),
            validator_signatures,
            status: PackageStatus::Verified,
            shielded: false,
            key_bundle: None,
            pgp_fingerprint: None,
            findings: vec![],
            analysis_bundles: AnalysisBundleRefs::default(),
            evidence_digest: String::new(),
            deterministic_risk: DeterministicRiskSummary::default(),
            access_count: 0,
            last_accessed: None,
            threshold: 0,
            publisher_pubkeys: vec![],
            manifest: None,
            analysis_committee_size: 0,
            analysis_selection_seed: String::new(),
            analysis_effective_size: 0,
            analysis_mandatory_contributed: 0,
            analysis_committee: None,
        }
    }

    #[test]
    fn sync_lag_blocks_saturates_at_zero_when_caught_up() {
        assert_eq!(super::sync_lag_blocks(10, 10), 0);
        assert_eq!(super::sync_lag_blocks(12, 10), 0);
        assert_eq!(super::sync_lag_blocks(3, 8), 5);
    }

    #[test]
    fn rebuilt_node_history_requires_exact_checkpoint_and_canonical_anchors() {
        let keys: Vec<_> = (1..=4)
            .map(|byte| SigningKey::from_bytes(&[byte; 32]))
            .collect();
        let bootstrap = validator_set(&keys);
        let anchor_one = "a".repeat(64);
        let anchor_three = "b".repeat(64);
        let checkpoint = signed_transition(2, &anchor_one, &bootstrap, &bootstrap, &keys);
        let descendant = signed_transition(4, &anchor_three, &bootstrap, &bootstrap, &keys);
        let history = PeerTransitionHistory {
            source: "peer-a".into(),
            advertised_tip: 4,
            transitions: vec![checkpoint.clone(), descendant.clone()],
        };
        let anchors = HashMap::from([(1, anchor_one.clone()), (3, anchor_three.clone())]);

        validate_peer_transition_history(&history, &bootstrap, Some(&checkpoint), 3, &anchors)
            .expect("exact checkpoint-descended history should be accepted");

        let missing_checkpoint = PeerTransitionHistory {
            transitions: vec![descendant],
            ..history.clone()
        };
        let error = validate_peer_transition_history(
            &missing_checkpoint,
            &bootstrap,
            Some(&checkpoint),
            3,
            &anchors,
        )
        .expect_err("a rebuilt node must not trust history missing its checkpoint");
        assert!(
            error.contains("does not contain"),
            "unexpected error: {error}"
        );

        let fork_anchors = HashMap::from([(1, "c".repeat(64)), (3, anchor_three)]);
        let error = validate_peer_transition_history(
            &history,
            &bootstrap,
            Some(&checkpoint),
            3,
            &fork_anchors,
        )
        .expect_err("a transition anchored to another fork must be rejected");
        assert!(
            error.contains("conflicting fork"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn long_range_or_conflicting_transition_timelines_are_rejected() {
        let keys: Vec<_> = (11..=14)
            .map(|byte| SigningKey::from_bytes(&[byte; 32]))
            .collect();
        let bootstrap = validator_set(&keys);
        let checkpoint = signed_transition(2, &"a".repeat(64), &bootstrap, &bootstrap, &keys);
        let canonical = signed_transition(4, &"b".repeat(64), &bootstrap, &bootstrap, &keys);
        let long_range = signed_transition(4, &"c".repeat(64), &bootstrap, &bootstrap, &keys);

        let canonical_history = vec![checkpoint.clone(), canonical];
        let alternative_history = vec![checkpoint.clone(), long_range];
        let error =
            ensure_histories_are_prefix_compatible(&canonical_history, &alternative_history)
                .expect_err("two certified timelines at the same height must not be peer-voted");
        assert!(error.contains("conflicting"), "unexpected error: {error}");

        let missing_intermediate = vec![
            checkpoint.clone(),
            signed_transition(5, &"d".repeat(64), &bootstrap, &bootstrap, &keys),
        ];
        let error =
            ensure_histories_are_prefix_compatible(&canonical_history, &missing_intermediate)
                .expect_err("a peer may not omit an intermediate authority transition");
        assert!(error.contains("conflicting"), "unexpected error: {error}");

        let conflicting_checkpoint =
            signed_transition(2, &"e".repeat(64), &bootstrap, &bootstrap, &keys);
        let alternative_checkpoint_history = PeerTransitionHistory {
            source: "long-range-peer".into(),
            advertised_tip: 4,
            transitions: vec![conflicting_checkpoint],
        };
        let anchors = HashMap::from([(1, "e".repeat(64))]);
        let error = validate_peer_transition_history(
            &alternative_checkpoint_history,
            &bootstrap,
            Some(&checkpoint),
            1,
            &anchors,
        )
        .expect_err("an internally valid long-range checkpoint replacement must be rejected");
        assert!(
            error.contains("does not contain"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn aggregated_votes_pass_sync_only_with_canonical_payload() {
        let package_id = PackageId::new("npm", "pkg", "1.0.0");
        let canonical = package_id.canonical();
        let content_hash = common::sha256_hex(b"vote-sync-regression");

        let signing_keys = [
            ("node-1", SigningKey::from_bytes(&[7u8; 32])),
            ("node-2", SigningKey::from_bytes(&[8u8; 32])),
            ("node-3", SigningKey::from_bytes(&[9u8; 32])),
        ];
        let validator_set = ValidatorSet::new(
            signing_keys
                .iter()
                .map(|(id, key)| validator(id, key))
                .collect(),
        );

        let canonical_votes: Vec<_> = signing_keys
            .iter()
            .map(|(id, key)| {
                signed_approval(
                    id,
                    key,
                    &canonical,
                    &content_hash,
                    VotePayloadFormat::Canonical,
                )
            })
            .collect();
        let canonical_outcome = aggregate_evidence_votes(
            &canonical_votes,
            &common::AnalysisCommittee {
                size: validator_set.validators.len() as u32,
                members: validator_set
                    .validators
                    .iter()
                    .map(|v| v.id.clone())
                    .collect(),
                seed: String::new(),
                effective_size: validator_set.validators.len() as u32,
                performance_bias: true,
                ..Default::default()
            },
        )
        .expect("three canonical approvals should satisfy aggregation quorum");
        let EvidenceVoteOutcome::Verified(canonical_signatures) = canonical_outcome else {
            panic!("expected verified aggregation outcome for canonical votes");
        };

        let canonical_block = publish_block(
            publish_record(&package_id, &content_hash, canonical_signatures),
            &validator_set,
            &signing_keys,
        );
        verify_block_consensus_certificate(&canonical_block, &validator_set)
            .expect("follower sync should accept the PBFT commit certificate");
        verify_block_signatures(&canonical_block, &validator_set)
            .expect("follower sync should accept canonical aggregated votes");

        let mut missing_commit = canonical_block.clone();
        missing_commit.pbft_signatures.truncate(1);
        assert!(verify_block_consensus_certificate(&missing_commit, &validator_set).is_err());

        let mut wrong_set_hash = canonical_block.clone();
        wrong_set_hash.header.validator_set_hash = "f".repeat(64);
        assert!(verify_block_consensus_certificate(&wrong_set_hash, &validator_set).is_err());

        let mut duplicate_commit = canonical_block.clone();
        duplicate_commit
            .pbft_signatures
            .push(duplicate_commit.pbft_signatures[0].clone());
        assert!(verify_block_consensus_certificate(&duplicate_commit, &validator_set).is_err());

        let legacy_votes: Vec<_> = signing_keys
            .iter()
            .map(|(id, key)| {
                signed_approval(
                    id,
                    key,
                    &canonical,
                    &content_hash,
                    VotePayloadFormat::Legacy,
                )
            })
            .collect();
        let legacy_outcome = aggregate_evidence_votes(
            &legacy_votes,
            &common::AnalysisCommittee {
                size: validator_set.validators.len() as u32,
                members: validator_set
                    .validators
                    .iter()
                    .map(|v| v.id.clone())
                    .collect(),
                seed: String::new(),
                effective_size: validator_set.validators.len() as u32,
                performance_bias: true,
                ..Default::default()
            },
        )
        .expect("three legacy approvals still satisfy vote aggregation quorum");
        let EvidenceVoteOutcome::Verified(legacy_signatures) = legacy_outcome else {
            panic!("expected verified aggregation outcome for legacy votes");
        };

        let legacy_block = publish_block(
            publish_record(&package_id, &content_hash, legacy_signatures),
            &validator_set,
            &signing_keys,
        );
        let error = verify_block_signatures(&legacy_block, &validator_set)
            .expect_err("follower sync must reject the legacy vote payload format");

        assert!(
            error
                .to_string()
                .contains("only 0 valid signatures, need 3"),
            "unexpected sync failure: {error}"
        );
    }
}
