#![cfg(test)]

//! Tests for `migrate_will` (#218).
//!
//! Covers the version bump to `CURRENT_SCHEMA_VERSION`, the no-op path when a
//! will is already current, and rejection of a non-owner caller.
//!
//! There is no entry point that creates a stale will — `create_will` always
//! stamps the current version — so these tests write `schema_version = 0`
//! directly into contract storage to produce one.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, TryIntoVal,
};

use crate::{
    storage, Allocation, Beneficiary, WillContract, WillContractClient, WillError,
    CURRENT_SCHEMA_VERSION,
};

fn setup() -> (Env, Address, Address, u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
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

    (env, contract_id, owner, will_id)
}

/// Rewrites `will_id`'s stored `schema_version`, simulating a will written by
/// an older contract version.
fn set_schema_version(env: &Env, contract_id: &Address, will_id: u64, version: u32) {
    env.as_contract(contract_id, || {
        let mut will = match storage::load_will(env, will_id) {
            Ok(will) => will,
            Err(_) => panic!("will not found"),
        };
        will.schema_version = version;
        storage::save_will(env, &will);
    });
}

/// Number of `will_migrated` events published so far.
fn migrated_event_count(env: &Env) -> u32 {
    let mut count = 0;
    for event in env.events().all().iter() {
        if event.1.is_empty() {
            continue;
        }
        let topic0: Result<soroban_sdk::Symbol, _> = event.1.get(0).unwrap().try_into_val(env);
        if topic0 == Ok(symbol_short!("migrated")) {
            count += 1;
        }
    }
    count
}

#[test]
fn migrate_bumps_a_stale_will_to_the_current_schema_version() {
    let (env, contract_id, owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    set_schema_version(&env, &contract_id, will_id, 0);
    assert_eq!(client.get_will(&will_id).schema_version, 0);

    client.migrate_will(&will_id, &owner);

    // Checked before any further client call: env.events().all() in
    // Soroban's test host only retains events from the most recent
    // top-level invocation, so a later get_will() call would wipe this.
    assert_eq!(
        migrated_event_count(&env),
        1,
        "a completed migration must publish exactly one will_migrated event"
    );
    assert_eq!(
        client.get_will(&will_id).schema_version,
        CURRENT_SCHEMA_VERSION
    );
}

#[test]
fn migrate_is_a_no_op_when_already_current() {
    let (env, contract_id, owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // `create_will` already stamps the current version.
    assert_eq!(
        client.get_will(&will_id).schema_version,
        CURRENT_SCHEMA_VERSION
    );

    client.migrate_will(&will_id, &owner);
    // Calling twice must be equally harmless.
    client.migrate_will(&will_id, &owner);

    assert_eq!(
        client.get_will(&will_id).schema_version,
        CURRENT_SCHEMA_VERSION
    );
    assert_eq!(
        migrated_event_count(&env),
        0,
        "a no-op migration must not re-emit will_migrated"
    );
}

#[test]
fn migrate_rejects_a_non_owner_caller() {
    let (env, contract_id, _owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    set_schema_version(&env, &contract_id, will_id, 0);
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_migrate_will(&will_id, &stranger),
        Err(Ok(WillError::NotOwner.into()))
    );
    assert_eq!(
        client.get_will(&will_id).schema_version,
        0,
        "a rejected migration must leave the stored version untouched"
    );
}
