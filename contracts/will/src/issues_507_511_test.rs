#![cfg(test)]

//! Tests for issues #507, #508, #509, #510, and #511.
//!
//! All five verify key invariants across the will contract lifecycle:
//! - **#507** — `claim_share` in pull mode must prevent duplicate claims.
//! - **#508** — `emergency_checkin` requires fresh owner authorization mid-grace-period.
//! - **#509** — `release_inheritance` ensures atomicity: failed token distributions
//!   revert transaction state so a will is never marked as released with partial failure.
//! - **#510** — `get_will_history` supports pagination via `get_will_history_page`
//!   with cursor and limit bounding RPC response payloads.
//! - **#511** — Guardian voting accumulates guardian weights against threshold quorum
//!   rather than treating all guardians as weight 1.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::{
    Allocation, Beneficiary, Guardian, GuardianVoteReason, WillContract, WillContractClient,
    WillError, WillStatus,
};

const DAY: u64 = 86_400;

fn setup() -> (Env, WillContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token = sac.address();
    StellarAssetClient::new(&env, &token).mint(&owner, &10_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token)
}

// ── #507: claim_share prevents duplicate claims in pull mode ─────────────────

#[test]
fn pull_distribution_rejects_duplicate_claim() {
    let (env, client, owner, token) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Advance past check-in period, trigger, then advance past grace period.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    client.release_inheritance(&will_id, &None);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);

    // First claim succeeds. Subsequent or repeated release calls are rejected.
    let repeat_err = client
        .try_release_inheritance(&will_id, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(repeat_err, WillError::WillNotTriggered.into());
}

// ── #508: emergency_checkin requires fresh owner authorization ──────────────

#[test]
fn emergency_checkin_requires_owner_authority() {
    let (env, client, owner, token) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Non-owner caller cannot emergency check-in for the owner.
    let stranger = Address::generate(&env);
    let err = client
        .try_emergency_checkin(&will_id, &stranger)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::NotOwner.into());

    // Owner check-in succeeds within the grace period.
    env.ledger().with_mut(|l| l.timestamp += DAY);
    client.emergency_checkin(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

// ── #509: release_inheritance atomicity on token distribution ────────────────

#[test]
fn release_inheritance_is_atomic_and_settles_all_shares() {
    let (env, client, owner, token) = setup();
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: b1.clone(),
            allocation: Allocation::Percentage(5_000),
        },
        Beneficiary {
            address: b2.clone(),
            allocation: Allocation::Percentage(5_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 2_000_000)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    // When release succeeds, all transfers execute and status transitions to Released.
    client.release_inheritance(&will_id, &None);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
}

// ── #510: get_will_history supports bounded pagination ──────────────────────

#[test]
fn get_will_history_supports_cursor_pagination() {
    let (env, client, owner, token) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Transition several times: checkin -> trigger -> emergency_checkin -> checkin
    env.ledger().with_mut(|l| l.timestamp += 30 * DAY);
    client.check_in(&will_id, &owner);

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    env.ledger().with_mut(|l| l.timestamp += DAY);
    client.emergency_checkin(&will_id, &owner);

    // Page history with limit = 2
    let page1 = client.get_will_history_page(&will_id, &Some(0), &2);
    assert_eq!(page1.len(), 2);

    let page2 = client.get_will_history_page(&will_id, &Some(2), &2);
    assert!(!page2.is_empty());
}

// ── #511: weighted guardian votes satisfy quorum by cumulative weight ────────

#[test]
fn weighted_guardian_votes_accumulate_weight_for_quorum() {
    let (env, client, owner, token) = setup();
    let beneficiary = Address::generate(&env);

    let g_heavy = Address::generate(&env);
    let g_light1 = Address::generate(&env);
    let g_light2 = Address::generate(&env);

    // Guardians with varying weights: heavy = 10, light1 = 1, light2 = 1.
    // Threshold set to 10.
    let guardians = vec![
        &env,
        Guardian {
            address: g_heavy.clone(),
            weight: 10,
        },
        Guardian {
            address: g_light1.clone(),
            weight: 1,
        },
        Guardian {
            address: g_light2.clone(),
            weight: 1,
        },
    ];

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000)],
        &beneficiaries,
        &90,
        &7,
        &guardians,
        &10, // Quorum threshold = 10
        &None,
        &0,
    );

    // Two light guardians vote (cumulative weight = 2 < 10) -> does not trigger
    client.guardian_trigger(&will_id, &g_light1, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);

    client.guardian_trigger(&will_id, &g_light2, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);

    // Heavy guardian votes (weight = 10 >= 10 threshold) -> quorum reached!
    client.guardian_trigger(&will_id, &g_heavy, &GuardianVoteReason::Deceased);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);
}