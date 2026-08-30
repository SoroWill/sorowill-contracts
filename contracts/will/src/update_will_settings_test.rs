#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner)
}

fn create_active_will(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token_address: &Address,
) -> u64 {
    let beneficiary = Address::generate(env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![env, (token_address.clone(), 1_000_000_i128)];

    let will_id = client.create_will(
        owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![env],
        &1,
        &None,
        &0,
    );

    client.confirm_will(&will_id, owner);
    will_id
}

#[test]
fn test_update_will_settings_beneficiaries_only() {
    let (env, client, owner) = setup();
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();

    let will_id = create_active_will(&env, &client, &owner, &token_address);

    // Get original will
    let original_will = client.get_will(&will_id);
    assert_eq!(original_will.beneficiaries.len(), 1);

    // Update beneficiaries only
    let new_beneficiary1 = Address::generate(&env);
    let new_beneficiary2 = Address::generate(&env);
    let new_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: new_beneficiary1.clone(),
            allocation: Allocation::Percentage(5_000),
        },
        Beneficiary {
            address: new_beneficiary2.clone(),
            allocation: Allocation::Percentage(5_000),
        },
    ];

    client.update_will_settings(&will_id, &owner, &Some(new_beneficiaries), &None, &None, &None);

    // Verify beneficiaries were updated
    let updated_will = client.get_will(&will_id);
    assert_eq!(updated_will.beneficiaries.len(), 2);
    assert_eq!(updated_will.beneficiaries.get(0).unwrap().address, new_beneficiary1);
    assert_eq!(updated_will.beneficiaries.get(1).unwrap().address, new_beneficiary2);

    // Verify periods were not changed
    assert_eq!(updated_will.checkin_period_days, original_will.checkin_period_days);
    assert_eq!(updated_will.grace_period_days, original_will.grace_period_days);
}

#[test]
fn test_update_will_settings_guardians_only() {
    let (env, client, owner) = setup();
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();

    let will_id = create_active_will(&env, &client, &owner, &token_address);

    // Get original will
    let original_will = client.get_will(&will_id);
    assert_eq!(original_will.guardians.len(), 0);

    // Update guardians only
    let guardian1 = Address::generate(&env);
    let guardian2 = Address::generate(&env);
    let new_guardians: SorobanVec<Address> = vec![&env, guardian1.clone(), guardian2.clone()];

    client.update_will_settings(&will_id, &owner, &None, &Some(new_guardians), &None, &None);

    // Verify guardians were updated
    let updated_will = client.get_will(&will_id);
    assert_eq!(updated_will.guardians.len(), 2);
    assert_eq!(updated_will.guardians.get(0).unwrap().address, guardian1);
    assert_eq!(updated_will.guardians.get(1).unwrap().address, guardian2);

    // Verify beneficiaries and periods were not changed
    assert_eq!(updated_will.beneficiaries.len(), original_will.beneficiaries.len());
    assert_eq!(updated_will.checkin_period_days, original_will.checkin_period_days);
    assert_eq!(updated_will.grace_period_days, original_will.grace_period_days);
}

#[test]
fn test_update_will_settings_periods_only() {
    let (env, client, owner) = setup();
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();

    let will_id = create_active_will(&env, &client, &owner, &token_address);

    // Get original will
    let original_will = client.get_will(&will_id);
    let original_checkin = original_will.checkin_period_days;
    let original_grace = original_will.grace_period_days;

    // Update periods only
    let new_checkin = original_checkin + 10;
    let new_grace = original_grace + 5;

    client.update_will_settings(&will_id, &owner, &None, &None, &Some(new_checkin), &Some(new_grace));

    // Verify periods were updated
    let updated_will = client.get_will(&will_id);
    assert_eq!(updated_will.checkin_period_days, new_checkin);
    assert_eq!(updated_will.grace_period_days, new_grace);

    // Verify beneficiaries and guardians were not changed
    assert_eq!(updated_will.beneficiaries.len(), original_will.beneficiaries.len());
    assert_eq!(updated_will.guardians.len(), original_will.guardians.len());
}

