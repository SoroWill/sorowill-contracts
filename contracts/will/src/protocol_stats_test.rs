#![cfg(test)]

//! Regression test for ProtocolStats.total_locked_by_token tracking.
//! Verifies that locked balances are incremented when wills are created
//! or topped up, and decremented when they are cancelled.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};


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

#[test]
fn create_will_increments_locked_value() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create first will
    let _will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Get protocol stats after first create
    let stats_1 = client.get_protocol_stats();
    let locked_1 = stats_1.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    assert_eq!(locked_1, 100_000, "First will should increment locked value");

    // Create second will with same token
    let _will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 250_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Verify cumulative locked value
    let stats_2 = client.get_protocol_stats();
    let locked_2 = stats_2.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    assert_eq!(locked_2, 350_000, "Second will should add to cumulative locked value");
}

#[test]
fn top_up_increments_locked_value() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create a will
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Get initial locked value
    let stats_before = client.get_protocol_stats();
    let locked_before = stats_before.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    assert_eq!(locked_before, 100_000);

    // Top up the will
    client.top_up(&will_id, &owner, &token_address, &75_000);

    // Verify locked value increased
    let stats_after = client.get_protocol_stats();
    let locked_after = stats_after.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    assert_eq!(locked_after, 175_000, "Top-up should increment locked value");
}

#[test]
fn cancel_will_decrements_locked_value() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create a will
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 200_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Get locked value after create
    let stats_before = client.get_protocol_stats();
    let locked_before = stats_before.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    assert_eq!(locked_before, 200_000);

    // Cancel the will
    client.cancel_will(&will_id, &owner);

    // Verify locked value decreased
    let stats_after = client.get_protocol_stats();
    let locked_after = stats_after.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    assert_eq!(locked_after, 0, "Cancelling should decrement locked value");
}

#[test]
fn multiple_tokens_track_independently() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create a second token
    let owner2 = Address::generate(&env);
    let sac2 = env.register_stellar_asset_contract_v2(owner2.clone());
    let token_address_2 = sac2.address();
    StellarAssetClient::new(&env, &token_address_2).mint(&owner, &1_000_000_000);

    // Create will with first token
    let _will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Create will with second token
    let _will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address_2.clone(), 50_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Verify each token is tracked independently
    let stats = client.get_protocol_stats();
    let locked_1 = stats.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address)
        .map(|e| e.total_locked)
        .unwrap_or(0);
    let locked_2 = stats.total_locked_by_token
        .iter()
        .find(|e| e.token == token_address_2)
        .map(|e| e.total_locked)
        .unwrap_or(0);

    assert_eq!(locked_1, 100_000);
    assert_eq!(locked_2, 50_000);
}
