#![cfg(test)]

//! Tests for the `update_guardians` + `update_will_settings` threshold-safety
//! check introduced to fix the "unreachable threshold" bug.
//!
//! Scenario: a will is created with three guardians and a threshold of 2.
//! The owner later tries to shrink the guardian list to a single address,
//! which would leave `guardian_threshold = 2` while only 1 guardian exists —
//! making the incapacitation safety mechanism permanently unusable.
//! Both `update_guardians` and `update_will_settings` must reject this.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

/// Creates a will with three guardians and `guardian_threshold = 2`.
/// Returns `(env, contract_address, owner, [g1, g2, g3], will_id)`.
fn setup_three_guardians_threshold_two() -> (Env, Address, Address, [Address; 3], u64) {
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
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);

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
        &vec![&env, g1.clone(), g2.clone(), g3.clone()],
        &2, // threshold = 2, list len = 3  →  valid at create time
        &None,
        &0,
    );

    (env, contract_id, owner, [g1, g2, g3], will_id)
}

// ---------------------------------------------------------------------------
// update_guardians
// ---------------------------------------------------------------------------

/// Shrinking 3 → 1 while `guardian_threshold = 2` must be rejected.
#[test]
fn update_guardians_rejects_new_list_shorter_than_threshold() {
    let (env, contract_id, owner, _gs, will_id) = setup_three_guardians_threshold_two();
    let client = WillContractClient::new(&env, &contract_id);

    let replacement = Address::generate(&env);
    let result = client.try_update_guardians(
        &will_id,
        &owner,
        &vec![&env, replacement], // 1 guardian, threshold still 2 → unreachable
    );

    assert_eq!(
        result,
        Err(Ok(WillError::InvalidGuardianThreshold.into())),
        "shrinking the guardian list below guardian_threshold must return InvalidGuardianThreshold"
    );
}

/// Shrinking 3 → 2 is fine: threshold = 2 is still exactly reachable.
#[test]
fn update_guardians_allows_list_equal_to_threshold() {
    let (env, contract_id, owner, [g1, g2, _g3], will_id) = setup_three_guardians_threshold_two();
    let client = WillContractClient::new(&env, &contract_id);

    // Should succeed — 2 guardians, threshold = 2.
    client.update_guardians(&will_id, &owner, &vec![&env, g1, g2]);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardians.len(), 2);
    assert_eq!(will.guardian_threshold, 2);
}

/// Clearing the guardian list entirely (0 guardians) is always valid: it
/// disables the guardian mechanism rather than leaving an unreachable threshold.
#[test]
fn update_guardians_allows_clearing_list_entirely() {
    let (env, contract_id, owner, _gs, will_id) = setup_three_guardians_threshold_two();
    let client = WillContractClient::new(&env, &contract_id);

    // Empty list — the guardian mechanism is simply disabled.
    client.update_guardians(&will_id, &owner, &vec![&env]);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardians.len(), 0);
}

// ---------------------------------------------------------------------------
// update_will_settings (guardian branch)
// ---------------------------------------------------------------------------

/// Same shrink-below-threshold scenario through the composite entry point.
#[test]
fn update_will_settings_rejects_new_guardian_list_shorter_than_threshold() {
    let (env, contract_id, owner, _gs, will_id) = setup_three_guardians_threshold_two();
    let client = WillContractClient::new(&env, &contract_id);

    let replacement = Address::generate(&env);
    let result = client.try_update_will_settings(
        &will_id,
        &owner,
        &None,
        &Some(vec![&env, replacement]), // 1 guardian, threshold still 2
        &None,
        &None,
    );

    assert_eq!(
        result,
        Err(Ok(WillError::InvalidGuardianThreshold.into())),
        "update_will_settings must also enforce the threshold-vs-list-length invariant"
    );
}

/// The cooldown guard must still fire even when the threshold check passes,
/// confirming the two checks are independent and both in effect.
#[test]
fn update_guardians_valid_shrink_then_cooldown_blocks_trigger() {
    let (env, contract_id, owner, [g1, g2, _g3], will_id) = setup_three_guardians_threshold_two();
    let client = WillContractClient::new(&env, &contract_id);

    // Shrink from 3 to 2 (threshold = 2 remains valid).
    client.update_guardians(&will_id, &owner, &vec![&env, g1.clone(), g2.clone()]);

    // Immediately try to trigger — the 7-day guardian cooldown must block it.
    let result = client.try_guardian_trigger(
        &will_id,
        &g1,
        &crate::GuardianVoteReason::Unreachable,
    );
    assert_eq!(
        result,
        Err(Ok(WillError::GuardianCooldownActive.into())),
        "the cooldown must still be enforced after a valid guardian-list shrink"
    );

    // After the cooldown elapses the trigger is accepted.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    // Guardian must accept their role before voting.
    client.accept_guardian_role(&will_id, &g1);
    client.guardian_trigger(&will_id, &g1, &crate::GuardianVoteReason::Unreachable);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);
}
