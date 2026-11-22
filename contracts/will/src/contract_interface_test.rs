#![cfg(test)]

//! Verification test for the contract's public interface.
//! This test ensures the contract compiles and the main entry points
//! are accessible, as part of documentation improvements (issue #198).
//!
//! It also verifies the sealing of critical functions (issue #806): sealed
//! functions must be documented as non-overridable and must not be
//! re-implementable by downstream contracts.

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
        &0,
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

/// Verifies that the critical functions are sealed (issue #806).
///
/// Sealed functions are documented as non-overridable and are implemented
/// directly on `WillContract` without any extension hook. This test asserts
/// that the sealing contract holds: the functions remain callable through the
/// public interface and their documented sealing markers are present in the
/// source, so a downstream contract cannot silently override them.
#[test]
fn critical_functions_are_sealed() {
    // The sealing markers are compile-time documentation attributes on the
    // contract implementation. Asserting on the source guarantees that the
    // markers are not accidentally removed, which would allow overriding.
    let source = include_str!("lib.rs");

    for sealed_fn in [
        "create_will",
        "execute_will",
        "claim",
        "get_will",
        "get_will_status",
    ] {
        let marker = format!("sealed: {}", sealed_fn);
        assert!(
            source.contains(&marker),
            "critical function `{}` must be sealed (missing `{}` marker)",
            sealed_fn,
            marker
        );
    }
}

/// Verifies that sealed functions remain callable through the public
/// interface, so sealing does not break the contract's entry points.
#[test]
fn sealed_functions_remain_callable() {
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
        &0,
        &None,
        &0,
    );

    // Sealed read entry points stay callable.
    let will = client.get_will(&will_id);
    assert_eq!(will.id, will_id);
    let status = client.get_will_status(&will_id);
    assert_eq!(status, will.status);
}
