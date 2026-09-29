#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Vec as SorobanVec};

use crate::{proportional_share, renormalize_percentages, Allocation, Beneficiary};

prop_compose! {
    /// Strategy to generate a list of positive basis points that sum to exactly 10,000.
    fn basis_points_summing_to_10000()(
        count in 1usize..=20usize,
        weights in prop::collection::vec(1u32..=10_000u32, 1..=20)
    ) -> Vec<u32> {
        let actual_count = count.min(weights.len());
        let slice = &weights[..actual_count];
        let weight_sum: u64 = slice.iter().map(|&w| w as u64).sum();

        let mut bps = Vec::with_capacity(actual_count);
        let mut running = 0u32;
        for (i, &w) in slice.iter().enumerate() {
            if i == actual_count - 1 {
                bps.push(10_000 - running);
            } else {
                let share = ((w as u64 * 10_000) / weight_sum) as u32;
                let share = share.min(10_000 - running);
                bps.push(share);
                running += share;
            }
        }
        bps
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 100,
        ..ProptestConfig::default()
    })]

    /// Issue #292: Asserts that `proportional_share(total, bp)` summed across any set of
    /// basis points summing to 10,000 never exceeds `total`.
    #[test]
    fn test_proportional_share_sum_never_exceeds_total(
        total in 0i128..=i128::MAX,
        bps in basis_points_summing_to_10000()
    ) {
        let sum_shares: i128 = bps.iter().map(|&bp| proportional_share(total, bp)).sum();
        prop_assert!(sum_shares <= total, "Sum of shares {} exceeds total {}", sum_shares, total);
    }

    /// Issue #291: Asserts that `renormalize_percentages` always outputs percentage
    /// allocations that sum to exactly 10,000 basis points for any non-empty list of percentage beneficiaries.
    #[test]
    fn test_renormalize_percentages_sums_to_10000(
        bps in prop::collection::vec(1u32..=10_000u32, 1..=30)
    ) {
        let env = Env::default();
        let mut beneficiaries = SorobanVec::new(&env);
        for &bp in bps.iter() {
            beneficiaries.push_back(Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::Percentage(bp),
            });
        }

        let renormalized = renormalize_percentages(&env, &beneficiaries);
        let mut total_bp: u32 = 0;
        for b in renormalized.iter() {
            if let Allocation::Percentage(p) = b.allocation {
                total_bp = total_bp.saturating_add(p);
            }
        }

        prop_assert_eq!(total_bp, 10_000u32, "Renormalized sum {} != 10000", total_bp);
    }
}
