#![cfg(test)]

//! Regression tests for issues #509-#522:
//! Validate status is reachable from previous schema.
//!
//! Verifies that `migration::upgrade` and `migrate_will` validate that the status
//! and state machine invariants (e.g. trigger timestamps) are consistent and reachable
//! from the previous schema version.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    storage, Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus,
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
fn test_migrate_active_status_reachable() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.status = WillStatus::Active;
        will.trigger_time = None;
        storage::save_will(&env, &will);
    });

    client.migrate_will(&will_id, &owner);
    let migrated = client.get_will(&will_id);
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.status, WillStatus::Active);
}

#[test]
fn test_migrate_triggered_status_reachable_with_trigger_time() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.status = WillStatus::Triggered;
        will.trigger_time = Some(1_700_000_000);
        storage::save_will(&env, &will);
    });

    client.migrate_will(&will_id, &owner);
    let migrated = client.get_will(&will_id);
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.status, WillStatus::Triggered);
}

#[test]
fn test_migrate_triggered_status_unreachable_without_trigger_time() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Corrupted state: Triggered but missing trigger_time
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.status = WillStatus::Triggered;
        will.trigger_time = None;
        storage::save_will(&env, &will);
    });

    let result = client.try_migrate_will(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::WillNotTriggered.into())));
}

#[test]
fn test_migrate_active_status_unreachable_with_orphaned_trigger_time() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Corrupted state: Active but has trigger_time
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.status = WillStatus::Active;
        will.trigger_time = Some(1_700_000_000);
        storage::save_will(&env, &will);
    });

    let result = client.try_migrate_will(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::WillNotActive.into())));
}

#[test]
fn test_migrate_terminal_statuses_reachable() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, will_id, contract_id) = setup_will(&env);

    // Released status
    env.as_contract(&contract_id, || {
        let mut will = storage::load_will(&env, will_id).expect("will not found");
        will.schema_version = 0;
        will.status = WillStatus::Released;
        storage::save_will(&env, &will);
    });
    client.migrate_will(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}
