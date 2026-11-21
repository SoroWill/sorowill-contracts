#![cfg(test)]

//! Regression tests for issue #716: a will must not hold more than
//! [`crate::MAX_BALANCE`] of a single token, so balance arithmetic can never
//! approach `i128` overflow.

use soroban_sdk::{
    testutils::Address as _,
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, MAX_BALANCE};

fn setup(env: &Env, mint: i128) -> (WillContractClient, Address, Address) {
    env.mock_all_auths();

    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &mint);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (client, owner, token_address)
}

fn beneficiaries(env: &Env) -> soroban_sdk::Vec<Beneficiary> {
    vec![
        env,
        Beneficiary {
            address: Address::generate(env),
            allocation: Allocation::Percentage(10_000),
        },
    ]
}

/// A deposit exactly at the cap is accepted (#716).
#[test]
fn deposit_at_max_balance_is_accepted() {
    let env = Env::default();
    let (client, owner, token) = setup(&env, MAX_BALANCE);

    let tokens = vec![&env, (token.clone(), MAX_BALANCE)];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries(&env),
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    assert_eq!(client.get_will(&will_id).balances.get(token).unwrap(), MAX_BALANCE);
}

/// A deposit one unit above the cap is rejected at creation (#716).
#[test]
fn deposit_above_max_balance_is_rejected() {
    let env = Env::default();
    let (client, owner, token) = setup(&env, MAX_BALANCE + 1);

    let tokens = vec![&env, (token.clone(), MAX_BALANCE + 1)];
    let result = client.try_create_will(
        &owner,
        &tokens,
        &beneficiaries(&env),
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );
    assert!(result.is_err(), "amount above MAX_BALANCE must be rejected");
}

/// A top-up that would push a token's balance past the cap is rejected, and
/// one that lands exactly on the cap is accepted (#716).
#[test]
fn top_up_past_max_balance_is_rejected() {
    let env = Env::default();
    let (client, owner, token) = setup(&env, MAX_BALANCE + 1);

    let tokens = vec![&env, (token.clone(), MAX_BALANCE - 1)];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries(&env),
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let over = client.try_top_up(&will_id, &owner, &token, &2);
    assert!(over.is_err(), "top-up crossing MAX_BALANCE must be rejected");

    client.top_up(&will_id, &owner, &token, &1);
    assert_eq!(client.get_will(&will_id).balances.get(token).unwrap(), MAX_BALANCE);
}
