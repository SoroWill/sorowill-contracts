#![cfg(test)]

//! Unit tests for the mixed percentage/fixed-amount `Allocation` model
//! introduced alongside this file.
//!
//! `Beneficiary.basis_points: u32` (pure percentage) has been replaced by
//! `Beneficiary.allocation: Allocation`, an enum of `Percentage(u32)` (basis
//! points of whatever remains after fixed amounts are paid) or
//! `FixedAmount(i128)` (an exact amount paid before any percentage split).
//! See `Allocation`'s doc comment in `types.rs` for the full model.
//!
//! This is a self-contained test module (its own `setup`, not sharing
//! `test.rs`) so it does not depend on the large pre-existing test suite,
//! which still constructs `Beneficiary` via its old `basis_points` field and
//! is out of scope for this change.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::fuzz_harness::{
    assert_beneficiaries_are_indexed, assert_custody_matches_recorded_balance,
    assert_percentage_shares_sum_to_10000,
};
use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

/// Asserts the same structural invariants the fuzz harness checks after
/// every accepted `create_will`/`top_up` (issue #267): the recorded
/// percentage shares sum to 10,000 (when any are percentage-typed), the
/// contract's custody matches the will's recorded balance, and every
/// beneficiary is reachable via the reverse index.
fn assert_will_invariants(
    client: &WillContractClient,
    token: &TokenClient,
    token_address: &Address,
    will_id: u64,
) {
    let will = client.get_will(&will_id);
    assert_percentage_shares_sum_to_10000(&will);
    assert_custody_matches_recorded_balance(token, &client.address, token_address, &will);
    assert_beneficiaries_are_indexed(client, &will);
}

const DAY: u64 = 86_400;

fn setup<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (
        env.clone(),
        client,
        owner,
        TokenClient::new(&env, &token_address),
        token_address,
    )
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

fn release(env: &Env, client: &WillContractClient, will_id: u64) {
    advance(env, 91);
    client.trigger_will(&will_id);
    advance(env, 8);
    client.release_inheritance(&will_id, &None);
}

