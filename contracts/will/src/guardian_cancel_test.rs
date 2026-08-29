#![cfg(test)]

//! Tests for `guardian_cancel_trigger` (#215).
//!
//! Covers the happy path (a guardian quorum returning a `Triggered` will to
//! `Active`), the independent-namespace guarantee between release votes and
//! cancel votes, and the `GuardianCooldownActive` / `NotGuardian` /
//! `AlreadyVoted` rejection paths.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillError,
    WillStatus,
};

const DAY: u64 = 86_400;

/// Registers the contract and creates a funded will with two guardians and a
/// threshold of 2. Returns the env, the contract address, both guardians and
/// the new will id.
fn setup(checkin_period_days: u64) -> (Env, Address, Address, Address, u64) {
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

    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &checkin_period_days,
        &7,
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &2,
        &None,
        &0,
    );

    (env, contract_id, guardian_a, guardian_b, will_id)
}

/// As [`setup`], but also advances past the check-in deadline and triggers the
/// will, leaving it `Triggered`.
fn setup_triggered(checkin_period_days: u64) -> (Env, Address, Address, Address, u64) {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup(checkin_period_days);
    env.ledger()
        .with_mut(|l| l.timestamp += (checkin_period_days + 1) * DAY);
    WillContractClient::new(&env, &contract_id).trigger_will(&will_id);
    (env, contract_id, guardian_a, guardian_b, will_id)
}

#[test]
fn cancel_quorum_returns_triggered_will_to_active() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);

    client.guardian_cancel_trigger(&will_id, &guardian_a);
    let after_first = client.get_will(&will_id);
    assert_eq!(
        after_first.status,
        WillStatus::Triggered,
        "a single cancel vote must not reach the threshold of 2"
    );
    assert_eq!(after_first.guardian_cancel_votes, 1);

    client.guardian_cancel_trigger(&will_id, &guardian_b);

    let after_quorum = client.get_will(&will_id);
    assert_eq!(after_quorum.status, WillStatus::Active);
    assert_eq!(after_quorum.trigger_time, None);
    assert_eq!(after_quorum.last_checkin, env.ledger().timestamp());
    assert_eq!(after_quorum.guardian_cancel_votes, 0);
    assert_eq!(after_quorum.guardian_cancel_vote_weight, 0);
    assert_eq!(
        client.get_triggered_wills(),
        Vec::<u64>::new(&env),
        "a cancelled trigger must be removed from the triggered index"
    );
}

#[test]
fn release_vote_and_cancel_vote_from_same_guardian_are_independent() {
    let (env, contract_id, guardian_a, _guardian_b, will_id) = setup(90);
    let client = WillContractClient::new(&env, &contract_id);

    // Cast a release vote (one short of the threshold) once the guardian-list
    // cooldown has elapsed, then let the will trigger on a missed check-in.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Unreachable);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);

    env.ledger().with_mut(|l| l.timestamp += 83 * DAY);
    client.trigger_will(&will_id);

    // The same guardian may still cast a cancel vote: the two namespaces are
    // deduplicated independently.
    client.guardian_cancel_trigger(&will_id, &guardian_a);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_votes, 1, "the release vote must be untouched");
    assert_eq!(will.guardian_cancel_votes, 1);
    assert_eq!(will.status, WillStatus::Triggered);
}

#[test]
fn cancel_is_rejected_during_the_guardian_list_cooldown() {
    // A 1-day check-in period means the will is triggered two days after
    // creation — well inside the 7-day guardian-list cooldown.
    let (env, contract_id, guardian_a, _guardian_b, will_id) = setup_triggered(1);
    let client = WillContractClient::new(&env, &contract_id);

    assert_eq!(
        client.try_guardian_cancel_trigger(&will_id, &guardian_a),
        Err(Ok(WillError::GuardianCooldownActive.into()))
    );
}

#[test]
fn cancel_is_rejected_for_a_non_guardian() {
    let (env, contract_id, _guardian_a, _guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_guardian_cancel_trigger(&will_id, &stranger),
        Err(Ok(WillError::NotGuardian.into()))
    );
}

#[test]
fn cancel_is_rejected_when_the_same_guardian_votes_twice() {
    let (env, contract_id, guardian_a, _guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);

    client.guardian_cancel_trigger(&will_id, &guardian_a);

    assert_eq!(
        client.try_guardian_cancel_trigger(&will_id, &guardian_a),
        Err(Ok(WillError::AlreadyVoted.into()))
    );
}

#[test]
fn weighted_guardian_voting_reaches_quorum_based_on_vote_weight() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_a.clone()],
        &1,
        &None,
        &0,
    );

    // Update guardian with weight 2 and threshold 2
    let specs = vec![
        &env,
        crate::GuardianSpec {
            address: guardian_a.clone(),
            weight: 2,
        },
    ];
    client.update_guardians_weighted(&will_id, &owner, &specs, &Some(2));
    client.accept_guardian_role(&will_id, &guardian_a);

    // Advance past cooldown
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    // Guardian A votes; weight = 2 >= threshold (2), so trigger succeeds in 1 vote
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
}
