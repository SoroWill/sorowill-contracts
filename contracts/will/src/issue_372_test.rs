#![cfg(test)]

//! Regression tests for issue #372: an expired guardian vote must stop
//! counting towards the quorum.
//!
//! `has_guardian_voted` reports a vote older than the will's grace period as
//! absent, so the same guardian may vote again — but both `guardian_trigger`
//! and `guardian_cancel_trigger` used to *add* to the persisted
//! `guardian_vote_weight` / `guardian_votes` counters, which nothing ever
//! decremented. The counters therefore counted the same guardian twice, and one
//! guardian could reach a threshold of 2 alone by voting once per expiry
//! window. Both entry points now recompute the tallies from the vote records
//! that are still live.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillError,
    WillStatus,
};

const DAY: u64 = 86_400;

/// Grace period used for every will in this file; it doubles as the guardian
/// vote expiry window (`expiry_days = will.grace_period_days`).
const GRACE_DAYS: u64 = 7;

/// Registers the contract and creates a funded will with two guardians, both of
/// weight 1, and a threshold of 2. Both guardians have accepted their role.
fn setup() -> (Env, Address, Address, Address, u64) {
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
        &90,
        &GRACE_DAYS,
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &2,
        &None,
        &0,
    );

    client.accept_guardian_role(&will_id, &guardian_a);
    client.accept_guardian_role(&will_id, &guardian_b);

    (env, contract_id, guardian_a, guardian_b, will_id)
}

/// Advances past the 7-day guardian-list cooldown so a vote is accepted.
fn past_cooldown(env: &Env) {
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
}

/// Misses the check-in deadline and triggers the will, leaving it `Triggered`.
fn trigger(env: &Env, client: &WillContractClient, will_id: u64) {
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
}

/// Issue #372: one guardian voting twice across an expiry window must not reach
/// a threshold of 2 on their own. The will must stay `Active` and the counters
/// must reflect the single live vote.
#[test]
fn repeated_release_votes_across_an_expiry_window_cannot_reach_the_threshold() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    past_cooldown(&env);

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 1);

    // One second past the expiry window: the first vote is now stale and the
    // same guardian is allowed to vote again.
    env.ledger()
        .with_mut(|l| l.timestamp += GRACE_DAYS * DAY + 1);
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased);

    let will = client.get_will(&will_id);
    assert_eq!(
        will.status,
        WillStatus::Active,
        "one guardian voting twice across an expiry window must not release the will"
    );
    assert_eq!(
        will.guardian_vote_weight, 1,
        "the stale vote must be replaced, not added to"
    );
    assert_eq!(will.guardian_votes, 1);

    // A genuinely second guardian still reaches quorum, so the recount has not
    // broken the happy path.
    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// The same scenario as above, for the cancel namespace.
///
/// Note the cancel side no longer has a window in which a vote can expire
/// *and* remain acceptable: the vote expiry window is the grace period, and
/// since #373 a cancel quorum is rejected once that same grace deadline
/// passes. So the second cancel attempt is expected to fail with
/// `GracePeriodExpired` rather than to be recorded — the will must stay
/// `Triggered`, which is the property this test is really about: one guardian
/// can never return the will to `Active` on their own.
#[test]
fn a_cancel_quorum_cannot_rewind_a_will_past_its_grace_period() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    trigger(&env, &client, will_id);

    client.guardian_cancel_trigger(&will_id, &guardian_a);
    assert_eq!(client.get_will(&will_id).guardian_cancel_vote_weight, 1);

    // Move past the grace period (the cancel deadline) and let the very same
    // guardian try again.
    env.ledger()
        .with_mut(|l| l.timestamp += GRACE_DAYS * DAY + 1);
    let err = client
        .try_guardian_cancel_trigger(&will_id, &guardian_a)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::GracePeriodExpired.into());

    let will = client.get_will(&will_id);
    assert_eq!(
        will.status,
        WillStatus::Triggered,
        "the trigger must stand once the grace period has elapsed"
    );
    assert_eq!(
        will.guardian_cancel_vote_weight, 1,
        "the rejected second vote must not have been recorded"
    );
    assert_eq!(will.guardian_cancel_votes, 1);

    // The other guardian cannot cancel either — the grace period is over for
    // everyone, which is the whole point of holding guardians to one deadline.
    assert!(client
        .try_guardian_cancel_trigger(&will_id, &guardian_b)
        .is_err());
}

