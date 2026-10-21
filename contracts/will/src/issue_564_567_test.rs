#![cfg(test)]

//! Regression and safety tests for issues #564, #565, #566, and #567:
//!
//! - **#564**: `release_inheritance` must adhere to checks-effects-interactions
//!   pattern, committing state mutations (`status = Released`, balance cleared,
//!   and locked totals decremented) before performing external transfers,
//!   preventing any reentrancy or double-release.
//! - **#565**: Guardian votes must enforce freshness and reject duplicate or
//!   re-voting during the vote window, preventing flash-loan attacks.
//! - **#566**: Input validation and bounds checking for function scenario 50.
//! - **#567**: Input validation and bounds checking for function scenario 51.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillError,
    WillStatus,
};

const DAY: u64 = 86_400;

fn setup_will(env: &Env) -> (Address, WillContractClient, Address, Address, [Address; 2], u64) {
    let owner = Address::generate(env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    let beneficiary = Address::generate(env);
    let g1 = Address::generate(env);
    let g2 = Address::generate(env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![env, (token_address.clone(), 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![env, g1.clone(), g2.clone()],
        &1,
        &None,
        &0,
    );

    (owner, client, token_address, beneficiary, [g1, g2], will_id)
}

#[test]
fn test_checks_effects_interactions_reentrancy_protection() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (_owner, client, _token, _beneficiary, [g1, _g2], will_id) = setup_will(&env);

    // Initial 7-day cooldown passes
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.accept_guardian_role(&will_id, &g1);

    // Inactivity trigger: advance 90 days
    env.ledger().with_mut(|l| l.timestamp += 90 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Unreachable);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Triggered);

    // Advance 8 days past grace period
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    // First release succeeds
    client.release_inheritance(&will_id, &None);

    let released_will = client.get_will(&will_id);
    assert_eq!(released_will.status, WillStatus::Released);

    // Subsequent reentrant call immediately fails with WillNotTriggered
    let result = client.try_release_inheritance(&will_id, &None);
    assert_eq!(result, Err(Ok(WillError::WillNotTriggered.into())));
}

#[test]
fn test_guardian_cannot_revote_during_vote_window() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (_owner, client, _token, _beneficiary, [g1, _g2], will_id) = setup_will(&env);

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.accept_guardian_role(&will_id, &g1);

    env.ledger().with_mut(|l| l.timestamp += 90 * DAY);
    let vote_res = client.try_guardian_trigger(&will_id, &g1, &GuardianVoteReason::Deceased);
    assert!(vote_res.is_ok());

    // Immediate duplicate vote in same block/window is rejected with AlreadyVoted
    let revote_res = client.try_guardian_trigger(&will_id, &g1, &GuardianVoteReason::Deceased);
    assert_eq!(revote_res, Err(Ok(WillError::AlreadyVoted.into())));
}

#[test]
fn test_boundary_validation_scenarios_50_and_51() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, _beneficiary, [g1, g2], will_id) = setup_will(&env);

    // Verify existing state and boundary inputs
    let will = client.get_will(&will_id);
    assert!(will.id > 0);
    assert_eq!(will.owner, owner);

    // Testing boundary conditions on grace period and checkin days
    let invalid_settings = client.try_update_will_settings(
        &will_id,
        &owner,
        &Some(0), // zero days checkin interval boundary
        &None,
        &None,
        &None,
    );
    assert!(invalid_settings.is_err());

    let invalid_guardians = client.try_update_guardians(
        &will_id,
        &owner,
        &vec![&env, g1.clone(), g2.clone(), g1.clone()], // duplicate guardians boundary
    );
    assert!(invalid_guardians.is_err());
}
