#![cfg(test)]

//! Regression tests for issues #588-#591.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    grace_boundary, Allocation, Beneficiary, WillContract, WillContractClient, WillError,
    WillStatus,
};

const DAY: u64 = 86_400;

/// Creates one Active will for `owner` and returns its id.
fn create_one(env: &Env, client: &WillContractClient, owner: &Address, token: &Address) -> u64 {
    let beneficiary = Address::generate(env);
    client.create_will(
        owner,
        &vec![env, (token.clone(), 1_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env],
        &0,
        &None,
        &0,
    )
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000_000);
    let contract_id = env.register(WillContract, ());
    (env, contract_id, owner, token)
}

#[test]
fn grace_predicates_are_exact_complements() {
    for (now, deadline) in [(9u64, 10u64), (10, 10), (11, 10)] {
        assert_ne!(
            grace_boundary::grace_elapsed(now, deadline),
            grace_boundary::owner_window_open(now, deadline)
        );
    }
    assert!(!grace_boundary::grace_elapsed(10, 10));
    assert!(grace_boundary::grace_elapsed(11, 10));
}

#[test]
fn boundary_second_only_owner_succeeds_and_release_is_rejected() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    let t0 = env.ledger().timestamp();
    let will_id = create_one(&env, &client, &owner, &token);

    env.ledger().set_timestamp(t0 + 91 * DAY);
    client.trigger_will(&will_id);
    let deadline = t0 + 91 * DAY + 7 * DAY;

    env.ledger().set_timestamp(deadline);
    let err = client
        .try_release_inheritance(&will_id, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::GracePeriodNotExpired.into());
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);

    client.emergency_checkin(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

#[test]
fn one_second_after_boundary_only_release_succeeds() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    let t0 = env.ledger().timestamp();
    let will_id = create_one(&env, &client, &owner, &token);

    env.ledger().set_timestamp(t0 + 91 * DAY);
    client.trigger_will(&will_id);
    let deadline = t0 + 91 * DAY + 7 * DAY;

    env.ledger().set_timestamp(deadline + 1);
    let err = client
        .try_emergency_checkin(&will_id, &owner)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::GracePeriodExpired.into());
    client.release_inheritance(&will_id, &None);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

#[test]
fn batch_check_in_with_invalid_middle_element_changes_nothing() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    let first = create_one(&env, &client, &owner, &token);
    let last = create_one(&env, &client, &owner, &token);
    let before_first = client.get_will(&first).last_checkin;
    let before_last = client.get_will(&last).last_checkin;

    env.ledger().set_timestamp(1_700_000_000 + DAY);
    let missing: u64 = 9_999;
    let err = client
        .try_batch_check_in(&vec![&env, first, missing, last], &owner)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::WillNotFound.into());
    assert_eq!(client.get_will(&first).last_checkin, before_first);
    assert_eq!(client.get_will(&last).last_checkin, before_last);
}

#[test]
fn batch_check_in_rejects_non_active_middle_element() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    let first = create_one(&env, &client, &owner, &token);
    let middle = create_one(&env, &client, &owner, &token);
    let last = create_one(&env, &client, &owner, &token);
    let t0 = env.ledger().timestamp();

    env.ledger().set_timestamp(t0 + 91 * DAY);
    client.trigger_will(&middle);
    let before_first = client.get_will(&first).last_checkin;

    let err = client
        .try_batch_check_in(&vec![&env, first, middle, last], &owner)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::WillNotActive.into());
    assert_eq!(client.get_will(&first).last_checkin, before_first);
}

#[test]
fn owner_pagination_is_stable_when_a_will_is_added_mid_pagination() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    let a = create_one(&env, &client, &owner, &token);
    let b = create_one(&env, &client, &owner, &token);
    let c = create_one(&env, &client, &owner, &token);

    let page1 = client.get_wills_by_owner(&owner, &None, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().id, a);
    assert_eq!(page1.get(1).unwrap().id, b);

    // The collection grows between page requests.
    let d = create_one(&env, &client, &owner, &token);

    let page2 = client.get_wills_by_owner(&owner, &Some(b), &10);
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().id, c);
    assert_eq!(page2.get(1).unwrap().id, d);

    // A cursor past the last id yields an empty page rather than an error.
    let page3 = client.get_wills_by_owner(&owner, &Some(d), &10);
    assert_eq!(page3.len(), 0);
}
