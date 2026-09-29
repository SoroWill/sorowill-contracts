#![cfg(test)]

//! Regression coverage for issue #221: `archive_will` removes settled wills from
//! active owner and beneficiary indexes while rejecting in-flight lifecycle states.
//!
//! And for issue #393: archival also drops the will's `WillHistory` entry and
//! every `GuardianVote` / `GuardianCancelVote` entry belonging to its guardians,
//! rather than leaving them orphaned in persistent storage.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::storage::DataKey;
use crate::{Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, token_address)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

#[test]
fn archive_will_removes_released_will_from_owner_and_beneficiary_indexes() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    client.archive_will(&will_id);

    let owner_wills = client.get_wills_by_owner(&owner, &None, &10);
    assert!(
        owner_wills.is_empty(),
        "released wills must be removed from owner index"
    );

    let beneficiary_wills = client.get_wills_by_beneficiary(&beneficiary, &None, &10);
    assert!(
        beneficiary_wills.is_empty(),
        "released wills must be removed from beneficiary index"
    );

    // archive_will removes the will's storage entry entirely (see its doc
    // comment in lib.rs), so get_will on an archived id must now panic with
    // WillNotFound rather than return a lingering Released record.
    let result = client.try_get_will(&will_id);
    assert!(
        result.is_err(),
        "an archived will's storage entry must be gone"
    );
}

#[test]
fn archive_will_removes_cancelled_will_from_active_indexes() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    client.cancel_will(&will_id, &owner);
    client.archive_will(&will_id);

    assert!(client.get_wills_by_owner(&owner, &None, &10).is_empty());
    assert!(client
        .get_wills_by_beneficiary(&beneficiary, &None, &10)
        .is_empty());
}

/// Regression test for issue #331: archiving a will that is currently in
/// `Triggered` status must remove its id from the global `TriggeredWills`
/// index so that keepers iterating `get_triggered_wills()` never see a
/// dangling id pointing at an entry that no longer resolves.
#[test]
fn archive_triggered_will_removes_id_from_triggered_index() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Advance past the check-in deadline and trigger the will.
    advance(&env, 91);
    client.trigger_will(&will_id);

    // The will's id must now appear in the triggered-wills index.
    assert_eq!(
        client.get_triggered_wills(&None, &50).len(),
        1,
        "triggered will must be in index before archival"
    );

    // archive_will only accepts Released/Cancelled wills (see its doc
    // comment in lib.rs) -- a Triggered will must go through the grace
    // period and release_inheritance first, which already clears the
    // triggered-wills index on its own.
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);
    assert!(
        client.get_triggered_wills(&None, &50).is_empty(),
        "release_inheritance must remove the will's id from the triggered-wills index"
    );

    // Archiving the now-Released will must not resurrect the stale id.
    client.archive_will(&will_id);
    assert!(
        client.get_triggered_wills(&None, &50).is_empty(),
        "archiving a released will must not leave a dangling id in the triggered-wills index"
    );
}

#[test]
#[should_panic]
fn archive_will_rejects_active_will() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

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
        &vec![&env],
        &2,
        &None,
        &0,
    );

    client.archive_will(&will_id);
}

// ── Issue #393: archival drops history and guardian vote entries ─────────

/// Whether `key` is absent from persistent storage.
///
/// Must be called from inside `env.as_contract` — a registered contract's
/// persistent storage is only addressable from its own frame. Asserting
/// absence (rather than reading through a query) is the point: the orphaned
/// entries have no public reader left once the will is gone, so the only way to
/// prove they were removed is to look at the key itself.
fn key_exists(env: &Env, contract_id: &Address, key: &DataKey) -> bool {
    env.as_contract(contract_id, || env.storage().persistent().has(key))
}

/// Regression test for issue #393: `archive_will` moved the will to
/// `ArchivedWill` and cleared the owner/beneficiary/Triggered indexes, but left
/// the `WillHistory` entry and every `GuardianVote` / `GuardianCancelVote` entry
/// behind. Those keys then described a will `load_will` can no longer resolve,
/// while occupying ledger state until their TTL lapsed.
///
/// Asserts each key is present before archival and absent after, so the test
/// fails both if the cleanup regresses and if the setup silently stopped
/// creating the entries in the first place.
#[test]
fn archive_will_removes_history_and_guardian_vote_entries() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);
    let guardians: Vec<Address> = vec![&env, guardian_a.clone(), guardian_b.clone()];

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
        &guardians,
        &2,
        &None,
        &0,
    );
    for guardian in guardians.iter() {
        client.accept_guardian_role(&will_id, &guardian);
    }

    // Guardian A casts a release vote while the will is Active. With a
    // threshold of 2 this stays short of quorum, so no release happens — but
    // the `GuardianVote` entry it wrote is now in storage.
    advance(&env, 8); // clear the 7-day guardian-list cooldown
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Other);

    // The will then goes through the normal missed-check-in path to Triggered.
    advance(&env, 91);
    client.trigger_will(&will_id);

    // Guardian B casts a cancel vote. Alone, it is short of quorum too, so the
    // will stays Triggered and the `GuardianCancelVote` entry survives.
    client.guardian_cancel_trigger(&will_id, &guardian_b);

    // Sanity: the entries exist before archival, so the post-archive assertions
    // below are testing the cleanup rather than a no-op setup.
    let history_key = DataKey::WillHistory(will_id);
    let vote_a = DataKey::GuardianVote(will_id, guardian_a.clone());
    let cancel_b = DataKey::GuardianCancelVote(will_id, guardian_b.clone());
    assert!(key_exists(&env, &contract_id, &history_key));
    assert!(key_exists(&env, &contract_id, &vote_a));
    assert!(key_exists(&env, &contract_id, &cancel_b));

    // Releasing the will does not clear the vote rows — `distribute` leaves the
    // guardian counters and their storage entries alone — which is exactly the
    // state that used to strand them.
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);
    assert!(key_exists(&env, &contract_id, &vote_a));
    assert!(key_exists(&env, &contract_id, &cancel_b));

    client.archive_will(&will_id);

    assert!(
        !key_exists(&env, &contract_id, &history_key),
        "archival must drop the will's WillHistory entry"
    );
    assert!(
        !key_exists(&env, &contract_id, &vote_a),
        "archival must drop the will's GuardianVote entries"
    );
    assert!(
        !key_exists(&env, &contract_id, &cancel_b),
        "archival must drop the will's GuardianCancelVote entries"
    );

    // The query surface agrees: the trail is empty for an archived will, so
    // clients must not treat get_will_history as a post-archival recovery path.
    assert!(client.get_will_history(&will_id).is_empty());
}

/// Archiving a will with no guardians must still clear its history, and the
/// empty guardian sweep must be a no-op rather than a failure.
#[test]
fn archive_will_removes_history_when_there_are_no_guardians() {
    let (env, client, owner, token_address) = setup();

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
        &2,
        &None,
        &0,
    );

    assert!(
        !client.get_will_history(&will_id).is_empty(),
        "the will must have a trail to lose"
    );

    client.cancel_will(&will_id, &owner);
    client.archive_will(&will_id);

    assert!(client.get_will_history(&will_id).is_empty());
}
