#![cfg(test)]

//! Regression test for issue #189: ensure that `merge_wills` preserves
//! `Allocation::FixedAmount` beneficiaries' fixed-amount semantics instead of
//! silently converting them to `Allocation::Percentage`.
//!
//! A beneficiary configured with a guaranteed fixed payout (e.g. "always get exactly
//! 500 USDC") should retain that guarantee through a merge operation, not have it
//! silently converted to a percentage share.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, TokenClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, TokenClient::new(&env, &token_address), token_address)
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

/// Regression test for issue #189: a FixedAmount beneficiary should retain
/// their fixed-amount semantics after a merge, not be silently converted to Percentage.
#[test]
fn merge_preserves_fixed_amount_allocation() {
    let (env, client, owner, token, token_address) = setup();
    let fixed_beneficiary = Address::generate(&env);
    let percentage_beneficiary = Address::generate(&env);

    let fixed_amount = 50_000i128;

    let will_a_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: fixed_beneficiary.clone(),
            allocation: Allocation::FixedAmount(fixed_amount),
        },
        Beneficiary {
            address: percentage_beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let will_a_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 100_000i128)];

    let will_b_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: fixed_beneficiary.clone(),
            allocation: Allocation::FixedAmount(fixed_amount),
        },
        Beneficiary {
            address: percentage_beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let will_b_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 900_000i128)];

    let will_a = client.create_will(&owner, &will_a_tokens, &will_a_beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    let will_b = client.create_will(&owner, &will_b_tokens, &will_b_beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    client.merge_wills(&owner, &will_a, &will_b);

    // Check that the merged will has both beneficiaries
    let merged_will = client.get_will(&will_a);
    assert_eq!(merged_will.beneficiaries.len(), 2, "should have both beneficiaries after merge");

    // Verify that the fixed beneficiary is still FixedAmount, not converted to Percentage
    let fixed_ben = merged_will.beneficiaries.iter()
        .find(|b| b.address == fixed_beneficiary)
        .expect("fixed beneficiary should exist");

    match fixed_ben.allocation {
        Allocation::FixedAmount(amt) => {
            // The amount should be the sum of both wills' fixed amounts
            assert_eq!(amt, fixed_amount * 2, "fixed amount should be sum of both wills");
        },
        Allocation::Percentage(_) => {
            panic!("fixed beneficiary was incorrectly converted to Percentage!");
        },
    }

    release(&env, &client, will_a);

    // Verify actual distribution matches the fixed-amount semantics
    let fixed_balance = token.balance(&fixed_beneficiary);
    assert_eq!(
        fixed_balance, fixed_amount * 2,
        "fixed beneficiary should receive exactly the fixed amount ({}), got {}",
        fixed_amount * 2, fixed_balance
    );
}

/// Regression test: a will with only FixedAmount beneficiaries should remain FixedAmount after merge
#[test]
fn merge_preserves_all_fixed_amounts() {
    let (env, client, owner, token, token_address) = setup();
    let fixed_a = Address::generate(&env);
    let fixed_b = Address::generate(&env);

    let amount_a = 100_000i128;
    let amount_b = 200_000i128;

    let will_1_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary { address: fixed_a.clone(), allocation: Allocation::FixedAmount(amount_a) },
        Beneficiary { address: fixed_b.clone(), allocation: Allocation::FixedAmount(amount_b) },
    ];
    let will_1_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 300_000i128)];

    let will_2_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary { address: fixed_a.clone(), allocation: Allocation::FixedAmount(amount_a) },
        Beneficiary { address: fixed_b.clone(), allocation: Allocation::FixedAmount(amount_b) },
    ];
    let will_2_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 300_000i128)];

    let will_1 = client.create_will(&owner, &will_1_tokens, &will_1_beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    let will_2 = client.create_will(&owner, &will_2_tokens, &will_2_beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    client.merge_wills(&owner, &will_1, &will_2);

    let merged_will = client.get_will(&will_1);

    // Verify both beneficiaries remain FixedAmount
    for beneficiary in merged_will.beneficiaries.iter() {
        match beneficiary.allocation {
            Allocation::FixedAmount(_) => {
                // Good, preserve the fixed amount semantics
            },
            Allocation::Percentage(_) => {
                panic!("beneficiary {:?} was incorrectly converted to Percentage!", beneficiary.address);
            },
        }
    }

    release(&env, &client, will_1);

    let balance_a = token.balance(&fixed_a);
    let balance_b = token.balance(&fixed_b);

    assert_eq!(balance_a, amount_a * 2, "fixed_a should receive exactly {} tokens", amount_a * 2);
    assert_eq!(balance_b, amount_b * 2, "fixed_b should receive exactly {} tokens", amount_b * 2);
}