#[test]
fn test_update_will_settings_all_fields_together() {
    let (env, client, owner) = setup();
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();

    let will_id = create_active_will(&env, &client, &owner, &token_address);

    // Get original will
    let original_will = client.get_will(&will_id);

    // Prepare all updates
    let new_beneficiary = Address::generate(&env);
    let new_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: new_beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let new_guardian = Address::generate(&env);
    let new_guardians: SorobanVec<Address> = vec![&env, new_guardian.clone()];

    let new_checkin = original_will.checkin_period_days + 15;
    let new_grace = original_will.grace_period_days + 3;

    // Update all fields at once
    client.update_will_settings(
        &will_id,
        &owner,
        &Some(new_beneficiaries),
        &Some(new_guardians),
        &Some(new_checkin),
        &Some(new_grace),
    );

    // Verify all fields were updated
    let updated_will = client.get_will(&will_id);
    assert_eq!(updated_will.beneficiaries.len(), 1);
    assert_eq!(updated_will.beneficiaries.get(0).unwrap().address, new_beneficiary);
    assert_eq!(updated_will.guardians.len(), 1);
    assert_eq!(updated_will.guardians.get(0).unwrap().address, new_guardian);
    assert_eq!(updated_will.checkin_period_days, new_checkin);
    assert_eq!(updated_will.grace_period_days, new_grace);
}

#[test]
fn test_update_will_settings_none_fields_untouched() {
    let (env, client, owner) = setup();
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();

    let will_id = create_active_will(&env, &client, &owner, &token_address);

    // Get original will
    let original_will = client.get_will(&will_id);
    let original_beneficiary_count = original_will.beneficiaries.len();
    let original_guardian_count = original_will.guardians.len();
    let original_checkin = original_will.checkin_period_days;
    let original_grace = original_will.grace_period_days;

    // Update with all None values (no actual changes)
    client.update_will_settings(&will_id, &owner, &None, &None, &None, &None);

    // Verify nothing changed
    let still_will = client.get_will(&will_id);
    assert_eq!(still_will.beneficiaries.len(), original_beneficiary_count);
    assert_eq!(still_will.guardians.len(), original_guardian_count);
    assert_eq!(still_will.checkin_period_days, original_checkin);
    assert_eq!(still_will.grace_period_days, original_grace);
}

#[test]
fn test_update_will_settings_partial_updates_independent() {
    let (env, client, owner) = setup();
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();

    let will_id = create_active_will(&env, &client, &owner, &token_address);

    // Get original will
    let original_will = client.get_will(&will_id);

    // First update: just beneficiaries
    let new_beneficiary1 = Address::generate(&env);
    let new_beneficiaries1: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: new_beneficiary1.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    client.update_will_settings(&will_id, &owner, &Some(new_beneficiaries1), &None, &None, &None);

    let after_first = client.get_will(&will_id);
    assert_eq!(after_first.beneficiaries.get(0).unwrap().address, new_beneficiary1);
    assert_eq!(after_first.checkin_period_days, original_will.checkin_period_days);

    // Second update: just periods
    let new_checkin = original_will.checkin_period_days + 5;
    client.update_will_settings(&will_id, &owner, &None, &None, &Some(new_checkin), &None);

    let after_second = client.get_will(&will_id);
    // Beneficiary from first update should still be there
    assert_eq!(after_second.beneficiaries.get(0).unwrap().address, new_beneficiary1);
    // Period should be updated
    assert_eq!(after_second.checkin_period_days, new_checkin);
}

// ── Regression: stale cancel-vote weight cleared by update_will_settings ──