/// Two votes inside one expiry window still accumulate: only *expired* records
/// are dropped, live ones keep counting.
#[test]
fn votes_within_the_expiry_window_still_accumulate() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    past_cooldown(&env);

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased);
    env.ledger().with_mut(|l| l.timestamp += GRACE_DAYS * DAY);
    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Deceased);

    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// A vote is still live right up to its expiry boundary, so two guardians
/// voting `GRACE_DAYS` apart reach quorum — the recount must not prune a record
/// that `has_guardian_voted` still considers present.
#[test]
fn a_vote_at_the_expiry_boundary_is_still_live() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    past_cooldown(&env);

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Incapacitated);
    env.ledger().with_mut(|l| l.timestamp += GRACE_DAYS * DAY);
    let will = client.get_will(&will_id);
    assert_eq!(
        (will.guardian_vote_weight, will.guardian_votes),
        (1, 1),
        "a vote exactly at its expiry boundary must still be tallied"
    );

    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// The `AlreadyVoted` guard is unchanged for a vote inside its window: the
/// recount must not turn a duplicate vote into a fresh one.
#[test]
fn a_second_vote_inside_the_window_is_still_rejected() {
    let (env, contract_id, guardian_a, _guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    past_cooldown(&env);

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased);

    assert_eq!(
        client.try_guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased),
        Err(Ok(WillError::AlreadyVoted.into())),
    );
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 1);
}

/// Regression guard for the `recount_*` storage helpers themselves: an expired
/// record is removed and contributes nothing, a live one is left in place.
#[test]
fn expired_vote_records_are_deleted_and_live_ones_are_kept() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    past_cooldown(&env);

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Deceased);

    env.as_contract(&contract_id, || {
        let will = crate::storage::load_will(&env, will_id).unwrap();
        let now = env.ledger().timestamp();

        // Still inside the window: one live vote, tallied as 1 and kept.
        let (weight, votes) = crate::storage::recount_guardian_votes(&env, &will, now, GRACE_DAYS);
        assert_eq!((weight, votes), (1, 1));
        assert!(crate::storage::get_guardian_vote(&env, will_id, &guardian_a).is_some());

        // Past the window: the record is gone from storage and contributes 0.
        let later = now + GRACE_DAYS * DAY + 1;
        let (weight, votes) =
            crate::storage::recount_guardian_votes(&env, &will, later, GRACE_DAYS);
        assert_eq!((weight, votes), (0, 0));
        assert!(crate::storage::get_guardian_vote(&env, will_id, &guardian_a).is_none());
    });

    // A later vote by a *different* guardian therefore starts from zero, not
    // from the stale record's weight.
    env.ledger()
        .with_mut(|l| l.timestamp += GRACE_DAYS * DAY + 1);
    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Other);

    let will = client.get_will(&will_id);
    assert_eq!((will.guardian_vote_weight, will.guardian_votes), (1, 1));
    assert_eq!(will.status, WillStatus::Active);
}

/// A partially-recounted `Triggered` will stays in the triggered index, so
/// index-driven queries still find it.
#[test]
fn triggered_index_is_unaffected_by_recounting() {
    let (env, contract_id, guardian_a, _guardian_b, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    trigger(&env, &client, will_id);
    assert_eq!(client.get_triggered_wills(&None, &50), vec![&env, will_id]);

    client.guardian_cancel_trigger(&will_id, &guardian_a);
    assert_eq!(client.get_triggered_wills(&None, &50), vec![&env, will_id]);
}
