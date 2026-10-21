#![cfg(test)]

//! Regression tests for issues #568-#571:
//! Verify cooldown resets for new guardian list.
//!
//! When a guardian list is replaced or updated via `update_guardians`,
//! `update_guardians_weighted`, or `update_will_settings`, the timestamp
//! `guardian_list_updated_at` must reset to the current ledger timestamp,
//! triggering a fresh 7-day cooldown during which guardian votes cannot be cast.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, GuardianSpec, GuardianVoteReason, WillContract,
    WillContractClient, WillError,
};

const DAY: u64 = 86_400;

fn setup_will(env: &Env) -> (Address, WillContractClient, Address, [Address; 3], u64) {
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
    let g3 = Address::generate(env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![env, (token_address, 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![env, g1.clone(), g2.clone()],
        &2,
        &None,
        &0,
    );

    (owner, client, token_address, [g1, g2, g3], will_id)
}

#[test]
fn test_update_guardians_resets_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, [g1, g2, g3], will_id) = setup_will(&env);

    client.accept_guardian_role(&will_id, &g1);
    client.accept_guardian_role(&will_id, &g2);

    // Initial 7-day cooldown elapses
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    // Past initial check-in period (90 days)
    env.ledger().with_mut(|l| l.timestamp += 90 * DAY);

    // Update guardians with a new list (g2 and g3)
    let update_time = env.ledger().timestamp();
    client.update_guardians(&will_id, &owner, &vec![&env, g2.clone(), g3.clone()]);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_list_updated_at, update_time);

    client.accept_guardian_role(&will_id, &g3);

    // Immediate vote attempt by g2 or g3 must fail with GuardianCooldownActive
    let result = client.try_guardian_trigger(&will_id, &g2, &GuardianVoteReason::Unreachable);
    assert_eq!(result, Err(Ok(WillError::GuardianCooldownActive.into())));

    // Fast-forward 6 days (still within 7-day cooldown)
    env.ledger().with_mut(|l| l.timestamp += 6 * DAY);
    let result_still_cooling =
        client.try_guardian_trigger(&will_id, &g2, &GuardianVoteReason::Unreachable);
    assert_eq!(result_still_cooling, Err(Ok(WillError::GuardianCooldownActive.into())));

    // Fast-forward past 7 days from update
    env.ledger().with_mut(|l| l.timestamp += 2 * DAY);
    let result_allowed =
        client.try_guardian_trigger(&will_id, &g2, &GuardianVoteReason::Unreachable);
    assert!(result_allowed.is_ok());
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);
}

#[test]
fn test_update_guardians_weighted_resets_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, [g1, _g2, g3], will_id) = setup_will(&env);

    // Let initial cooldown pass
    env.ledger().with_mut(|l| l.timestamp += 100 * DAY);

    // Update guardians with weighted guardians
    let weighted_guardians: SorobanVec<GuardianSpec> = vec![
        &env,
        GuardianSpec {
            address: g1.clone(),
            weight: 1,
        },
        GuardianSpec {
            address: g3.clone(),
            weight: 2,
        },
    ];
    let update_time = env.ledger().timestamp();
    client.update_guardians_weighted(&will_id, &owner, &weighted_guardians, &Some(2));

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_list_updated_at, update_time);

    client.accept_guardian_role(&will_id, &g1);
    client.accept_guardian_role(&will_id, &g3);

    // Trigger must fail immediately due to cooldown reset
    let result = client.try_guardian_trigger(&will_id, &g3, &GuardianVoteReason::Deceased);
    assert_eq!(result, Err(Ok(WillError::GuardianCooldownActive.into())));

    // Advance 8 days past cooldown
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    let result_ok = client.try_guardian_trigger(&will_id, &g3, &GuardianVoteReason::Deceased);
    assert!(result_ok.is_ok());
}

#[test]
fn test_update_will_settings_guardians_resets_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (owner, client, _token, [_g1, g2, g3], will_id) = setup_will(&env);

    // Initial cooldown passes
    env.ledger().with_mut(|l| l.timestamp += 100 * DAY);

    // Update will settings with a new guardian list
    let update_time = env.ledger().timestamp();
    client.update_will_settings(
        &will_id,
        &owner,
        &None,
        &None,
        &Some(vec![&env, g2.clone(), g3.clone()]),
        &None,
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_list_updated_at, update_time);

    client.accept_guardian_role(&will_id, &g2);
    client.accept_guardian_role(&will_id, &g3);

    // Trigger must fail immediately due to cooldown reset
    let result = client.try_guardian_trigger(&will_id, &g2, &GuardianVoteReason::Unreachable);
    assert_eq!(result, Err(Ok(WillError::GuardianCooldownActive.into())));

    // Fast-forward past 7 days cooldown
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    let result_ok = client.try_guardian_trigger(&will_id, &g2, &GuardianVoteReason::Unreachable);
    assert!(result_ok.is_ok());
}
