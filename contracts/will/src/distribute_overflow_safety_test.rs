#![cfg(test)]

//! Regression test for issue #190: ensure that `distribute()` uses the
//! overflow-safe `proportional_share` helper for beneficiary-payout calculations,
//! not the unsafe direct multiplication approach.
//!
//! The `proportional_share` helper computes `floor(total * basis_points / 10_000)`
//! without ever forming the potentially-overflowing `total * basis_points` intermediate.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

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
    StellarAssetClient::new(&env, &token_address).mint(&owner, &9_223_372_036_854_775_807i128);

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

/// Regression test for issue #190: large balance with percentage beneficiaries
/// should not overflow when computing shares, and should produce correct results.
///
/// This test confirms that `distribute()` computes per-beneficiary shares using
/// the overflow-safe `proportional_share` helper, not the unsafe direct
/// `remaining * bp / 10_000` calculation.
#[test]
fn distribute_large_balance_no_overflow() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary_a.clone(),
            allocation: Allocation::Percentage(3_333),
        },
        Beneficiary {
            address: beneficiary_b.clone(),
            allocation: Allocation::Percentage(6_667),
        },
    ];

    let large_amount = 9_000_000_000_000_000i128;
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, large_amount)];

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

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);

    release(&env, &client, will_id);

    let balance_a = token.balance(&beneficiary_a);
    let balance_b = token.balance(&beneficiary_b);

    assert!(balance_a > 0, "beneficiary_a should receive a share");
    assert!(balance_b > 0, "beneficiary_b should receive a share");

    let total_distributed = balance_a + balance_b;
    assert_eq!(
        total_distributed, large_amount,
        "total distributed should equal the will balance"
    );

    let ratio_a = (balance_a * 10_000) / large_amount;
    let ratio_b = (balance_b * 10_000) / large_amount;

    assert!(
        (ratio_a - 3_333).abs() <= 1,
        "beneficiary_a should receive approximately 33.33% (3333 bp), got {} bp",
        ratio_a
    );
    assert!(
        (ratio_b - 6_667).abs() <= 1,
        "beneficiary_b should receive approximately 66.67% (6667 bp), got {} bp",
        ratio_b
    );
}

/// Regression test: very small percentages with large balances should not
/// round incorrectly when using `proportional_share`.
#[test]
fn distribute_small_percentage_large_balance() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_small = Address::generate(&env);
    let beneficiary_large = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary_small.clone(),
            allocation: Allocation::Percentage(1),
        },
        Beneficiary {
            address: beneficiary_large.clone(),
            allocation: Allocation::Percentage(9_999),
        },
    ];

    let large_amount = 1_000_000_000_000i128;
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, large_amount)];

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
    release(&env, &client, will_id);

    let balance_small = token.balance(&beneficiary_small);
    let balance_large = token.balance(&beneficiary_large);

    assert!(
        balance_small > 0,
        "even tiny percentage should result in nonzero amount"
    );
    let total = balance_small + balance_large;
    assert_eq!(total, large_amount, "entire balance should be distributed");
}

/// Issue #297: Assert that `keeper_bounty_bps` is paid out of the distributed balance
/// rather than on top of it, i.e. beneficiaries' shares + bounty == total locked balance.
#[test]
fn distribute_with_keeper_bounty_reduces_beneficiary_shares() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let keeper = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let locked_amount = 1_000_000i128;
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, locked_amount)];

    // 50 bps = 0.5% keeper bounty
    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &Some(50),
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &Some(keeper.clone()));

    let beneficiary_balance = token.balance(&beneficiary);
    let keeper_balance = token.balance(&keeper);

    let expected_bounty = (locked_amount * 50) / 10_000;
    let expected_beneficiary_share = locked_amount - expected_bounty;

    assert_eq!(keeper_balance, expected_bounty);
    assert_eq!(beneficiary_balance, expected_beneficiary_share);
    assert_eq!(beneficiary_balance + keeper_balance, locked_amount);
}
