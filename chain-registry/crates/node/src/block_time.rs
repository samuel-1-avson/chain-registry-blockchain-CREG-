//! Shared block-timestamp validation.
//!
//! Chain replay needs a deterministic parent/child ordering rule, while live
//! PBFT proposal validation additionally needs a wall-clock bound so an
//! otherwise expired authorization cannot be revived by backdating a new
//! block. Historical sync must not apply the live wall-clock lower bound.

use chrono::{DateTime, Utc};

/// Maximum clock disagreement tolerated for a live PBFT proposal.
pub const MAX_LIVE_BLOCK_CLOCK_SKEW_SECS: i64 = 60;

/// Deterministic rule suitable for insertion, replay, sync, and reorg checks.
pub fn validate_chain_timestamp(
    parent: DateTime<Utc>,
    candidate: DateTime<Utc>,
) -> anyhow::Result<()> {
    if candidate <= parent {
        anyhow::bail!("block timestamp must be strictly later than its canonical parent");
    }
    Ok(())
}

/// Additional rule for a newly observed PBFT pre-prepare.
pub fn validate_live_timestamp(
    parent: DateTime<Utc>,
    candidate: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    validate_chain_timestamp(parent, candidate)?;
    if candidate.timestamp_subsec_nanos() != 0 {
        anyhow::bail!("live block timestamp must use whole Unix seconds");
    }
    let candidate_seconds = candidate.timestamp();
    let now_seconds = now.timestamp();
    if candidate_seconds < now_seconds.saturating_sub(MAX_LIVE_BLOCK_CLOCK_SKEW_SECS)
        || candidate_seconds > now_seconds.saturating_add(MAX_LIVE_BLOCK_CLOCK_SKEW_SECS)
    {
        anyhow::bail!("live block timestamp exceeds the permitted clock skew");
    }
    Ok(())
}

/// Select the timestamp for a locally produced block.
///
/// Wall-clock time is preferred. If the parent is in the same or a later
/// second, advance exactly one whole second and then apply the live skew rule.
/// A parent already at the future-skew boundary therefore fails closed instead
/// of causing this node to extend an invalid future timeline.
pub fn next_live_timestamp(
    parent: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<DateTime<Utc>> {
    let now_whole = DateTime::from_timestamp(now.timestamp(), 0)
        .ok_or_else(|| anyhow::anyhow!("current Unix time is not representable"))?;
    let candidate = if now_whole > parent {
        now_whole
    } else {
        let next_second = parent
            .timestamp()
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("canonical parent timestamp cannot be advanced"))?;
        DateTime::from_timestamp(next_second, 0)
            .ok_or_else(|| anyhow::anyhow!("next block timestamp is not representable"))?
    };
    validate_live_timestamp(parent, candidate, now)?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64, nanos: u32) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, nanos).expect("valid test timestamp")
    }

    #[test]
    fn chain_time_requires_strict_parent_order_but_allows_historical_age() {
        let parent = at(1_700_000_000, 500_000_000);
        assert!(validate_chain_timestamp(parent, at(1_700_000_001, 0)).is_ok());
        assert!(validate_chain_timestamp(parent, parent).is_err());
        assert!(validate_chain_timestamp(parent, at(1_700_000_000, 0)).is_err());
    }

    #[test]
    fn live_time_requires_whole_seconds_and_bounded_clock_skew() {
        let now = at(1_900_000_000, 900_000_000);
        let parent = at(1_899_999_900, 0);
        assert!(validate_live_timestamp(parent, at(1_899_999_940, 0), now).is_ok());
        assert!(validate_live_timestamp(parent, at(1_900_000_060, 0), now).is_ok());
        assert!(validate_live_timestamp(parent, at(1_899_999_939, 0), now).is_err());
        assert!(validate_live_timestamp(parent, at(1_900_000_061, 0), now).is_err());
        assert!(
            validate_live_timestamp(parent, at(1_900_000_000, 1), now).is_err(),
            "sub-second spellings must not create alternate live block identities"
        );
    }

    #[test]
    fn local_timestamp_advances_parent_and_fails_closed_at_future_boundary() {
        let now = at(1_900_000_000, 900_000_000);
        assert_eq!(
            next_live_timestamp(at(1_899_999_999, 0), now).unwrap(),
            at(1_900_000_000, 0)
        );
        assert_eq!(
            next_live_timestamp(at(1_900_000_000, 500_000_000), now).unwrap(),
            at(1_900_000_001, 0)
        );
        assert!(
            next_live_timestamp(at(now.timestamp() + MAX_LIVE_BLOCK_CLOCK_SKEW_SECS, 0), now,)
                .is_err(),
            "a producer must not extend a parent already at the future-skew boundary"
        );
    }
}
