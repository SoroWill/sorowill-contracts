//! Rounding tests for beneficiary percentage distribution (#729).
//!
//! These tests call the pure `proportional_share` helper directly. They check
//! the floor rule and the remainder bound that `distribute` relies on. The
//! rules are documented in `docs/BENEFICIARY_PRECISION.md`.

use crate::proportional_share;

#[test]
fn full_share_returns_whole_balance() {
    assert_eq!(proportional_share(10_000, 10_000), 10_000);
}

#[test]
fn half_share_of_even_balance_is_exact() {
    assert_eq!(proportional_share(10_000, 5_000), 5_000);
}

#[test]
fn share_below_one_unit_floors_to_zero() {
    // 9 * 1000 / 10000 = 0.9, which floors to 0.
    assert_eq!(proportional_share(9, 1_000), 0);
}

#[test]
fn fractional_share_floors_not_rounds() {
    // 10001 * 3333 / 10000 = 3333.3333, which floors to 3333.
    assert_eq!(proportional_share(10_001, 3_333), 3_333);
}

#[test]
fn large_balance_does_not_overflow_and_is_exact_at_full_share() {
    let total: i128 = 1_000_000_007;
    assert_eq!(proportional_share(total, 10_000), total);
}

#[test]
fn last_beneficiary_absorbs_rounding_remainder() {
    // Balance 10,007 split 3,333 / 3,333 / 3,334 bp.
    // Floored shares: 3,335, 3,335, 3,336 (sum 10,006), so one unit is left.
    // The last beneficiary takes the remainder, giving 3,337.
    let total: i128 = 10_007;
    let first = proportional_share(total, 3_333);
    let second = proportional_share(total, 3_333);
    let last_floor = proportional_share(total, 3_334);
    assert_eq!(first, 3_335);
    assert_eq!(second, 3_335);
    assert_eq!(last_floor, 3_336);

    let last = total - first - second;
    assert_eq!(last, 3_337);
    assert!(last >= last_floor, "absorbed remainder must not be negative");
    assert_eq!(first + second + last, total);
}

#[test]
fn floored_shares_leave_less_than_one_unit_per_beneficiary() {
    // Four equal shares. Over a range of balances, the dust left by flooring
    // must stay below the number of beneficiaries, so the final beneficiary's
    // remainder is always small and non-negative.
    let bps = [2_500u32, 2_500, 2_500, 2_500];
    for total in 0i128..=2_000 {
        let mut distributed: i128 = 0;
        for &b in bps.iter() {
            distributed += proportional_share(total, b);
        }
        let dust = total - distributed;
        assert!(dust >= 0, "floored shares must never exceed the balance");
        assert!(
            dust < bps.len() as i128,
            "dust {} must be less than the beneficiary count for total {}",
            dust,
            total
        );
    }
}

#[test]
fn zero_balance_pays_zero_for_every_share() {
    for bp in [1u32, 2_500, 5_000, 10_000] {
        assert_eq!(proportional_share(0, bp), 0);
    }
}
