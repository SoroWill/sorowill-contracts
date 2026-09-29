#![cfg(test)]

//! Regression test for issue #264: `adjust_locked_value` used to add a
//! fresh `TokenLockedBalance` entry the first time any token was used and
//! never pruned it again, even once `total_locked` returned to exactly
//! zero -- growing `total_locked_by_token` by one permanent row per
//! distinct token ever used, however briefly.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn a_fully_cancelled_one_off_token_is_pruned_from_protocol_stats() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // The token is now locked, so it has an entry with a positive total.
    let stats_while_locked = client.get_protocol_stats();
    let locked_entry = stats_while_locked
        .total_locked_by_token
        .iter()
        .find(|entry| entry.token == token_address)
        .expect("token should have a locked entry while the will is active");
    assert_eq!(locked_entry.total_locked, 1_000_000);

    // Cancelling withdraws the full balance, returning total_locked to zero.
    client.cancel_will(&will_id, &owner);

    let stats_after_cancel = client.get_protocol_stats();
    assert!(
        !stats_after_cancel
            .total_locked_by_token
            .iter()
            .any(|entry| entry.token == token_address),
        "a token whose total_locked returned to zero must be pruned, not kept as a zero row"
    );
}
