#![cfg(test)]

//! Regression tests for issues #535-#538:
//! Invariant checks after each schema migration.
//!
//! Verifies that `migration::upgrade` enforces invariants on the upgraded `Will`,
//! ensuring corrupted or invalid state in a legacy schema version cannot successfully
//! complete migration.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    migration, storage, Allocation, Beneficiary, WillContract, WillContractClient, WillError,
    CURRENT_SCHEMA_VERSION,
};

fn setup_will(env: &Env) -> (Address, WillContractClient, Address, Address, u64, Address) {
    let owner = Address::generate(env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    let beneficiary = Address::generate(env);
    let guardian = Address::generate(env);

    let will_id = client.create_will(
        &owner,
        &vec![env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    (owner, client, token_address, beneficiary, will_id, contract_id)
}

#[test]
fn test_migrate_valid_will_preserves_invariants() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Set schema_version to 0 directly in storage
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        storage::save_will(&env, &will);
    });

    assert_eq!(client.get_will(&will_id).schema_version, 0);

    // Migration succeeds
    client.migrate_will(&will_id, &owner);

    let migrated = client.get_will(&will_id);
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.balance, 1_000_000);
    assert_eq!(migrated.guardian_threshold, 1);
}

#[test]
fn test_migrate_corrupted_balance_mirror_fails_invariant() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Simulate corrupted balance mirror in v0
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.balance = 500_000; // Mismatch with balances map
        storage::save_will(&env, &will);
    });

    let result = client.try_migrate_will(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::DistributionMismatch.into())));
}

#[test]
fn test_migrate_invalid_period_fails_invariant() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Simulate invalid check-in period in v0
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.checkin_period_days = 0;
        storage::save_will(&env, &will);
    });

    let result = client.try_migrate_will(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::InvalidPeriod.into())));
}

#[test]
fn test_migrate_invalid_guardian_threshold_fails_invariant() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Simulate unreachable guardian threshold in v0
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.guardian_threshold = 99; // Only 1 guardian exists
        storage::save_will(&env, &will);
    });

    let result = client.try_migrate_will(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::InvalidGuardianThreshold.into())));
}

#[test]
fn test_migrate_invalid_percentages_fails_invariant() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, beneficiary, will_id, contract_id) = setup_will(&env);

    // Simulate invalid percentage > 10,000 in v0
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.beneficiaries = vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(15_000),
            },
        ];
        storage::save_will(&env, &will);
    });

    let result = client.try_migrate_will(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::InvalidPercentages.into())));
}
