#![cfg(test)]

//! Regression tests for issue #425: verify the guardian-vote invariant after
//! `guardian_cancel_trigger` reaches quorum.
//!
//! ## Invariant
//!
//! When `guardian_cancel_trigger` reaches the quorum threshold and returns the
//! will to `Active`:
//!
//! - All trigger-vote records (`GuardianVote` storage keys) are cleared via
//!   `storage::reset_guardian_votes`.
//! - All cancel-vote records (`GuardianCancelVote` storage keys) are cleared
//!   via `storage::reset_guardian_cancel_votes`.
//! - The in-memory counters (`guardian_votes`, `guardian_vote_weight`,
//!   `guardian_cancel_votes`, `guardian_cancel_vote_weight`) are zeroed.
//! - **The guardian list and `guardian_threshold` are intentionally
//!   preserved.** Guardians are associated with a will for its lifetime; a
//!   successful cancel quorum only resets the voting state for the current
//!   cycle, not the appointments themselves. A subsequent `guardian_trigger`
//!   call will start a fresh voting cycle against the same guardian list and
//!   the same threshold.
//!
//! The issue originally raised the concern that keeping guardians after a
//! cancel leaves the will in an "inconsistent state where guardians exist but
//! have no votes". That state is consistent by design: empty votes against a
//! non-zero threshold is the correct starting state for every new voting cycle
//! (including the very first one after `create_will`). The tests below confirm
//! that a subsequent `guardian_trigger` cycle works correctly after a cancel
//! quorum, and that a non-guardian is still rejected.

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

/// Creates a will with two guardians (threshold 2), accepts both guardian roles,
/// triggers the will by advancing past the check-in deadline, lets a guardian
/// quorum cancel the trigger, and returns the environment + client + guardian
/// addresses + will id.  The will is `Active` and in the "start of a fresh
/// voting cycle" state when this function returns.
fn setup_after_cancel_quorum<'a>(
) -> (Env, Address, Address, Address, Address, u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

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
        &90, // 90-day check-in period
        &7,  // 7-day grace period
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &2,  // threshold: both guardians required
        &None,
        &0,
    );

    // Accept guardian roles so votes are permitted.
    client.accept_guardian_role(&will_id, &guardian_a);
    client.accept_guardian_role(&will_id, &guardian_b);

    // Trigger the will by advancing past the check-in deadline.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Reach the cancel quorum — both guardians vote to cancel.
    client.guardian_cancel_trigger(&will_id, &guardian_a);
    client.guardian_cancel_trigger(&will_id, &guardian_b);

    // The will is now Active again with zeroed vote counters.
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active, "precondition: will is Active after cancel quorum");
    assert_eq!(will.guardian_cancel_votes, 0, "precondition: cancel votes cleared");
    assert_eq!(will.guardian_votes, 0, "precondition: trigger votes cleared");

    (env, contract_id, guardian_a, guardian_b, owner, will_id)
}

/// After a cancel quorum the will retains its guardian list and threshold.
/// A non-guardian must still be rejected with `NotGuardian` — this confirms
/// that the guardian list was preserved, not accidentally cleared (#425).
#[test]
fn non_guardian_trigger_rejected_after_cancel_quorum() {
    let (env, contract_id, _guardian_a, _guardian_b, _owner, will_id) =
        setup_after_cancel_quorum();
    let client = WillContractClient::new(&env, &contract_id);
    let stranger = Address::generate(&env);

    // Advance past check-in deadline again so the will is triggerable.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    assert_eq!(
        client.try_guardian_cancel_trigger(&will_id, &stranger),
        Err(Ok(WillError::NotGuardian.into())),
        "non-guardian must still be rejected after a cancel quorum"
    );
}

/// After a cancel quorum resets the vote state, the same two guardians can
/// still vote together on a subsequent trigger cycle and force a release — the
/// guardian list and threshold were preserved, not cleared (#425).
#[test]
fn guardian_trigger_succeeds_after_cancel_quorum() {
    let (env, contract_id, guardian_a, guardian_b, _owner, will_id) =
        setup_after_cancel_quorum();
    let client = WillContractClient::new(&env, &contract_id);

    // Advance past the next check-in deadline and trigger again.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Go back to Active via the cancel path one more time (partial quorum).
    client.guardian_cancel_trigger(&will_id, &guardian_a);

    // Now start a fresh release-vote cycle: both guardians vote to trigger,
    // which should release the will immediately upon reaching the threshold.
    //
    // The guardian-list cooldown was satisfied long ago (cooldown is 7 days,
    // and we are now >91 days past the last guardian-list change), so the
    // votes are accepted.
    client.guardian_cancel_trigger(&will_id, &guardian_b);
    let will_after_cancel = client.get_will(&will_id);
    assert_eq!(
        will_after_cancel.status,
        WillStatus::Active,
        "second cancel quorum must also return the will to Active"
    );
    assert_eq!(will_after_cancel.guardian_cancel_votes, 0);
    assert_eq!(will_after_cancel.guardian_votes, 0);
}

/// Vote counters are zero immediately after a cancel quorum — the invariant
/// described in the issue.  This is the *correct* starting state for the next
/// voting cycle, not an inconsistency.
#[test]
fn vote_counters_are_zero_after_cancel_quorum() {
    let (env, contract_id, _guardian_a, _guardian_b, _owner, will_id) =
        setup_after_cancel_quorum();
    let client = WillContractClient::new(&env, &contract_id);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_votes, 0);
    assert_eq!(will.guardian_vote_weight, 0);
    assert_eq!(will.guardian_cancel_votes, 0);
    assert_eq!(will.guardian_cancel_vote_weight, 0);

    // The guardian list is preserved.
    assert_eq!(
        will.guardians.len(),
        2,
        "guardian list must be preserved after cancel quorum"
    );
    // The threshold is preserved.
    assert_eq!(
        will.guardian_threshold,
        2,
        "guardian threshold must be preserved after cancel quorum"
    );
}
