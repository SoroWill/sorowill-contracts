#![cfg(test)]

//! Regression tests for issue #625: each top-up re-checks the allocations
//! against the final primary balance before it is saved.
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn a_sequence_of_top_ups_keeps_the_allocations_valid() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let primary = env.register_stellar_asset_contract_v2(owner.clone()).address();
    let secondary = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &primary).mint(&owner, &10_000);
    StellarAssetClient::new(&env, &secondary).mint(&owner, &10_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (primary.clone(), 1_000_i128)],
        &vec![
            &env,
            Beneficiary { address: Address::generate(&env), allocation: Allocation::FixedAmount(800) },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );
    client.top_up(&will_id, &owner, &secondary, &500);
    client.top_up(&will_id, &owner, &primary, &200);
    client.top_up(&will_id, &owner, &secondary, &300);
    assert_eq!(client.get_will(&will_id).balance, 1_200);
}
