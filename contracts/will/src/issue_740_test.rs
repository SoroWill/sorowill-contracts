#![cfg(test)]

//! Tests for issue #740: the guardian vote-expiry window is configurable per
//! will. The default follows `grace_period_days`, an owner can override it
//! while the will is `Active`, and the override changes when a vote lapses.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    (env, client, owner, token)
}

/// Creates an active will with a 7-day grace period and two guardians
/// (threshold 2), so a single guardian vote never triggers release.
fn create_will_with_guardians(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
) -> (u64, Address, Address) {
    let beneficiary = Address::generate(env);
    let guardian_1 = Address::generate(env);
    let guardian_2 = Address::generate(env);

    let will_id = client.create_will(
        owner,
        &vec![env, (token.clone(), 1_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env, guardian_1.clone(), guardian_2.clone()],
        &2,
        &None,
        &0,
    );
    client.accept_guardian_role(&will_id, &guardian_1);
    client.accept_guardian_role(&will_id, &guardian_2);
    // Let the guardian-list cooldown elapse before any vote is cast.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    (will_id, guardian_1, guardian_2)
}

#[test]
fn default_expiry_follows_grace_period() {
    let (env, client, owner, token) = setup();
    let (will_id, _, _) = create_will_with_guardians(&env, &client, &owner, &token);

    assert_eq!(client.get_guardian_vote_expiry(&will_id), 7);
    assert_eq!(client.get_will(&will_id).grace_period_days, 7);
}

#[test]
fn set_override_is_stored_per_will_and_leaves_grace_untouched() {
    let (env, client, owner, token) = setup();
    let (will_a, _, _) = create_will_with_guardians(&env, &client, &owner, &token);
    let (will_b, _, _) = create_will_with_guardians(&env, &client, &owner, &token);

    client.set_guardian_vote_expiry(&will_a, &owner, &Some(3));

    assert_eq!(client.get_guardian_vote_expiry(&will_a), 3);
    // The other will keeps the default, and the grace period is unchanged.
    assert_eq!(client.get_guardian_vote_expiry(&will_b), 7);
    assert_eq!(client.get_will(&will_a).grace_period_days, 7);
}

#[test]
fn clearing_override_restores_grace_default() {
    let (env, client, owner, token) = setup();
    let (will_id, _, _) = create_will_with_guardians(&env, &client, &owner, &token);

    client.set_guardian_vote_expiry(&will_id, &owner, &Some(2));
    assert_eq!(client.get_guardian_vote_expiry(&will_id), 2);

    client.set_guardian_vote_expiry(&will_id, &owner, &None);
    assert_eq!(client.get_guardian_vote_expiry(&will_id), 7);
}

#[test]
fn out_of_range_expiry_is_rejected() {
    let (env, client, owner, token) = setup();
    let (will_id, _, _) = create_will_with_guardians(&env, &client, &owner, &token);

    assert!(client
        .try_set_guardian_vote_expiry(&will_id, &owner, &Some(0))
        .is_err());
    assert!(client
        .try_set_guardian_vote_expiry(&will_id, &owner, &Some(u64::MAX))
        .is_err());
    // A rejected call leaves the default in place.
    assert_eq!(client.get_guardian_vote_expiry(&will_id), 7);
}

#[test]
fn only_owner_can_set_expiry() {
    let (env, client, owner, token) = setup();
    let (will_id, _, _) = create_will_with_guardians(&env, &client, &owner, &token);
    let stranger = Address::generate(&env);

    assert!(client
        .try_set_guardian_vote_expiry(&will_id, &stranger, &Some(3))
        .is_err());
    assert_eq!(client.get_guardian_vote_expiry(&will_id), 7);
}

#[test]
fn expiry_cannot_change_once_triggered() {
    let (env, client, owner, token) = setup();
    let (will_id, _, _) = create_will_with_guardians(&env, &client, &owner, &token);

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    assert!(client
        .try_set_guardian_vote_expiry(&will_id, &owner, &Some(3))
        .is_err());
    assert_eq!(client.get_guardian_vote_expiry(&will_id), 7);
}

#[test]
fn shorter_override_lapses_a_vote_that_default_would_keep() {
    let (env, client, owner, token) = setup();
    let (will_id, guardian_1, _) = create_will_with_guardians(&env, &client, &owner, &token);
    client.set_guardian_vote_expiry(&will_id, &owner, &Some(2));

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Deceased);
    assert!(client.get_guardian_vote_status(&will_id, &guardian_1).is_some());

    // Three days later the 2-day window has passed, so the vote has lapsed and
    // the guardian may vote again. Under the 7-day default it would still count.
    env.ledger().with_mut(|l| l.timestamp += 3 * DAY);
    assert!(client.get_guardian_vote_status(&will_id, &guardian_1).is_none());
    assert!(client
        .try_guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Deceased)
        .is_ok());
}

#[test]
fn default_window_keeps_vote_that_override_would_lapse() {
    let (env, client, owner, token) = setup();
    let (will_id, guardian_1, _) = create_will_with_guardians(&env, &client, &owner, &token);

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Deceased);

    env.ledger().with_mut(|l| l.timestamp += 3 * DAY);
    assert!(client.get_guardian_vote_status(&will_id, &guardian_1).is_some());
    // Still within the 7-day default, so a second vote is a duplicate.
    assert!(client
        .try_guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Deceased)
        .is_err());
}
