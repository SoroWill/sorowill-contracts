#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

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

    (env, client, owner, token_address)
}

fn create_active_will(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token_address: &Address,
    checkin_period_days: u64,
    grace_period_days: u64,
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

    

    // confirmation_delay_seconds is 0 above, so the will starts Active
    // immediately -- no confirm_will call is needed (or valid).
    client.create_will(
        owner,
        &tokens,
        &beneficiaries,
        &checkin_period_days,
        &grace_period_days,
        &vec![env],
        &1,
        &None,
        &0,
    )
}

#[test]
fn test_get_time_until_deadline_active_will_positive_time_remaining() {
    let (env, client, owner, token_address) = setup();

    let checkin_period_days = 30;
    let will_id = create_active_will(&env, &client, &owner, &token_address, checkin_period_days, 7);

    // Will was just created, so check-in deadline is ~30 days away
    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_some());
    let seconds_remaining = time_until_deadline.unwrap();

    // Should be approximately 30 days in seconds
    let expected_approx = (checkin_period_days * DAY) as i64;
    let diff = (seconds_remaining - expected_approx).abs();
    assert!(diff < 100, "Deadline should be ~30 days away, got {} seconds", seconds_remaining);
}

#[test]
fn test_get_time_until_deadline_active_will_negative_after_missed_deadline() {
    let (env, client, owner, token_address) = setup();

    let checkin_period_days = 30;
    let will_id = create_active_will(&env, &client, &owner, &token_address, checkin_period_days, 7);

    // Advance time past the check-in deadline but don't trigger yet
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);

    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_some());
    let seconds_remaining = time_until_deadline.unwrap();

    // Should be negative (deadline has passed)
    assert!(
        seconds_remaining < 0,
        "Deadline should be in the past (negative), got {} seconds",
        seconds_remaining
    );
}

#[test]
fn test_get_time_until_deadline_triggered_will_counts_grace_period() {
    let (env, client, owner, token_address) = setup();

    let checkin_period_days = 30;
    let grace_period_days = 7;
    let will_id = create_active_will(&env, &client, &owner, &token_address, checkin_period_days, grace_period_days);

    // Advance past check-in deadline
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);

    // Trigger the will
    client.trigger_will(&will_id);

    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_some());
    let seconds_remaining = time_until_deadline.unwrap();

    // Should be approximately 7 days in seconds
    let expected_approx = (grace_period_days * DAY) as i64;
    let diff = (seconds_remaining - expected_approx).abs();
    assert!(
        diff < 100,
        "Grace period deadline should be ~7 days away, got {} seconds",
        seconds_remaining
    );
}

#[test]
fn test_get_time_until_deadline_triggered_will_negative_after_grace_expires() {
    let (env, client, owner, token_address) = setup();

    let checkin_period_days = 30;
    let grace_period_days = 7;
    let will_id = create_active_will(&env, &client, &owner, &token_address, checkin_period_days, grace_period_days);

    // Advance past check-in deadline
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);

    // Trigger the will
    client.trigger_will(&will_id);

    // Advance past grace period
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_some());
    let seconds_remaining = time_until_deadline.unwrap();

    // Should be negative (grace period has passed)
    assert!(
        seconds_remaining < 0,
        "Grace period should have expired (negative), got {} seconds",
        seconds_remaining
    );
}

#[test]
fn test_get_time_until_deadline_pending_confirmation_returns_none() {
    let (env, client, owner, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &1,
        &None,
        &3600,
    );

    // Don't confirm the will - it's in PendingConfirmation status
    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_none(), "PendingConfirmation status should return None");
}

#[test]
fn test_get_time_until_deadline_released_returns_none() {
    let (env, client, owner, token_address) = setup();

    let checkin_period_days = 30;
    let grace_period_days = 7;
    let will_id = create_active_will(&env, &client, &owner, &token_address, checkin_period_days, grace_period_days);

    // Advance past check-in deadline
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);

    // Trigger and release
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_none(), "Released status should return None");
}

#[test]
fn test_get_time_until_deadline_cancelled_returns_none() {
    let (env, client, owner, token_address) = setup();

    let checkin_period_days = 30;
    let will_id = create_active_will(&env, &client, &owner, &token_address, checkin_period_days, 7);

    // Cancel the will
    client.cancel_will(&will_id, &owner);

    let time_until_deadline = client.get_time_until_deadline(&will_id);
    assert!(time_until_deadline.is_none(), "Cancelled status should return None");
}

#[test]
fn test_get_time_until_deadline_nonexistent_will_panics() {
    let (_env, client, _owner, _token_address) = setup();
    // Soroban's panic message only shows the numeric error code, never the
    // enum variant name, so should_panic(expected = "WillNotFound") can
    // never match -- use try_get_time_until_deadline instead.
    assert_eq!(
        client.try_get_time_until_deadline(&9999),
        Err(Ok(crate::WillError::WillNotFound.into())),
    );
}
