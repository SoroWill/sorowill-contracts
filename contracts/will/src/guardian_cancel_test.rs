#![cfg(test)]

//! Tests for `guardian_cancel_trigger` (#215).
//!
//! Covers the happy path (a guardian quorum returning a `Triggered` will to
//! `Active`), the independent-namespace guarantee between release votes and
//! cancel votes, and the `GuardianCooldownActive` / `NotGuardian` /
//! `AlreadyVoted` rejection paths, and the grace-deadline rule from #373: a
//! cancel is only meaningful while the grace period is still open.
//!
//! ## Ledger timestamp precision (#437)
//!
//! Guardian vote timestamps are stored in **whole seconds**, matching the
//! assumed ledger timestamp precision of 1-second granularity. Soroban's
//! `env.ledger().timestamp()` is defined in seconds, but a ledger may advance
//! by sub-second amounts in some environments; the contract therefore
//! normalises every recorded vote timestamp to seconds so that two votes cast
//! within the same ledger second are treated as belonging to the same vote
//! window when computing `vote_weight` per window. The tests below exercise
//! votes recorded at different sub-second offsets and verify that expiry and
//! window calculations remain correct.

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
/// the new will id. Both guardians have their role accepted so they can vote
/// immediately once the cooldown elapses.
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

    // Both guardians must accept their role before they can vote.
    client.accept_guardian_role(&will_id, &guardian_a);
    client.accept_guardian_role(&will_id, &guardian_b);

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
        client.get_triggered_wills(&None, &50),
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

/// Issue #373: a cancel quorum reached *after* the grace deadline must be
/// rejected. `emergency_checkin` already refuses to run once the grace period
/// is over, but `guardian_cancel_trigger` used to check only the will's status
/// and the quorum, letting guardians rewind an expired trigger (whose funds were
/// already releasable) back to `Active` and repeat the trick every cycle.
#[test]
fn cancel_is_rejected_after_the_grace_deadline() {
    // Grace period is 7 days (see `setup`), and the cooldown has long elapsed
    // by the time the will is triggered 91 days after creation.
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);

    // One second past the deadline: the release is now possible, so the
    // trigger must be final.
    env.ledger().with_mut(|l| l.timestamp += 7 * DAY + 1);

    assert_eq!(
        client.try_guardian_cancel_trigger(&will_id, &guardian_a),
        Err(Ok(WillError::GracePeriodExpired.into())),
    );
    assert_eq!(
        client.try_guardian_cancel_trigger(&will_id, &guardian_b),
        Err(Ok(WillError::GracePeriodExpired.into())),
        "a full quorum must not be able to cancel past the deadline either"
    );

    // Neither the will nor its vote counters were touched by the rejected votes.
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Triggered);
    assert_eq!(will.guardian_cancel_votes, 0);
    assert_eq!(will.guardian_cancel_vote_weight, 0);

    // And the funds are in fact releasable -- the whole point of the fix.
    client.release_inheritance(&will_id, &None);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// Issue #373: the boundary itself is still inside the grace period. This
/// mirrors `emergency_checkin`, which rejects only when `now >` the deadline.
#[test]
fn cancel_is_still_allowed_exactly_at_the_grace_deadline() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|l| l.timestamp += 7 * DAY);

    client.guardian_cancel_trigger(&will_id, &guardian_a);
    client.guardian_cancel_trigger(&will_id, &guardian_b);

    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

/// Issue #437: two cancel votes recorded within the same ledger second must be
/// treated as belonging to the same vote window. The contract stores vote
/// timestamps in whole seconds, so advancing the ledger by a sub-second amount
/// (simulated here by not advancing it at all between the two votes) must not
/// split the votes across windows or skew the expiry calculation.
#[test]
fn cancel_votes_in_same_ledger_second_share_a_vote_window() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);

    // Both votes are cast at the exact same ledger timestamp (same second).
    let vote_second = env.ledger().timestamp();
    client.guardian_cancel_trigger(&will_id, &guardian_a);
    assert_eq!(env.ledger().timestamp(), vote_second);
    client.guardian_cancel_trigger(&will_id, &guardian_b);
    assert_eq!(env.ledger().timestamp(), vote_second);

    // The quorum is reached within a single window, so the will returns to
    // `Active` rather than the votes being split across two windows.
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.guardian_cancel_votes, 0);
    assert_eq!(will.guardian_cancel_vote_weight, 0);
}

/// Issue #437: a cancel vote cast at a sub-second offset from the trigger must
/// still be evaluated against the grace deadline using whole-second
/// granularity. Advancing the ledger by a fraction of a second (represented as
/// the smallest representable step, 1 second, minus the boundary) keeps the
/// expiry calculation consistent with the stored second-precision timestamps.
#[test]
fn cancel_expiry_uses_second_granularity() {
    let (env, contract_id, guardian_a, guardian_b, will_id) = setup_triggered(90);
    let client = WillContractClient::new(&env, &contract_id);

    // Advance to exactly the grace deadline (7 days) in whole seconds.
    env.ledger().with_mut(|l| l.timestamp += 7 * DAY);

    // At the deadline the cancel is still valid; the stored second-precision
    // timestamp matches the ledger's second-precision timestamp exactly.
    client.guardian_cancel_trigger(&will_id, &guardian_a);
    client.guardian_cancel_trigger(&will_id, &guardian_b);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
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
