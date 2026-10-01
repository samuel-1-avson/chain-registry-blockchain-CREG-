// crates/node/src/l1_quorum.rs
//
// Quorum/aggregation helpers for reading L1 (Sepolia) from MULTIPLE RPC
// endpoints instead of trusting a single provider. A single compromised,
// censoring, or merely stale RPC must not be able to skew the validator-set
// view or force a destructive reorg rebuild.
//
// This module holds the *pure* aggregation logic so it can be unit-tested
// without a network. The async orchestration (fan-out across endpoints) lives
// in `validator_set_sync.rs`, which calls these helpers.
//
// Trust model:
//   * Block height — `aggregate_height` is deliberately conservative. With 3+
//     endpoints it takes the median (one outlier in either direction cannot
//     move it). With 2 it takes the minimum (a stale/lying-high RPC cannot
//     push the head forward; trailing slightly is safe because a finality lag
//     is applied on top). With 1 it degrades to that single value.
//   * Block hash — `majority_hash` requires a STRICT majority. Reorg detection
//     keys off this: if endpoints disagree with no majority, the caller must
//     treat the result as inconclusive and NOT rebuild, so one bad RPC cannot
//     trigger a spurious validator-set rebuild.

use std::collections::HashMap;

/// Number of responses needed to form a strict majority of configured
/// endpoints. A majority of only the responders is insufficient: if four
/// providers are configured, two matching answers and two timeouts are not a
/// quorum.
pub fn configured_quorum_size(configured: usize) -> usize {
    if configured == 0 {
        0
    } else {
        configured / 2 + 1
    }
}

/// Whether the successful responses include a strict majority of all
/// configured endpoints.
pub fn has_configured_quorum(responders: usize, configured: usize) -> bool {
    configured > 0 && responders >= configured_quorum_size(configured)
}

/// Combine per-endpoint head block numbers into a single conservative height.
///
/// * 0 values  → `None` (all endpoints failed; caller should mark degraded).
/// * 1 value   → that value (single-endpoint behaviour).
/// * 2 values  → the minimum (do not let one endpoint race the head forward).
/// * 3+ values → the median (resists a single high or low outlier).
pub fn aggregate_height(values: &[u64]) -> Option<u64> {
    match values.len() {
        0 => None,
        1 => Some(values[0]),
        2 => Some(values[0].min(values[1])),
        _ => {
            let mut sorted = values.to_vec();
            sorted.sort_unstable();
            Some(sorted[sorted.len() / 2])
        }
    }
}

/// Combine head heights only when a strict majority of configured endpoints
/// answered. Height aggregation remains conservative after the quorum check.
pub fn aggregate_height_with_quorum(values: &[u64], configured: usize) -> Option<u64> {
    has_configured_quorum(values.len(), configured)
        .then(|| aggregate_height(values))
        .flatten()
}

