#![cfg(test)]

//! Property-based fuzzing of `create_will` and `update_beneficiaries`.
//!
//! These tests drive the same runners as the `cargo-fuzz` targets under
//! `fuzz/` (see [`crate::fuzz_harness`]), but with `proptest` supplying the
//! input. That keeps the invariants enforced by ordinary `cargo test` — and so
//! by CI — on stable Rust, without anyone needing a nightly toolchain.
//!
//! `cargo-fuzz` explores much deeper; `proptest` here is the regression net.
//! Every bug the fuzzer has actually found is additionally pinned by a
//! hand-written test at the bottom of this file, so a regression fails loudly
//! and deterministically rather than waiting for a lucky random draw.

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::fuzz_harness::{
    run_create_will, run_update_beneficiaries, sanitize_specs, BeneficiarySpec, CreateWillInput,
    Outcome, UpdateBeneficiariesInput, MAX_FUZZ_UPDATES,
};
use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

/// Basis-point shares worth trying: the valid range and extremes.
fn basis_points_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        4 => 0u32..=10_001,
        1 => Just(0u32),
        1 => Just(10_000u32),
        1 => Just(u32::MAX),
        1 => Just(u32::MAX / 2),
        1 => Just(u32::MAX - 1),
        1 => any::<u32>(),
    ]
}

fn beneficiary_spec_strategy() -> impl Strategy<Value = BeneficiarySpec> {
    (any::<u8>(), basis_points_strategy()).prop_map(|(address_slot, basis_points)| {
        BeneficiarySpec {
            address_slot,
            basis_points,
        }
    })
}

/// Lists that straddle the 1..=10 limit on both sides, including empty.
fn beneficiaries_strategy() -> impl Strategy<Value = Vec<BeneficiarySpec>> {
    prop::collection::vec(beneficiary_spec_strategy(), 0..13)
}

/// Periods worth trying: zero, sane values, the exact overflow threshold for
/// `days * 86_400`, and the extremes beyond it.
fn period_strategy() -> impl Strategy<Value = u64> {
    prop_oneof![
        4 => 0u64..=4_000,
        1 => Just(0u64),
        1 => Just(u64::MAX),
        1 => Just(u64::MAX / 86_400),
        1 => Just(u64::MAX / 86_400 + 1),
        1 => any::<u64>(),
    ]
}

/// Amounts worth trying: zero, negatives, ordinary values, and amounts far
/// beyond what the owner was funded with.
fn amount_strategy() -> impl Strategy<Value = i128> {
    prop_oneof![
        4 => 1i128..=1_000_000_000,
        1 => Just(0i128),
        1 => Just(-1i128),
        1 => Just(i128::MIN),
        1 => Just(i128::MAX),
        1 => any::<i128>(),
    ]
}

fn create_will_input_strategy() -> impl Strategy<Value = CreateWillInput> {
    (
        amount_strategy(),
        amount_strategy(),
        beneficiaries_strategy(),
        prop::collection::vec(any::<u8>(), 0..6),
        period_strategy(),
        period_strategy(),
        1u32..=5,
        any::<bool>(),
    )
        .prop_map(
            |(
                mint,
                amount,
                beneficiaries,
                guardian_slots,
                checkin_period_days,
                grace_period_days,
                guardian_threshold,
                release_after_create,
            )| CreateWillInput {
                mint,
                amount,
                beneficiaries,
                guardian_slots,
                checkin_period_days,
                grace_period_days,
                guardian_threshold,
                release_after_create,
            },
        )
}

/// Generates between one and MAX_BENEFICIARIES positive shares whose sum is
/// exactly 10,000 basis points. Random weights are normalized after reserving
/// one basis point per beneficiary; truncation dust goes to the last entry.
fn valid_basis_point_split_strategy() -> impl Strategy<Value = Vec<u32>> {
    (1usize..=crate::MAX_BENEFICIARIES as usize).prop_flat_map(|count| {
        prop::collection::vec(1u32..=10_000, count).prop_map(move |weights| {
            let weight_total: u64 = weights.iter().map(|weight| *weight as u64).sum();
            let distributable = 10_000u64 - count as u64;
            let mut allocated = 0u32;
            let mut shares = Vec::with_capacity(count);

            for weight in weights.iter().take(count - 1) {
                let share = 1 + (distributable * *weight as u64 / weight_total) as u32;
                shares.push(share);
                allocated += share;
            }
            shares.push(10_000 - allocated);
            shares
        })
    })
}

fn update_beneficiaries_input_strategy() -> impl Strategy<Value = UpdateBeneficiariesInput> {
    (
        create_will_input_strategy(),
        prop::collection::vec(beneficiaries_strategy(), 0..5),
        any::<bool>(),
    )
        .prop_map(
            |(create, updates, probe_non_owner)| UpdateBeneficiariesInput {
                create,
                updates,
                probe_non_owner,
            },
        )
}