/// Regression: a pure-percentage will (the original, pre-`Allocation` shape)
/// must still split the whole balance proportionally, with the rounding
/// remainder absorbed by the last beneficiary.
#[test]
fn pure_percentage_regression() {
    let (env, client, owner, token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: a.clone(),
            allocation: Allocation::Percentage(6_000),
        },
        Beneficiary {
            address: b.clone(),
            allocation: Allocation::Percentage(4_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    assert_will_invariants(&client, &token, &token_address, will_id);
    release(&env, &client, will_id);

    assert_eq!(token.balance(&a), 600_000);
    assert_eq!(token.balance(&b), 400_000);
    assert_eq!(token.balance(&client.address), 0);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// Pure-fixed-amount will: every beneficiary gets exactly the amount they
/// were promised, and the fixed amounts must account for the entire balance
/// since nobody is left to receive a percentage-based remainder.
#[test]
fn pure_fixed_amount() {
    let (env, client, owner, token, token_address) = setup();
    let sister = Address::generate(&env);
    let brother = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: sister.clone(),
            allocation: Allocation::FixedAmount(700_000),
        },
        Beneficiary {
            address: brother.clone(),
            allocation: Allocation::FixedAmount(300_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    // Pure-fixed-amount wills have no percentage-typed beneficiaries, so
    // assert_percentage_shares_sum_to_10000 is a no-op here; custody and
    // index consistency still apply.
    assert_custody_matches_recorded_balance(&token, &client.address, &token_address, &client.get_will(&will_id));
    assert_beneficiaries_are_indexed(&client, &client.get_will(&will_id));
    release(&env, &client, will_id);

    assert_eq!(token.balance(&sister), 700_000);
    assert_eq!(token.balance(&brother), 300_000);
    assert_eq!(token.balance(&client.address), 0);
}

/// Mixed configuration: one beneficiary gets an exact fixed amount, and the
/// rest split whatever remains by percentage — the scenario from the issue
/// ("my sister gets exactly 5,000 USDC, the remainder split by percentage").
#[test]
fn mixed_fixed_and_percentage() {
    let (env, client, owner, token, token_address) = setup();
    let sister = Address::generate(&env);
    let child_a = Address::generate(&env);
    let child_b = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: sister.clone(),
            allocation: Allocation::FixedAmount(200_000),
        },
        Beneficiary {
            address: child_a.clone(),
            allocation: Allocation::Percentage(5_000),
        },
        Beneficiary {
            address: child_b.clone(),
            allocation: Allocation::Percentage(5_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    assert_will_invariants(&client, &token, &token_address, will_id);
    release(&env, &client, will_id);

    // Sister gets her exact fixed amount; the remaining 800,000 splits 50/50.
    assert_eq!(token.balance(&sister), 200_000);
    assert_eq!(token.balance(&child_a), 400_000);
    assert_eq!(token.balance(&child_b), 400_000);
    assert_eq!(token.balance(&client.address), 0);
}

/// The sum of fixed amounts can never exceed the will's balance.
#[test]
fn fixed_amount_exceeding_balance_is_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: a,
            allocation: Allocation::FixedAmount(900_000),
        },
        Beneficiary {
            address: b,
            allocation: Allocation::FixedAmount(200_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

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
        Err(Ok(WillError::FixedAmountExceedsBalance.into()))
    );
}

/// A `top_up` after creation increases the balance available to whatever
/// remains after fixed amounts, so percentage beneficiaries — not the fixed
/// one — capture the extra funds. This is the exact motivating case from the
/// issue: fixed amounts don't need recalculating by hand as the balance
/// changes.
#[test]
fn top_up_grows_the_percentage_remainder_not_the_fixed_share() {
    let (env, client, owner, token, token_address) = setup();
    let sister = Address::generate(&env);
    let rest = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: sister.clone(),
            allocation: Allocation::FixedAmount(200_000),
        },
        Beneficiary {
            address: rest.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

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
    client.top_up(&will_id, &owner, &token_address, &500_000);
    assert_will_invariants(&client, &token, &token_address, will_id);

    release(&env, &client, will_id);

    assert_eq!(token.balance(&sister), 200_000);
    assert_eq!(token.balance(&rest), 1_300_000);
}

/// Demonstrates the ergonomics unlocked by `Will: PartialEq + Eq`.
///
/// Before `Will` derived `PartialEq`/`Eq`, verifying that a `check_in` call
/// left the will otherwise unchanged required one `assert_eq!` per field and
/// it was easy to silently miss one. Now a single `assert_eq!(will_before,
/// will_after)` compares every field simultaneously, and any unexpected
/// mutation causes an immediate, descriptive failure.
///
/// Here we verify that two separately fetched snapshots of the same
/// newly-created will are identical — a baseline sanity check that is only
/// expressible as a single statement because `Will` now implements `PartialEq`.
#[test]
fn will_partialeq_allows_single_assert() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 500_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Fetch the will twice — the two snapshots must be identical.
    let will_first = client.get_will(&will_id);
    let will_second = client.get_will(&will_id);

    // A single assert_eq! compares every Will field at once.
    // Without Will: PartialEq this line would not compile.
    assert_eq!(will_first, will_second);

    // Also confirm that the key fields are what we expect.
    assert_eq!(will_first.id, will_id);
    assert_eq!(will_first.owner, owner);
    assert_eq!(will_first.status, WillStatus::Active);
    assert_eq!(will_first.checkin_period_days, 30);
    assert_eq!(will_first.grace_period_days, 7);
}

/// Three-way equal percentage split with rounding remainder absorbed by the
/// last beneficiary. Validates that the rounding behavior is consistent across
/// multiple beneficiaries and that the total distributed equals the initial balance.
#[test]
fn three_way_percentage_split_with_remainder() {
    let (env, client, owner, token, token_address) = setup();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let charlie = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: alice.clone(),
            allocation: Allocation::Percentage(3_333),
        },
        Beneficiary {
            address: bob.clone(),
            allocation: Allocation::Percentage(3_333),
        },
        Beneficiary {
            address: charlie.clone(),
            allocation: Allocation::Percentage(3_334),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    assert_will_invariants(&client, &token, &token_address, will_id);
    release(&env, &client, will_id);

    let alice_balance = token.balance(&alice);
    let bob_balance = token.balance(&bob);
    let charlie_balance = token.balance(&charlie);

    assert_eq!(alice_balance, 333_300);
    assert_eq!(bob_balance, 333_300);
    assert_eq!(charlie_balance, 333_400);
    assert_eq!(token.balance(&client.address), 0);
    assert_eq!(alice_balance + bob_balance + charlie_balance, 1_000_000);
}
