//! Share-rounding helpers for `distribute` (#599, #782).
//!
//! The release path computes percentage shares as
//! `floor(remaining * bps / 10_000)`. For a small balance that floor is zero
//! for any beneficiary with a small allocation, so they would receive nothing
//! while the final percentage beneficiary absorbed their value.
//!
//! ## Dust prevention strategy (#782)
//!
//! "Dust" is any leftover value that would otherwise be stranded or swept
//! into the final percentage beneficiary purely because of integer rounding.
//! The strategy is:
//!
//! 1. **Collect** — every non-final percentage beneficiary is paid at least
//!    one unit via [`min_one_unit_share`], so rounding dust is never silently
//!    dropped from a small allocation.
//! 2. **Distribute** — the final percentage beneficiary absorbs the exact
//!    remainder (`percentage_remaining`), which is the sum of all rounding
//!    dust. Because the plan sums exactly to the balance, no tiny leftover
//!    remains after distribution.
//! 3. **Bound** — [`min_one_unit_share`] never over-allocates: it leaves one
//!    unit for each later beneficiary, so the remainder stays non-negative and
//!    the final absorber always has a well-defined share.
//!
//! Together these guarantee that the distributed total equals the balance
//! exactly, with no stranded dust.

use soroban_sdk::{panic_with_error, Env};

use crate::errors::WillError;
use crate::MAX_RELEASE_PAYOUTS;

/// Pre-flight check run at the top of `distribute` (#597): rejects a release
/// whose beneficiary x token payout count exceeds [`MAX_RELEASE_PAYOUTS`]
/// before any state changes or transfers happen. Every entry point already
/// enforces the per-list caps, but `distribute` is also reached through the
/// guardian-quorum path and by wills written under an older layout, so the
/// bound is asserted here as well. See `docs/DISTRIBUTE_RESOURCES.md`.
pub(crate) fn assert_payout_budget(env: &Env, beneficiaries: u32, tokens: u32) {
    let payouts = match beneficiaries.checked_mul(tokens) {
        Some(n) => n,
        None => panic_with_error!(env, WillError::DistributionTooLarge),
    };
    if payouts > MAX_RELEASE_PAYOUTS {
        panic_with_error!(env, WillError::DistributionTooLarge);
    }
}

/// Returns the share a non-final percentage beneficiary is paid.
///
/// - `floor_share`: `floor(remaining * bps / 10_000)` for this beneficiary.
/// - `percentage_remaining`: the amount not yet assigned to any percentage
///   beneficiary.
/// - `later_count`: how many percentage beneficiaries are still to be paid
///   after this one, including the final remainder absorber.
///
/// The result is at least one unit, and never more than leaves one unit for
/// each later beneficiary, so the plan still sums exactly to the balance. When
/// the balance is too small to give every beneficiary a unit, the later
/// beneficiaries receive what is left.
///
/// This is the dust-collection step of the dust prevention strategy (#782):
/// rounding dust is folded into the final absorber rather than stranded.
pub(crate) fn min_one_unit_share(
    floor_share: i128,
    percentage_remaining: i128,
    later_count: u32,
) -> i128 {
    let cap = (percentage_remaining - later_count as i128).max(0);
    floor_share.max(1).min(cap)
}

/// Returns the dust that the final percentage beneficiary must absorb so the
/// plan sums exactly to the balance (#782).
///
/// `percentage_remaining` is the amount not yet assigned after all non-final
/// percentage beneficiaries have been paid; it is exactly the accumulated
/// rounding dust plus the final beneficiary's own floor share. Returning it
/// unchanged documents that the final absorber collects all dust, leaving no
/// stranded remainder.
pub(crate) fn final_dust_share(percentage_remaining: i128) -> i128 {
    percentage_remaining.max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_one_unit_share_never_strands_dust() {
        // A tiny allocation that floors to zero still gets one unit.
        assert_eq!(min_one_unit_share(0, 100, 1), 1);
        // The share never exceeds what leaves a unit for later beneficiaries.
        assert_eq!(min_one_unit_share(50, 10, 3), 7);
        // When the balance is too small, later beneficiaries get what is left.
        assert_eq!(min_one_unit_share(0, 0, 2), 0);
    }

    #[test]
    fn plan_sums_exactly_to_balance() {
        // Three beneficiaries at 33/33/34 bps of 100 units.
        let balance: i128 = 100;
        let mut remaining = balance;
        let mut paid: i128 = 0;

        // First two non-final beneficiaries.
        for _ in 0..2 {
            let floor_share = remaining * 3_300 / 10_000;
            let share = min_one_unit_share(floor_share, remaining, 1);
            paid += share;
            remaining -= share;
        }
        // Final absorber collects the dust.
        let dust = final_dust_share(remaining);
        paid += dust;
        remaining -= dust;

        assert_eq!(paid, balance);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn final_dust_share_is_non_negative() {
        assert_eq!(final_dust_share(0), 0);
        assert_eq!(final_dust_share(7), 7);
        assert_eq!(final_dust_share(-3), 0);
    }
}
