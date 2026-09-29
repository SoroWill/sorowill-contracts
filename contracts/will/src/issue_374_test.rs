#![cfg(test)]

//! Regression tests for issue #374: `accept_guardian_role` and
//! `reject_guardian_role` used to work on any will status and had no consent
//! state machine, so a `Rejected` guardian could accept again and a guardian who
//! had already voted could reject while their vote weight stayed counted.
//!
//! Covered here:
//! - both entry points refuse `Released` and `Cancelled` (terminal) wills,
//! - a rejection is terminal (accepting afterwards panics with
//!   `InvalidConsentTransition`),
//! - re-accepting or re-rejecting an already-settled consent is a no-op,
//! - rejecting after a trigger vote withdraws the vote and its weight,
//! - a rejected guardian cannot vote until the owner re-appoints them.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, GuardianConsent, GuardianSpec, GuardianVoteReason,
    WillContract, WillContractClient, WillStatus,
};


const DAY: u64 = 86_400;

/// Creates a funded will with one beneficiary and two guardians of weight 1.
/// Returns the env, the client, the owner and the will id.
fn setup() -> (Env, WillContractClient<'static>, Address, u64) {
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
        &vec![&env, Address::generate(&env), Address::generate(&env)],
        &2,
        &None,
        &0,
    );

    (env, client, owner, will_id)
}

/// Returns the consent recorded for `guardian` on `will_id`.
fn consent_of(client: &WillContractClient, will_id: u64, guardian: &Address) -> GuardianConsent {
    client
        .get_will(&will_id)
        .guardians
        .iter()
        .find(|g| g.address == *guardian)
        .expect("guardian is on the will")
        .consent
}

/// Returns the address of the guardian at `index` on `will_id`.
fn guardian_at(client: &WillContractClient, will_id: u64, index: u32) -> Address {
    client.get_will(&will_id).guardians.get(index).unwrap().address
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn accept_guardian_role_rejects_a_released_will() {
    let (env, client, _owner, will_id) = setup();
    let first = guardian_at(&client, will_id, 0);
    let second = guardian_at(&client, will_id, 1);

    client.accept_guardian_role(&will_id, &first);
    client.accept_guardian_role(&will_id, &second);

    // Two guardian votes reach quorum and release the will.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.guardian_trigger(&will_id, &first, &GuardianVoteReason::Deceased);
    client.guardian_trigger(&will_id, &second, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);

    // A terminal will accepts no further consent changes.
    client.accept_guardian_role(&will_id, &first);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn reject_guardian_role_rejects_a_cancelled_will() {
    let (_env, client, owner, will_id) = setup();
    let guardian = guardian_at(&client, will_id, 0);

    client.cancel_will(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Cancelled);

    client.reject_guardian_role(&will_id, &guardian);
}

#[test]
#[should_panic(expected = "Error(Contract, #40)")]
fn a_rejected_guardian_cannot_accept_again() {
    let (_env, client, _owner, will_id) = setup();
    let guardian = guardian_at(&client, will_id, 0);

    client.reject_guardian_role(&will_id, &guardian);
    assert_eq!(consent_of(&client, will_id, &guardian), GuardianConsent::Rejected);

    // `Rejected` is terminal: the owner must re-appoint the guardian through
    // `update_guardians_weighted` to give them a fresh `Pending` consent.
    client.accept_guardian_role(&will_id, &guardian);
}

#[test]
fn re_accepting_or_re_rejecting_is_a_no_op() {
    let (_env, client, _owner, will_id) = setup();
    let guardian = guardian_at(&client, will_id, 0);

    client.accept_guardian_role(&will_id, &guardian);
    client.accept_guardian_role(&will_id, &guardian);
    assert_eq!(consent_of(&client, will_id, &guardian), GuardianConsent::Accepted);

    client.reject_guardian_role(&will_id, &guardian);
    client.reject_guardian_role(&will_id, &guardian);
    assert_eq!(consent_of(&client, will_id, &guardian), GuardianConsent::Rejected);
}

#[test]
fn rejecting_after_voting_withdraws_the_vote_and_its_weight() {
    let (env, client, owner, will_id) = setup();
    let guardian = guardian_at(&client, will_id, 0);
    let other = guardian_at(&client, will_id, 1);

    // Give the withdrawing guardian a weight of 2 so the deduction is visible.
    client.update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: guardian.clone(),
                weight: 2,
            },
            GuardianSpec {
                address: other.clone(),
                weight: 1,
            },
        ],
        &Some(3),
    );

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.accept_guardian_role(&will_id, &guardian);
    client.guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Incapacitated);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_vote_weight, 2);
    assert_eq!(will.guardian_votes, 1);

    // Withdrawing consent takes the already-cast vote back out of the tally, so
    // the guardian can no longer push the will towards quorum.
    client.reject_guardian_role(&will_id, &guardian);
    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_vote_weight, 0, "vote weight must be withdrawn");
    assert_eq!(will.guardian_votes, 0, "vote count must be withdrawn");
    assert_eq!(client.get_guardian_vote_status(&will_id, &guardian), None);
    assert_eq!(consent_of(&client, will_id, &guardian), GuardianConsent::Rejected);
}

#[test]
#[should_panic(expected = "Error(Contract, #38)")]
fn a_rejected_guardian_cannot_vote() {
    let (env, client, _owner, will_id) = setup();
    let guardian = guardian_at(&client, will_id, 0);

    client.accept_guardian_role(&will_id, &guardian);
    client.reject_guardian_role(&will_id, &guardian);

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    // `GuardianNotConsented`: the withdrawn guardian may no longer vote.
    client.guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Deceased);
}

#[test]
fn a_re_appointed_guardian_starts_from_pending_and_can_vote_again() {
    let (env, client, owner, will_id) = setup();
    let guardian = guardian_at(&client, will_id, 0);

    client.accept_guardian_role(&will_id, &guardian);
    client.reject_guardian_role(&will_id, &guardian);

    // Re-appointed by the owner: consent resets to `Pending` with the weight
    // preserved, and only after a fresh acceptance can the guardian vote again.
    client.update_guardians_weighted(
        &will_id,
        &owner,
        &vec![&env, GuardianSpec { address: guardian.clone(), weight: 1 }],
        &Some(1),
    );
    assert_eq!(consent_of(&client, will_id, &guardian), GuardianConsent::Pending);

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.accept_guardian_role(&will_id, &guardian);
    client.guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 1);
}
