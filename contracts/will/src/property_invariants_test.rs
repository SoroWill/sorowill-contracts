#![cfg(test)]

//! Property-based tests for issue #742: contract-level invariants checked over
//! randomly generated wills.
//!
//! Each case runs the full lifecycle (create, trigger, release) on a fresh
//! Soroban environment, so these are slower than the pure-math properties in
//! `proptest_math.rs`. The case count is kept at [`CASES`] to bound runtime.
//! Raise it locally with `PROPTEST_CASES=<n>` for a deeper run.
//!
//! Invariants checked:
//! - **Beneficiaries are always distributed:** the payouts sum to the locked
//!   amount, and nothing is left in the contract or refunded to the owner.
//! - **Percentages match:** the stored percentage allocations total exactly
//!   10,000 basis points.
//! - **No duplicate distributions:** every beneficiary receives at least its
//!   floor share, and a second release is rejected, so funds cannot be paid out
//!   twice.

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;
const T0: u64 = 1_700_000_000;
const CASES: u32 = 64;
const MAX_BENEFICIARIES: usize = 8;

/// Splits 10,000 basis points across `weights.len()` beneficiaries.
///
/// Every entry is at least 1 bp, so no beneficiary is given a zero share. The
/// remainder from integer division is added to the last entry, so the result
/// sums to exactly 10,000.
fn bps_from_weights(weights: &[u32]) -> Vec<u32> {
    let n = weights.len() as u32;
    let weight_sum: u64 = weights.iter().map(|&w| w as u64).sum();
    let spare = (10_000 - n) as u64;
    let mut bps: Vec<u32> = weights
        .iter()
        .map(|&w| 1 + ((w as u64 * spare) / weight_sum) as u32)
        .collect();
    let assigned: u32 = bps.iter().sum();
    *bps.last_mut().expect("at least one weight") += 10_000 - assigned;
    bps
}

/// Generates a percentage split and a locked amount. Amounts are large enough
/// that every share is non-zero.
fn scenario() -> impl Strategy<Value = (Vec<u32>, i128)> {
    (
        prop::collection::vec(1u32..=10_000u32, 1..=MAX_BENEFICIARIES),
        1_000_000i128..=1_000_000_000_000i128,
    )
        .prop_map(|(weights, total)| (bps_from_weights(&weights), total))
}

/// A will that has been created, triggered, and released.
struct Released<'a> {
    client: WillContractClient<'a>,
    token: TokenClient<'a>,
    owner: Address,
    will_id: u64,
    beneficiaries: Vec<Address>,
    total: i128,
}

fn create_and_release(bps: &[u32], total: i128) -> Released<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &total);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiaries: Vec<Address> = bps.iter().map(|_| Address::generate(&env)).collect();
    let mut spec: SorobanVec<Beneficiary> = SorobanVec::new(&env);
    for (address, &bp) in beneficiaries.iter().zip(bps.iter()) {
        spec.push_back(Beneficiary {
            address: address.clone(),
            allocation: Allocation::Percentage(bp),
        });
    }

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), total)],
        &spec,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Past the check-in deadline, then trigger.
    env.ledger().set_timestamp(T0 + 91 * DAY);
    client.trigger_will(&will_id);
    // Past the grace period, then release.
    env.ledger().set_timestamp(T0 + 99 * DAY);
    client.release_inheritance(&will_id, &None);

    let token = TokenClient::new(&env, &token_address);
    Released {
        client,
        token,
        owner,
        will_id,
        beneficiaries,
        total,
    }
}

impl Released<'_> {
    fn received(&self) -> Vec<i128> {
        self.beneficiaries
            .iter()
            .map(|a| self.token.balance(a))
            .collect()
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: CASES,
        ..ProptestConfig::default()
    })]

    /// Every locked unit reaches a beneficiary. Nothing stays in the contract
    /// and nothing is refunded to the owner, because the percentages cover the
    /// whole balance.
    #[test]
    fn beneficiaries_are_always_fully_distributed((bps, total) in scenario()) {
        let r = create_and_release(&bps, total);

        let paid: i128 = r.received().iter().sum();
        prop_assert_eq!(paid, total, "payouts must sum to the locked total");
        prop_assert_eq!(
            r.token.balance(&r.client.address), 0,
            "no balance may remain in the contract"
        );
        prop_assert_eq!(
            r.token.balance(&r.owner), 0,
            "no refund is due when percentages cover the whole balance"
        );
        prop_assert_eq!(r.client.get_will(&r.will_id).balance, 0);
    }

    /// The stored percentage allocations total exactly 10,000 basis points,
    /// and every beneficiary has a non-zero share.
    #[test]
    fn percentages_total_exactly_10000((bps, total) in scenario()) {
        let r = create_and_release(&bps, total);

        let will = r.client.get_will(&r.will_id);
        let mut sum: u32 = 0;
        for b in will.beneficiaries.iter() {
            match b.allocation {
                Allocation::Percentage(bp) => {
                    prop_assert!(bp > 0, "a percentage share must be non-zero");
                    sum += bp;
                }
                _ => prop_assert!(false, "expected percentage allocations only"),
            }
        }
        prop_assert_eq!(sum, 10_000);
    }

    /// Each beneficiary receives at least its floor share, and the total paid
    /// is the locked amount. Together these mean the remainder was paid once,
    /// not twice. A second release is then rejected, so no payout can repeat.
    #[test]
    fn no_duplicate_distributions((bps, total) in scenario()) {
        let r = create_and_release(&bps, total);

        for (received, &bp) in r.received().iter().zip(bps.iter()) {
            let floor_share = total * bp as i128 / 10_000;
            prop_assert!(
                *received >= floor_share,
                "beneficiary received {} which is below its floor share {}",
                received,
                floor_share
            );
            prop_assert!(*received > 0);
        }
        let paid: i128 = r.received().iter().sum();
        prop_assert_eq!(paid, r.total);

        // A second release must not pay anything out again.
        let second = r.client.try_release_inheritance(&r.will_id, &None);
        prop_assert!(second.is_err(), "a released will must not release again");
        let paid_after: i128 = r.received().iter().sum();
        prop_assert_eq!(paid_after, r.total, "a rejected release must not move funds");
    }
}