/// Return the block hash agreed by a STRICT majority of responders
/// (case-insensitive), or `None` when there is no majority (tie, even split,
/// or empty). Comparison is normalised to lowercase; the returned value is the
/// normalised form.
pub fn majority_hash(hashes: &[String]) -> Option<String> {
    if hashes.is_empty() {
        return None;
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for h in hashes {
        *counts.entry(h.trim().to_ascii_lowercase()).or_default() += 1;
    }
    let (best, count) = counts.into_iter().max_by_key(|(_, c)| *c)?;
    if count * 2 > hashes.len() {
        Some(best)
    } else {
        None
    }
}

/// Return a block hash only when it has a strict majority of all configured
/// endpoints. The responder count must itself reach that configured quorum.
pub fn majority_hash_with_quorum(hashes: &[String], configured: usize) -> Option<String> {
    if !has_configured_quorum(hashes.len(), configured) {
        return None;
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for hash in hashes {
        *counts.entry(hash.trim().to_ascii_lowercase()).or_default() += 1;
    }
    let (best, count) = counts.into_iter().max_by_key(|(_, count)| *count)?;
    (count >= configured_quorum_size(configured)).then_some(best)
}

/// Return a cloned response only when the exact value has a strict majority
/// of all configured endpoints.
pub fn identical_response_with_quorum<T: Clone + PartialEq>(
    responses: &[T],
    configured: usize,
) -> Option<T> {
    if !has_configured_quorum(responses.len(), configured) {
        return None;
    }
    responses.iter().find_map(|candidate| {
        (responses
            .iter()
            .filter(|response| *response == candidate)
            .count()
            >= configured_quorum_size(configured))
        .then(|| candidate.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_height_handles_small_sets() {
        assert_eq!(aggregate_height(&[]), None);
        assert_eq!(aggregate_height(&[100]), Some(100));
        // Two endpoints: take the minimum so a high outlier cannot push us ahead.
        assert_eq!(aggregate_height(&[100, 105]), Some(100));
    }

    #[test]
    fn aggregate_height_takes_median_with_three_or_more() {
        // A single high liar (999) is ignored by the median.
        assert_eq!(aggregate_height(&[100, 101, 999]), Some(101));
        // A single low/stale endpoint (1) is ignored too.
        assert_eq!(aggregate_height(&[1, 100, 101]), Some(100));
        assert_eq!(aggregate_height(&[100, 100, 100, 100, 999]), Some(100));
    }

    #[test]
    fn majority_hash_requires_strict_majority() {
        let two_one = vec!["0xAA".to_string(), "0xaa".to_string(), "0xbb".to_string()];
        assert_eq!(majority_hash(&two_one), Some("0xaa".to_string()));

        // Even split → no majority → inconclusive.
        let split = vec!["0xaa".to_string(), "0xbb".to_string()];
        assert_eq!(majority_hash(&split), None);

        // Single responder is trivially a majority of itself.
        assert_eq!(
            majority_hash(&["0xaa".to_string()]),
            Some("0xaa".to_string())
        );

        assert_eq!(majority_hash(&[]), None);
    }

    #[test]
    fn majority_hash_no_winner_on_three_way_tie() {
        let tie = vec!["0xaa".to_string(), "0xbb".to_string(), "0xcc".to_string()];
        assert_eq!(majority_hash(&tie), None);
    }

    #[test]
    fn configured_quorum_counts_failed_endpoints() {
        assert_eq!(configured_quorum_size(0), 0);
        assert_eq!(configured_quorum_size(1), 1);
        assert_eq!(configured_quorum_size(3), 2);
        assert_eq!(configured_quorum_size(4), 3);

        assert!(!has_configured_quorum(2, 4));
        assert!(has_configured_quorum(3, 4));
    }

    #[test]
    fn aggregate_height_requires_configured_quorum() {
        assert_eq!(aggregate_height_with_quorum(&[10], 3), None);
        assert_eq!(aggregate_height_with_quorum(&[10, 12], 3), Some(10));
        assert_eq!(aggregate_height_with_quorum(&[10, 12], 4), None);
        assert_eq!(aggregate_height_with_quorum(&[10, 12, 11], 4), Some(11));
    }

    #[test]
    fn majority_hash_requires_quorum_of_configured_endpoints() {
        assert_eq!(
            majority_hash_with_quorum(&["0xaa".to_string(), "0xaa".to_string()], 3),
            Some("0xaa".to_string())
        );
        // Two matching responses are not a strict majority of four configured
        // providers, even when the other two did not answer.
        assert_eq!(
            majority_hash_with_quorum(
                &["0xaa".to_string(), "0xaa".to_string(), "0xbb".to_string()],
                4
            ),
            None
        );
        assert_eq!(
            majority_hash_with_quorum(
                &["0xaa".to_string(), "0xaa".to_string(), "0xbb".to_string()],
                3
            ),
            Some("0xaa".to_string())
        );
    }

    #[test]
    fn identical_rpc_responses_require_configured_quorum() {
        assert_eq!(
            identical_response_with_quorum(&["same", "same", "different"], 3),
            Some("same")
        );
        assert_eq!(
            identical_response_with_quorum(&["same", "same", "same", "different"], 4),
            Some("same")
        );
        assert_eq!(
            identical_response_with_quorum(&["same", "same", "different"], 4),
            None
        );
        assert_eq!(identical_response_with_quorum::<&str>(&[], 0), None);
    }
}