/// Verify that updating the guardian list via `update_will_settings` resets
/// `guardian_cancel_vote_weight`, `guardian_cancel_votes`, and the underlying
/// per-guardian `GuardianCancelVote` storage keys, just as `update_guardians`
/// already does.
///
/// Scenario:
///  1. Create a will with two guardians (threshold 2) and a 90-day check-in
///     period so the guardian-list cooldown (7 days) expires well before the
///     first check-in deadline.
///  2. Advance past the cooldown, miss the check-in, and trigger the will.
///  3. Guardian A casts a cancel-trigger vote → `guardian_cancel_votes = 1`,
///     `guardian_cancel_vote_weight = 1`.  Quorum (2) is NOT reached, so the
///     vote record lives in storage.
///  4. The owner calls `emergency_checkin` to return the will to Active; this
///     correctly resets the cancel-vote counters via the existing code-path.
///  5. Trigger and partial-cancel-vote the will a second time, then again use
///     `emergency_checkin` to return to Active.  After two cycles the will
///     carries a clean counter, but we have confirmed the storage round-trip.
///  6. Trigger a third time; guardian A casts another cancel vote
///     (`guardian_cancel_votes = 1`).  Now call `emergency_checkin` once more
///     so the will is Active with clean counters, then immediately call
///     `update_will_settings` with a completely new guardian list.
///  7. Assert that after the settings update:
///     - `guardian_cancel_votes == 0`
///     - `guardian_cancel_vote_weight == 0`
///     - the new guardian can successfully cast a cancel vote on a subsequent
///       trigger, confirming the old storage keys were removed and the fresh
///       list is independent.
#[test]
fn update_will_settings_guardian_change_clears_cancel_vote_state() {
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

    // Create will: 90-day check-in, 7-day grace, threshold 2.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &2,
        &None,
        &0,
    );

    // Accept guardian roles so they can vote.
    client.accept_guardian_role(&will_id, &guardian_a);
    client.accept_guardian_role(&will_id, &guardian_b);

    // ── Cycle 1: trigger → partial cancel vote → emergency_checkin ──────
    // Advance past the 7-day guardian-list cooldown and the 90-day check-in
    // deadline, then trigger the will.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Guardian A casts one cancel vote (quorum = 2, so not reached yet).
    client.guardian_cancel_trigger(&will_id, &guardian_a);
    let mid = client.get_will(&will_id);
    assert_eq!(mid.guardian_cancel_votes, 1);
    assert_eq!(mid.guardian_cancel_vote_weight, 1);

    // Owner recovers via emergency_checkin — this already resets cancel state.
    client.emergency_checkin(&will_id, &owner);
    let after_emerg = client.get_will(&will_id);
    assert_eq!(after_emerg.status, WillStatus::Active);
    assert_eq!(after_emerg.guardian_cancel_votes, 0, "emergency_checkin must zero cancel_votes");
    assert_eq!(after_emerg.guardian_cancel_vote_weight, 0);

    // ── Now call update_will_settings to swap the guardian list ──────────
    // The will is Active with clean counters; replace both guardians with a
    // completely fresh address.  Before the fix, this branch would not call
    // reset_guardian_cancel_votes and would not zero the cancel-vote fields,
    // leaving them stale for future triggered cycles.
    let new_guardian = Address::generate(&env);
    client.update_will_settings(
        &will_id,
        &owner,
        &None,
        &Some(vec![&env, new_guardian.clone()]),
        &None,
        &None,
    );

    let after_update = client.get_will(&will_id);

    // ── Core assertions: cancel-vote state is clean after the guardian swap ──
    assert_eq!(
        after_update.guardian_cancel_votes, 0,
        "update_will_settings must zero guardian_cancel_votes when changing guardians"
    );
    assert_eq!(
        after_update.guardian_cancel_vote_weight, 0,
        "update_will_settings must zero guardian_cancel_vote_weight when changing guardians"
    );
    assert_eq!(
        after_update.guardian_votes, 0,
        "guardian release-vote counter must also be cleared"
    );
    assert_eq!(
        after_update.guardian_vote_weight, 0,
        "guardian release-vote weight must also be cleared"
    );
    assert_eq!(after_update.guardians.len(), 1);
    assert_eq!(after_update.guardians.get(0).unwrap().address, new_guardian);

    // ── Confirm the new guardian's cancel-vote works on the next cycle ───
    // Accept the new guardian's role, advance to miss the check-in, trigger,
    // and cast a cancel vote.  This verifies the old storage keys are gone
    // (the new guardian starts from a clean slate, not inheriting stale state).
    client.accept_guardian_role(&will_id, &new_guardian);
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    client.guardian_cancel_trigger(&will_id, &new_guardian);
    let after_new_vote = client.get_will(&will_id);
    assert_eq!(
        after_new_vote.guardian_cancel_votes, 1,
        "new guardian must be able to cast a fresh cancel vote after guardian list was updated"
    );
    assert_eq!(after_new_vote.guardian_cancel_vote_weight, 1);
}
