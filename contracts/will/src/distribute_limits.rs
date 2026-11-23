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

/// Basis-point denominator used for percentage share math.
pub(crate) const BPS_DENOMINATOR: i128 = 10_000;

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

/// Divides `numerator` by `divisor`, rejecting a zero divisor with a clear
/// contract error instead of panicking on the raw division (#802).
///
/// Returns `None` when the divisor is zero so callers can surface
/// [`WillError::DivisionByZero`] through their own error path; use
/// [`checked_div_or_error`] to panic with that error directly.
pub(crate) fn checked_div(numerator: i128, divisor: i128) -> Option<i128> {
    if divisor == 0 {
        return None;
    }
    numerator.checked_div(divisor)
}

/// Divides `numerator` by `divisor`, panicking with
/// [`WillError::DivisionByZero`] when the divisor is zero (#802).
pub(crate) fn checked_div_or_error(env: &Env, numerator: i128, divisor: i128) -> i128 {
    match checked_div(numerator, divisor) {
        Some(quotient) => quotient,
        None => panic_with_error!(env, WillError::DivisionByZero),
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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn checked_div_returns_none_on_zero_divisor() {
        assert_eq!(checked_div(100, 0), None);
    }

    #[test]
    fn checked_div_divides_normally() {
        assert_eq!(checked_div(100, 4), Some(25));
        assert_eq!(checked_div(0, 4), Some(0));
    }

    #[test]
    fn checked_div_or_error_panics_on_zero_divisor() {
        let env = Env::default();
        let result = env.try_invoke_contract::<i128, WillError, _>(
            &env.register_contract(None, ZeroDivProbe),
            &soroban_sdk::symbol_short!("probe"),
            soroban_sdk::vec![&env],
        );
        assert!(result.is_err());
    }

    #[test]
    fn checked_div_or_error_divides_normally() {
        let env = Env::default();
        assert_eq!(checked_div_or_error(&env, 100, 4), 25);
    }

    #[soroban_sdk::contract]
    struct ZeroDivProbe;

    #[soroban_sdk::contractimpl]
    impl ZeroDivProbe {
        pub fn probe(env: Env) -> i128 {
            checked_div_or_error(&env, 1, 0)
        }
    }
}
