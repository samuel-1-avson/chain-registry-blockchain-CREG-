//! EIP-1559 fee selection and replace-by-fee bumping.

use crate::rpc::FeeEstimate;

/// Geth and most clients require a replacement to raise both fee fields by at
/// least 10%. The default bump keeps a margin above that.
pub const DEFAULT_FEE_BUMP_PERCENT: u32 = 13;

/// Operator fee ceilings for one hot wallet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeePolicy {
    /// Never sign a transaction whose `maxFeePerGas` exceeds this (wei).
    pub max_fee_per_gas_cap: u128,
    /// Never sign a transaction whose `maxPriorityFeePerGas` exceeds this (wei).
    pub max_priority_fee_per_gas_cap: u128,
    /// Percentage added to both fee fields on each replacement (>= 10).
    pub bump_percent: u32,
}

impl Default for FeePolicy {
    fn default() -> Self {
        Self {
            // 500 gwei / 50 gwei: far above Sepolia norms, a backstop against a
            // fee spike draining the hot wallet.
            max_fee_per_gas_cap: 500_000_000_000,
            max_priority_fee_per_gas_cap: 50_000_000_000,
            bump_percent: DEFAULT_FEE_BUMP_PERCENT,
        }
    }
}

impl FeePolicy {
    /// Fees for a first broadcast, or `None` when the network estimate is
    /// above the policy ceiling (the caller refuses to sign).
    pub fn initial(&self, estimate: FeeEstimate) -> Option<FeeEstimate> {
        if estimate.max_fee_per_gas > self.max_fee_per_gas_cap {
            return None;
        }
        let priority = estimate
            .max_priority_fee_per_gas
            .min(self.max_priority_fee_per_gas_cap)
            .min(estimate.max_fee_per_gas);
        Some(FeeEstimate {
            max_fee_per_gas: estimate.max_fee_per_gas,
            max_priority_fee_per_gas: priority,
        })
    }
}

fn raise(value: u128, percent: u32) -> u128 {
    // ceil(value * (100 + percent) / 100), saturating.
    let numerator = value.saturating_mul(100 + u128::from(percent));
    numerator.div_ceil(100).max(value.saturating_add(1))
}

/// Replacement fees for a stuck transaction: at least `bump_percent` above
/// the previous attempt on both fields, at least the current network
/// estimate, and never above the policy caps. Returns `None` when the caps
/// leave no room for a valid replacement (the caller keeps rebroadcasting the
/// last attempt instead).
pub fn bump_fees(
    previous: FeeEstimate,
    current: FeeEstimate,
    policy: &FeePolicy,
) -> Option<FeeEstimate> {
    let percent = policy.bump_percent.max(10);
    let mut max_fee = raise(previous.max_fee_per_gas, percent).max(current.max_fee_per_gas);
    let mut priority =
        raise(previous.max_priority_fee_per_gas, percent).max(current.max_priority_fee_per_gas);

    max_fee = max_fee.min(policy.max_fee_per_gas_cap);
    priority = priority
        .min(policy.max_priority_fee_per_gas_cap)
        .min(max_fee);

    // A replacement must raise both fields by the client's minimum (10%).
    let min_max_fee = raise(previous.max_fee_per_gas, 10);
    let min_priority = raise(previous.max_priority_fee_per_gas, 10);
    if max_fee < min_max_fee || priority < min_priority {
        return None;
    }
    Some(FeeEstimate {
        max_fee_per_gas: max_fee,
        max_priority_fee_per_gas: priority,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GWEI: u128 = 1_000_000_000;

    fn fees(max_fee: u128, priority: u128) -> FeeEstimate {
        FeeEstimate {
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: priority,
        }
    }

    #[test]
    fn initial_fees_respect_caps() {
        let policy = FeePolicy {
            max_fee_per_gas_cap: 100 * GWEI,
            max_priority_fee_per_gas_cap: 2 * GWEI,
            bump_percent: 13,
        };
        assert_eq!(
            policy.initial(fees(30 * GWEI, 5 * GWEI)),
            Some(fees(30 * GWEI, 2 * GWEI))
        );
        assert_eq!(policy.initial(fees(101 * GWEI, GWEI)), None);
        // Priority can never exceed the max fee.
        assert_eq!(policy.initial(fees(GWEI, 3 * GWEI)), Some(fees(GWEI, GWEI)));
    }

    #[test]
    fn bump_raises_both_fields_by_at_least_ten_percent() {
        let policy = FeePolicy::default();
        let previous = fees(20 * GWEI, 2 * GWEI);
        let bumped = bump_fees(previous, fees(10 * GWEI, GWEI), &policy).expect("room to bump");
        assert!(bumped.max_fee_per_gas * 100 >= previous.max_fee_per_gas * 110);
        assert!(bumped.max_priority_fee_per_gas * 100 >= previous.max_priority_fee_per_gas * 110);
        assert!(bumped.max_priority_fee_per_gas <= bumped.max_fee_per_gas);
    }

    #[test]
    fn bump_follows_a_rising_network_estimate() {
        let policy = FeePolicy::default();
        let bumped = bump_fees(
            fees(20 * GWEI, 2 * GWEI),
            fees(80 * GWEI, 9 * GWEI),
            &policy,
        )
        .expect("room to bump");
        assert_eq!(bumped, fees(80 * GWEI, 9 * GWEI));
    }

    #[test]
    fn bump_stops_at_the_cap() {
        let policy = FeePolicy {
            max_fee_per_gas_cap: 21 * GWEI,
            max_priority_fee_per_gas_cap: 5 * GWEI,
            bump_percent: 13,
        };
        // 20 -> 22.6 gwei would exceed the 21 gwei cap and 21 is below +10%.
        assert_eq!(
            bump_fees(fees(20 * GWEI, GWEI), fees(GWEI, GWEI), &policy),
            None
        );
    }

    #[test]
    fn bump_handles_zero_and_tiny_fees() {
        let policy = FeePolicy::default();
        let bumped = bump_fees(fees(1, 0), fees(0, 0), &policy).expect("room to bump");
        assert!(bumped.max_fee_per_gas >= 2);
        assert!(bumped.max_priority_fee_per_gas >= 1);
    }
}