// Each case registers a fresh Soroban env plus two contracts, so the case
// counts are kept modest to hold the suite to a few seconds. Deep exploration
// is cargo-fuzz's job; see docs/FUZZING.md.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 512,
        ..ProptestConfig::default()
    })]

    /// `create_will` must reject malformed input with a declared `WillError`,
    /// never by aborting, and every will it accepts must satisfy the
    /// invariants documented on `Will`.
    #[test]
    fn create_will_upholds_invariants(input in create_will_input_strategy()) {
        if let Outcome::Accepted(will_id) = run_create_will(&input) {
            // Ids are allocated from a counter that starts at 1; a zero id
            // would mean `get_will` could never find the will again.
            prop_assert!(will_id >= 1);
        }
    }

    /// Every valid beneficiary split must transfer the complete token balance.
    /// The public release path exercises `distribute` and its final-recipient
    /// remainder handling exactly as production does.
    #[test]
    fn distribution_conserves_every_valid_balance(
        basis_points in valid_basis_point_split_strategy(),
        balance in 1i128..=1_000_000_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);

        let owner = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(owner.clone());
        let token_address = sac.address();
        StellarAssetClient::new(&env, &token_address).mint(&owner, &balance);
        let token = TokenClient::new(&env, &token_address);

        let contract_id = env.register(WillContract, ());
        let client = WillContractClient::new(&env, &contract_id);
        let mut beneficiaries = SorobanVec::new(&env);
        let mut beneficiary_addresses = Vec::with_capacity(basis_points.len());
        for share in basis_points {
            let address = Address::generate(&env);
            beneficiary_addresses.push(address.clone());
            beneficiaries.push_back(Beneficiary {
                address,
                allocation: Allocation::Percentage(share),
            });
        }

        let will_id = client.create_will(
            &owner,
            &vec![&env, (token_address, balance)],
            &beneficiaries,
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        );
        env.ledger().set_timestamp(1_700_000_000 + 91 * 86_400);
        client.trigger_will(&will_id);
        env.ledger().set_timestamp(1_700_000_000 + 99 * 86_400);
        client.release_inheritance(&will_id, &None);

        let transferred: i128 = beneficiary_addresses
            .iter()
            .map(|address| token.balance(address))
            .sum();
        prop_assert_eq!(transferred, balance);
        prop_assert_eq!(token.balance(&client.address), 0);
        prop_assert!(client.get_will(&will_id).balances.is_empty());
    }

    /// `update_beneficiaries` must likewise never abort, must leave the will
    /// untouched when it rejects an update, and must keep the beneficiary
    /// reverse index in step with the list it stores.
    #[test]
    fn update_beneficiaries_upholds_invariants(input in update_beneficiaries_input_strategy()) {
        let outcomes = run_update_beneficiaries(&input);
        // One outcome per replacement list actually applied.
        prop_assert!(outcomes.len() <= input.updates.len());
    }

    /// Issue #27-style invariant test: `assert_valid_percentages` is called
    /// from both `create_will` and `update_beneficiaries`, but nothing
    /// previously proved that, across an arbitrary *sequence* of operations,
    /// a will's beneficiary percentages can never drift away from summing to
    /// exactly 100% (10,000 basis points).
    ///
    /// Every replacement list here is built with `sanitize_specs`, the same
    /// helper `sanitize_create` uses, so every `update_beneficiaries` call in
    /// the sequence is valid-by-construction and therefore guaranteed to be
    /// `Accepted`. `run_update_beneficiaries` reloads the will with `get_will`
    /// after every accepted call and checks the stored basis points sum to
    /// 10,000 (see `assert_updated_beneficiaries` in `fuzz_harness`) — so
    /// asserting every outcome is `Accepted` here proves that check actually
    /// ran after each and every operation in the sequence, not just the last.
    #[test]
    fn percentages_never_drift_across_valid_operation_sequences(
        create in create_will_input_strategy(),
        raw_updates in prop::collection::vec(beneficiaries_strategy(), 1..6),
        probe_non_owner in any::<bool>(),
    ) {
        let updates: Vec<Vec<BeneficiarySpec>> = raw_updates
            .iter()
            .map(|specs| sanitize_specs(specs))
            .collect();
        let input = UpdateBeneficiariesInput {
            create,
            updates,
            probe_non_owner,
        };

        let outcomes = run_update_beneficiaries(&input);
        // The harness caps replays at MAX_FUZZ_UPDATES per scenario.
        prop_assert_eq!(outcomes.len(), input.updates.len().min(MAX_FUZZ_UPDATES));
        for outcome in outcomes.iter() {
            prop_assert!(
                matches!(outcome, Outcome::Accepted(_)),
                "a valid-by-construction update was rejected: {:?}",
                outcome,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Regression tests for the defects this harness found.
// ---------------------------------------------------------------------------

/// Sets up a will contract with a funded owner, mirroring `test::setup` but
/// returning only what the regression tests below need.
fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

fn single_beneficiary(env: &Env, basis_points: u32) -> SorobanVec<Beneficiary> {
    vec![
        env,
        Beneficiary {
            address: Address::generate(env),
            allocation: crate::Allocation::Percentage(basis_points),
        },
    ]
}

/// Two huge basis-point values used to overflow the running total in
/// `assert_valid_allocations`, aborting the contract instead of returning
/// `InvalidPercentages`.
#[test]
fn create_will_rejects_overflowing_basis_points() {
    let (env, client, owner, token) = setup();
    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: crate::Allocation::Percentage(u32::MAX),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: crate::Allocation::Percentage(u32::MAX),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0
        ),
        Err(Ok(WillError::InvalidPercentages.into()))
    );
}

/// A basis-point sum that does not equal 10,000 must be rejected.
#[test]
fn create_will_rejects_non_10000_basis_points() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 9_999);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0
        ),
        Err(Ok(WillError::InvalidPercentages.into()))
    );
}

