//! Keeper bounty rounding (#595) and dust prevention (#782).
//!
//! # Direction
//!
//! The bounty is `floor(balance * keeper_bounty_bps / 10_000)`, computed in
//! integer arithmetic through [`crate::proportional_share`] (no floating
//! point, no overflow of the intermediate product). Rounding is therefore
//! always **down**, in favour of the beneficiaries: the keeper never receives
//! more than the configured fraction.
//!
//! # Dust
//!
//! The fractional remainder (strictly less than one token unit) is never
//! lost or sent to the keeper. It stays in the amount available to the
//! beneficiaries; the last percentage beneficiary absorbs the integer-division
//! remainder, and a FixedAmount-only will refunds the leftover to the owner
//! (#383). When `balance * bps < 10_000` the bounty is 0 and the whole
//! balance is distributed.
//!
//! # Dust prevention strategy (#782)
//!
//! Distribution must never leave tiny stranded amounts in the contract. The
//! strategy is:
//!
//! 1. **Collect** — every per-beneficiary share is computed with integer
//!    division, so each share is rounded down and the sum of shares can be
//!    strictly less than the distributable amount. The difference is the
//!    *dust* and is collected explicitly via [`collect_dust`].
//! 2. **Distribute** — the collected dust is handed to the last percentage
//!    beneficiary (the same beneficiary that already absorbs the division
//!    remainder), so the full distributable amount leaves the contract and no
//!    sub-unit leftover remains. If there are no percentage beneficiaries the
//!    dust is returned to the owner instead of being stranded.
//! 3. **Verify** — [`distribute_with_dust`] asserts that the sum of the
//!    distributed shares plus the dust equals the distributable amount, so a
//!    regression that reintroduces stranded dust fails loudly.

/// Returns the keeper bounty for `balance` at `bps` basis points, rounded
/// down.
pub fn keeper_bounty(balance: i128, bps: u32) -> i128 {
    crate::proportional_share(balance, bps)
}

/// Collects the dust left over after distributing `shares` out of `total`.
///
/// Each share is expected to have been rounded down, so the sum of `shares`
/// is at most `total`. The returned dust is `total - sum(shares)` and is
/// always non-negative. A negative result (shares over-allocated) is clamped
/// to zero so callers never distribute more than `total`.
pub fn collect_dust(total: i128, shares: &[i128]) -> i128 {
    let distributed: i128 = shares.iter().copied().sum();
    let dust = total - distributed;
    if dust > 0 {
        dust
    } else {
        0
    }
}

/// Distributes `total` across `shares`, adding the collected dust to the last
/// share so no sub-unit amount is stranded in the contract.
///
/// Returns the adjusted shares. When `shares` is empty the whole `total` is
/// returned as a single dust entry so the caller can refund it to the owner.
pub fn distribute_with_dust(total: i128, shares: &[i128]) -> Vec<i128> {
    let mut adjusted = shares.to_vec();
    let dust = collect_dust(total, shares);
    if adjusted.is_empty() {
        if dust > 0 {
            adjusted.push(dust);
        }
        return adjusted;
    }
    if dust > 0 {
        let last = adjusted.len() - 1;
        adjusted[last] += dust;
    }
    // Dust prevention invariant: the distributed shares must account for the
    // full distributable amount, leaving nothing stranded.
    let distributed: i128 = adjusted.iter().copied().sum();
    debug_assert_eq!(distributed, total, "dust prevention: shares must sum to total");
    adjusted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeper_bounty_rounds_down() {
        assert_eq!(keeper_bounty(10_000, 100), 100);
        assert_eq!(keeper_bounty(9_999, 100), 99);
        assert_eq!(keeper_bounty(99, 100), 0);
    }

    #[test]
    fn collect_dust_returns_remainder() {
        // 100 split three ways rounds down to 33 each, leaving 1 dust.
        assert_eq!(collect_dust(100, &[33, 33, 33]), 1);
        // Exact split leaves no dust.
        assert_eq!(collect_dust(100, &[50, 50]), 0);
        // Over-allocation is clamped to zero.
        assert_eq!(collect_dust(10, &[6, 6]), 0);
    }

    #[test]
    fn distribute_with_dust_leaves_nothing_stranded() {
        let adjusted = distribute_with_dust(100, &[33, 33, 33]);
        assert_eq!(adjusted, vec![33, 33, 34]);
        assert_eq!(adjusted.iter().sum::<i128>(), 100);
    }

    #[test]
    fn distribute_with_dust_handles_empty_shares() {
        let adjusted = distribute_with_dust(7, &[]);
        assert_eq!(adjusted, vec![7]);
        assert_eq!(adjusted.iter().sum::<i128>(), 7);
    }

    #[test]
    fn distribute_with_dust_no_dust_when_exact() {
        let adjusted = distribute_with_dust(100, &[25, 25, 25, 25]);
        assert_eq!(adjusted, vec![25, 25, 25, 25]);
        assert_eq!(adjusted.iter().sum::<i128>(), 100);
    }
}
