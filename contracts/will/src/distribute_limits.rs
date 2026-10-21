//! Share-rounding helpers for `distribute` (#599).
//!
//! The release path computes percentage shares as
//! `floor(remaining * bps / 10_000)`. For a small balance that floor is zero
//! for any beneficiary with a small allocation, so they would receive nothing
//! while the final percentage beneficiary absorbed their value.

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
pub(crate) fn min_one_unit_share(
    floor_share: i128,
    percentage_remaining: i128,
    later_count: u32,
) -> i128 {
    let cap = (percentage_remaining - later_count as i128).max(0);
    floor_share.max(1).min(cap)
}
