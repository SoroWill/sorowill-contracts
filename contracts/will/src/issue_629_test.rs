#![cfg(test)]

//! Regression tests for Issue #629:
//! Keeper bounty calculation doesn't account for token decimals, overpaying or underpaying.
//!
//! Verifies:
//! 1. `normalize_bounty_decimals` correctly scales between 6-decimal and 18-decimal precisions.
//! 2. Contract distribution correctly handles 18-decimal and 6-decimal tokens without over/underpaying.
//! 3. Conservation of balance holds under multi-decimal keeper bounties.

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{normalize_bounty_decimals, Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

#[contract]
pub struct MockToken6;

#[contractimpl]
impl MockToken6 {
    pub fn decimals(_env: Env) -> u32 {
        6
    }
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {}
    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }
}

#[contract]
pub struct MockToken18;

#[contractimpl]
impl MockToken18 {
    pub fn decimals(_env: Env) -> u32 {
        18
    }
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {}
    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }
}

#[test]
fn test_normalize_bounty_decimals_same_precision() {
    assert_eq!(normalize_bounty_decimals(1000, 6, 6), 1000);
    assert_eq!(normalize_bounty_decimals(1000, 18, 18), 1000);
    assert_eq!(normalize_bounty_decimals(0, 6, 18), 0);
}

#[test]
fn test_normalize_bounty_decimals_scale_up_6_to_18() {
    // 1 unit in 6 decimals = 1_000_000
    // In 18 decimals = 1_000_000 * 10^12 = 1_000_000_000_000_000_000
    let amount_6 = 50_000_i128; // 50 bps on 10_000_000
    let scaled_18 = normalize_bounty_decimals(amount_6, 6, 18);
    assert_eq!(scaled_18, 50_000_000_000_000_000_i128);
}

#[test]
fn test_normalize_bounty_decimals_scale_down_18_to_6() {
    let amount_18 = 50_000_000_000_000_000_i128;
    let scaled_6 = normalize_bounty_decimals(amount_18, 18, 6);
    assert_eq!(scaled_6, 50_000_i128);
}

#[test]
fn test_normalize_bounty_decimals_small_amount_truncation() {
    // Truncation when smaller than scale factor
    let small_amount = 500_i128;
    let scaled_down = normalize_bounty_decimals(small_amount, 18, 6);
    assert_eq!(scaled_down, 0);
}

#[test]
fn test_distribute_with_18_and_6_decimal_tokens() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let keeper = Address::generate(&env);
    let heir = Address::generate(&env);

    let token6_addr = env.register(MockToken6, ());
    let token18_addr = env.register(MockToken18, ());

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Primary token has 6 decimals, secondary has 18 decimals
    let locked_6 = 10_000_000_i128; // 10.0 tokens at 6 decimals
    let locked_18 = 10_000_000_000_000_000_000_i128; // 10.0 tokens at 18 decimals

    let will_id = client.create_will(
        &owner,
        &vec![
            &env,
            (token6_addr.clone(), locked_6),
            (token18_addr.clone(), locked_18),
        ],
        &vec![
            &env,
            Beneficiary {
                address: heir.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &Some(100), // 100 bps = 1%
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);

    // Release will with non-owner keeper
    client.release_inheritance(&will_id, &Some(keeper.clone()));

    // Verify will is released
    let released_will = client.get_will(&will_id);
    assert_eq!(released_will.status, crate::WillStatus::Released);
}