/// A check-in period beyond `u64::MAX / 86_400` used to overflow while
/// computing the deadline for the `WillCreated` event.
#[test]
fn create_will_rejects_overflowing_checkin_period() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];
    let overflowing = u64::MAX / 86_400 + 1;

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &overflowing,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidPeriod.into()))
    );
}

/// A grace period that cannot be converted to a timestamp would leave the
/// balance unreleasable, since `release_inheritance` overflows on every call.
#[test]
fn create_will_rejects_overflowing_grace_period() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &u64::MAX,
            &vec![&env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidPeriod.into()))
    );
}

/// A zero-day check-in period makes the will triggerable in the very ledger it
/// was created in, which defeats the dead man's switch.
#[test]
fn create_will_rejects_zero_length_periods() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &0,
            &7,
            &vec![&env],
            &2,
            &None,
            &0
        ),
        Err(Ok(WillError::InvalidPeriod.into()))
    );
    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &0,
            &vec![&env],
            &2,
            &None,
            &0
        ),
        Err(Ok(WillError::InvalidPeriod.into()))
    );
}

/// The longest allowed periods must still be accepted.
#[test]
fn create_will_accepts_maximum_periods() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &crate::MAX_PERIOD_DAYS,
        &crate::MAX_PERIOD_DAYS,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    assert_eq!(
        client.get_will(&will_id).checkin_period_days,
        crate::MAX_PERIOD_DAYS
    );
}

/// A guardian list of `[g, g]` looks like a 2-of-2 quorum but can only ever
/// reach one vote, so the guardian override could never fire.
#[test]
fn create_will_rejects_duplicate_guardians() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];
    let guardian = Address::generate(&env);

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &90,
            &7,
            &vec![&env, guardian.clone(), guardian],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::DuplicateGuardian.into()))
    );
}

/// `update_guardians` shares the validation, so it must reject duplicates too.
#[test]
fn update_guardians_rejects_duplicate_guardians() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    assert_eq!(
        client.try_update_guardians(&will_id, &owner, &vec![&env, guardian.clone(), guardian]),
        Err(Ok(WillError::DuplicateGuardian.into()))
    );
}

/// The same overflow reachable through `create_will` is reachable through
/// `update_beneficiaries`, which validates the replacement list independently.
#[test]
fn update_beneficiaries_rejects_overflowing_basis_points() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let replacement = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: crate::Allocation::Percentage(u32::MAX),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: crate::Allocation::Percentage(1),
        },
    ];

    assert_eq!(
        client.try_update_beneficiaries(&will_id, &owner, &replacement),
        Err(Ok(WillError::InvalidPercentages.into()))
    );
    // The rejected update must not have disturbed the stored list.
    assert_eq!(client.get_will(&will_id).beneficiaries, beneficiaries);
}

/// A checkin period one day above the maximum must still be rejected, not
/// just values large enough to overflow the deadline arithmetic.
#[test]
fn create_will_rejects_period_just_above_maximum() {
    let (env, client, owner, token) = setup();
    let beneficiaries = single_beneficiary(&env, 10_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token.clone(), 1_000_000_i128)];

    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens,
            &beneficiaries,
            &(crate::MAX_PERIOD_DAYS + 1),
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidPeriod.into()))
    );
}
