#![cfg(test)]

//! Verification test for the contract's public interface.
//! This test ensures the contract compiles and the main entry points
//! are accessible, as part of documentation improvements (issue #198).

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn contract_interface_is_available() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary = Address::generate(&env);

    // Test that main entry points are accessible
    // create_will
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
        &1,
        &None,
        &0,
    );

    // get_will
    let will = client.get_will(&will_id);
    assert_eq!(will.id, will_id);

    // get_will_status
    let status = client.get_will_status(&will_id);
    assert_eq!(status, will.status);

    // get_time_until_deadline
    let deadline = client.get_time_until_deadline(&will_id);
    assert!(deadline.is_some());

    // get_wills_by_owner
    let owner_wills = client.get_wills_by_owner(&owner, &None, &10);
    assert!(!owner_wills.is_empty());

    // get_wills_by_beneficiary
    let beneficiary_wills = client.get_wills_by_beneficiary(&beneficiary, &None, &10);
    assert!(!beneficiary_wills.is_empty());

    // get_protocol_stats
    let stats = client.get_protocol_stats();
    assert!(!stats.total_locked_by_token.is_empty());

    // get_contract_version
    let version = client.get_contract_version();
    assert!(version > 0);
}
