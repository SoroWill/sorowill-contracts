#![cfg(test)]

//! Regression coverage for `get_triggered_wills` (#212): asserts a will's id
//! appears in the triggered-wills index after `trigger_will`, and is
//! correctly removed after each of `emergency_checkin`, `release_inheritance`,
//! and `guardian_cancel_trigger`.

use soroban_sdk::{
    testutils::{storage::Persistent as _, Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::storage::{self, DataKey};
use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

/// Mirrors `DAY_IN_LEDGERS` in `storage.rs`: 86,400 s/day at a 5-second
/// average ledger close time.
const DAY_IN_LEDGERS: u32 = 17_280;

fn setup(env: &Env) -> (WillContractClient<'_>, Address, Address) {
    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);
    (client, owner, token_address)
}

fn create_will(
    env: &Env,
    client: &WillContractClient<'_>,
    owner: &Address,
    token_address: &Address,
    beneficiary: &Address,
    guardians: soroban_sdk::Vec<Address>,
    guardian_threshold: u32,
) -> u64 {
    client.create_will(
        owner,
        &vec![env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &guardians,
        &guardian_threshold,
        &None,
        &0,
    )
}

#[test]
fn trigger_will_adds_id_to_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    assert!(client.get_triggered_wills(&None, &50).is_empty());

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    let triggered = client.get_triggered_wills(&None, &50);
    assert_eq!(triggered.len(), 1);
    assert_eq!(triggered.get(0).unwrap(), will_id);
}

#[test]
fn emergency_checkin_removes_id_from_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills(&None, &50).len(), 1);

    client.emergency_checkin(&will_id, &owner);
    assert!(client.get_triggered_wills(&None, &50).is_empty());
}

#[test]
fn release_inheritance_removes_id_from_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills(&None, &50).len(), 1);

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);
    assert!(client.get_triggered_wills(&None, &50).is_empty());
}

#[test]
fn guardian_cancel_trigger_removes_id_from_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env, guardian.clone()],
        1,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills(&None, &50).len(), 1);

    client.accept_guardian_role(&will_id, &guardian);
    client.guardian_cancel_trigger(&will_id, &guardian);
    assert!(client.get_triggered_wills(&None, &50).is_empty());
}

// ── Issue #391: unindex_triggered_will must extend the TTL ────────────────

/// Reads the remaining TTL, in ledgers, of the `TriggeredWills` index entry.
///
/// Must be called from inside `env.as_contract` — persistent storage of a
/// registered contract is only addressable from the contract's own frame.
fn triggered_index_ttl(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || {
        env.storage().persistent().get_ttl(&DataKey::TriggeredWills)
    })
}

/// Regression test for issue #391: `unindex_triggered_will` rewrites the
/// `TriggeredWills` entry but used to return without calling `extend_ttl`,
/// unlike `index_triggered_will` and every other removal helper (fixed for the
/// beneficiary/owner index helpers in #69).
///
/// The consequence is a workload made up of *only* removals — wills leaving
/// `Triggered` without any will entering it — never renews the index entry, so
/// its TTL walks down to zero and takes the still-Triggered ids with it. This
/// test drives a pure-removal period and asserts the TTL is still healthy
/// afterwards.
#[test]
fn unindex_triggered_will_extends_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);

    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    // Get the will into Triggered so the index entry exists at all.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills(&None, &50).len(), 1);
    let ttl_at_index = triggered_index_ttl(&env, &contract_id);
    assert!(ttl_at_index > 0, "indexing must give the entry a live TTL");

    // Age the entry down: advance 40 days of ledgers, which is past
    // LIFETIME_THRESHOLD (30 days) but short of the 60-day BUMP_AMOUNT, so the
    // entry survives but its remaining TTL sits well below the bump target.
    env.ledger()
        .with_mut(|l| l.sequence_number += DAY_IN_LEDGERS * 40);
    let ttl_before_unindex = triggered_index_ttl(&env, &contract_id);
    assert!(
        ttl_before_unindex < ttl_at_index,
        "advancing the ledger must consume the entry's remaining TTL"
    );

    // A pure-removal period: emergency_checkin returns the will to Active and
    // unindexes it, with no other will entering the index.
    env.ledger().with_mut(|l| l.timestamp += DAY);
    client.emergency_checkin(&will_id, &owner);
    assert!(client.get_triggered_wills(&None, &50).is_empty());

    let ttl_after_unindex = triggered_index_ttl(&env, &contract_id);
    assert!(
        ttl_after_unindex > ttl_before_unindex,
        "unindex_triggered_will must extend the index TTL: expected it to rise \
         above {ttl_before_unindex} after the removal, got {ttl_after_unindex}",
    );
    assert!(
        ttl_after_unindex > DAY_IN_LEDGERS * 30,
        "the renewed TTL must clear the ~30-day LIFETIME_THRESHOLD so an \
         exclusively-pruned index never lapses",
    );
}

/// The early return for an id that is not in the index must stay a pure read:
/// no `set` and no `extend_ttl`, so an unindex that changes nothing costs
/// nothing. Asserted by ageing the entry and confirming an unindex of an absent
/// id leaves the TTL exactly where it was.
#[test]
fn unindex_triggered_will_leaves_ttl_untouched_when_id_is_absent() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);

    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills(&None, &50).len(), 1);

    env.ledger()
        .with_mut(|l| l.sequence_number += DAY_IN_LEDGERS * 40);
    let ttl_before = triggered_index_ttl(&env, &contract_id);

    // `will_id + 1` was never allocated, so it cannot be in the index.
    env.as_contract(&contract_id, || {
        storage::unindex_triggered_will(&env, will_id + 1);
    });

    assert_eq!(
        triggered_index_ttl(&env, &contract_id),
        ttl_before,
        "an unindex of an absent id must not renew the entry's TTL",
    );
    assert_eq!(
        client.get_triggered_wills(&None, &50).get(0).unwrap(),
        will_id,
        "the present id must be untouched by the no-op unindex",
    );
}
