#![cfg(test)]

//! Tests for `clone_will` (#216).
//!
//! Covers that a clone copies the source's beneficiaries, guardians and
//! periods, that it gets a fresh id and check-in deadline, and pins the
//! current `active_will_count` behaviour for a clone.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

/// Registers the contract and creates a funded source will with two
/// beneficiaries and two guardians. Returns the env, the contract address,
/// the owner, the token address and the source will id.
fn setup() -> (Env, Address, Address, Address, u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let source_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::Percentage(6_000),
            },
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::Percentage(4_000),
            },
        ],
        &90,
        &14,
        &vec![&env, Address::generate(&env), Address::generate(&env)],
        &2,
        &None,
        &0,
    );

    (env, contract_id, owner, token_address, source_id)
}

#[test]
fn clone_copies_beneficiaries_guardians_and_periods() {
    let (env, contract_id, owner, token_address, source_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );

    let source = client.get_will(&source_id);
    let cloned = client.get_will(&clone_id);

    assert_eq!(cloned.beneficiaries, source.beneficiaries);
    assert_eq!(cloned.guardians, source.guardians);
    assert_eq!(cloned.guardian_threshold, source.guardian_threshold);
    assert_eq!(cloned.checkin_period_days, source.checkin_period_days);
    assert_eq!(cloned.grace_period_days, source.grace_period_days);
    assert_eq!(cloned.keeper_bounty_bps, source.keeper_bounty_bps);
    assert_eq!(cloned.owner, owner);

    // The clone is funded independently of the source: it holds only the
    // tokens supplied to `clone_will`.
    assert_eq!(cloned.balance, 500_000);
    assert_eq!(source.balance, 1_000_000);
}

#[test]
fn clone_gets_a_fresh_id_and_checkin_deadline() {
    let (env, contract_id, owner, token_address, source_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // Let the source's check-in clock run for a while before cloning.
    env.ledger().with_mut(|l| l.timestamp += 30 * DAY);
    let clone_time = env.ledger().timestamp();

    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );

    assert_ne!(clone_id, source_id, "a clone must get a fresh id");

    let source = client.get_will(&source_id);
    let cloned = client.get_will(&clone_id);

    assert_eq!(cloned.id, clone_id);
    assert_eq!(
        cloned.last_checkin, clone_time,
        "the clone's check-in clock must start at clone time, not inherit the source's"
    );
    assert_ne!(cloned.last_checkin, source.last_checkin);
    assert_eq!(cloned.status, WillStatus::Active);
    assert_eq!(cloned.trigger_time, None);
    assert_eq!(cloned.confirmation_deadline, None);

    // The clone is reachable from the owner's index alongside the source.
    let mut saw_source = false;
    let mut saw_clone = false;
    for will in client.get_wills_by_owner(&owner, &None, &10).iter() {
        saw_source |= will.id == source_id;
        saw_clone |= will.id == clone_id;
    }
    assert!(saw_source, "the source will is missing from the owner index");
    assert!(saw_clone, "the cloned will is missing from the owner index");
}

#[test]
fn clone_does_not_yet_increment_active_will_count() {
    let (env, contract_id, owner, token_address, source_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    let before = client.get_protocol_stats().active_will_count;

    client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );

    // #191 landed (see regression_test::issue_191_clone_will_increments_active_count):
    // clone_will now correctly increments active_will_count for the new will.
    assert_eq!(
        client.get_protocol_stats().active_will_count,
        before + 1,
        "clone_will must increment active_will_count (#191)"
    );
}
