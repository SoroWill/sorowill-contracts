#![cfg(test)]

//! Regression tests for issues #551-#554:
//! Test deserialization of partially migrated state.
//!
//! Verifies that `migration::decode_will` and `storage::load_will` correctly
//! deserialize wills that are partially migrated or from older schema versions (v0),
//! preserving stored fields and returning UnsupportedSchemaVersion on unrecognised layouts.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, IntoVal,
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
fn test_deserialize_partially_migrated_v0_state() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (_owner, client, token_address, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Simulate an older schema version (v0) prior to migration
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        storage::save_will(&env, &will);
    });

    // Deserialization via get_will and storage::load_will must succeed
    let loaded = client.get_will(&will_id);
    assert_eq!(loaded.schema_version, 0);
    assert_eq!(loaded.balance, 1_000_000);
    assert_eq!(loaded.token, token_address);
    assert_eq!(loaded.beneficiaries.len(), 1);
    assert_eq!(loaded.guardians.len(), 1);
}

#[test]
fn test_decode_will_unsupported_schema_version() {
    let env = Env::default();

    // Invalid raw Val that does not match Will struct shape
    let raw_symbol = symbol_short!("corrupt").into_val(&env);
    let result = migration::decode_will(&env, &raw_symbol);

    assert_eq!(result, Err(WillError::UnsupportedSchemaVersion));
}

#[test]
fn test_partially_migrated_will_completes_migration() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Set schema_version to 0
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        storage::save_will(&env, &will);
    });

    // Verify it is readable as schema_version = 0
    assert_eq!(client.get_will(&will_id).schema_version, 0);

    // Migrate will to latest schema
    client.migrate_will(&will_id, &owner);

    // Verify deserialization after migration
    let migrated = client.get_will(&will_id);
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.balance, 1_000_000);
}

#[test]
fn test_deserialization_of_partially_migrated_multitoken_state() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, token1, _beneficiary, will_id, contract_id) = setup_will(&env);

    let token2 = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token2).mint(&owner, &500_000);

    // Top up second token
    client.top_up(&will_id, &owner, &token2, &500_000);

    // Mark as un-migrated / v0
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        storage::save_will(&env, &will);
    });

    // Deserialization preserves multi-token balances
    let loaded = client.get_will(&will_id);
    assert_eq!(loaded.schema_version, 0);
    assert_eq!(loaded.balances.get(token1), Some(1_000_000));
    assert_eq!(loaded.balances.get(token2), Some(500_000));
}
