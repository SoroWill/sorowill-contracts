#![cfg(test)]

//! Regression tests for issue #715: grace-period deadline edge cases.
//!
//! Covers the acceptance criteria: a grace period of 0 is rejected, the
//! maximum grace period produces the correct deadline and release boundary,
//! and repeated grace-period changes (extensions) before a trigger leave the
//! deadline computed from the final value only.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;
const MAX_PERIOD_DAYS: u64 = 3_650;

fn setup(env: &Env) -> (WillContractClient, Address, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000i128);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (client, owner, token_address)
}

fn one_beneficiary(env: &Env) -> SorobanVec<Beneficiary> {
    let beneficiary = Address::generate(env);
    vec![
        env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ]
}

fn create(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
    checkin_days: u64,
    grace_days: u64,
) -> u64 {
    let tokens = vec![env, (token.clone(), 1_000i128)];
    client.create_will(
        owner,
        &tokens,
        &one_beneficiary(env),
        &checkin_days,
        &grace_days,
        &vec![env],
        &0,
        &None,
        &0,
    )
}

/// A grace period of zero days must be rejected at creation (#715).
#[test]
fn grace_period_zero_is_rejected() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let tokens = vec![&env, (token.clone(), 1_000i128)];
    let result = client.try_create_will(
        &owner,
        &tokens,
        &one_beneficiary(&env),
        &90,
        &0,
        &vec![&env],
        &0,
        &None,
        &0,
    );
    assert!(result.is_err(), "grace_period_days = 0 must be rejected");
}

/// At the maximum grace period the deadline is exactly `trigger + MAX * DAY`:
/// release is refused on that second and allowed one second later (#715).
#[test]
fn max_grace_period_release_boundary_is_exact() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let start = env.ledger().timestamp();
    let will_id = create(&env, &client, &owner, &token, 90, MAX_PERIOD_DAYS);

    // Trigger exactly at the missed check-in deadline.
    env.ledger()
        .with_mut(|l| l.timestamp = start + 90 * DAY);
    client.trigger_will(&will_id);

    let trigger_time = env.ledger().timestamp();
    assert_eq!(
        client.get_time_until_deadline(&will_id),
        Some((MAX_PERIOD_DAYS * DAY) as i64),
        "full max grace window must remain right after trigger"
    );

    // Deadline second itself is not releasable.
    env.ledger()
        .with_mut(|l| l.timestamp = trigger_time + MAX_PERIOD_DAYS * DAY);
    assert!(
        client.try_release_inheritance(&will_id, &None).is_err(),
        "release must be refused at exactly the grace deadline"
    );

    // One second later the grace period has elapsed.
    env.ledger()
        .with_mut(|l| l.timestamp = trigger_time + MAX_PERIOD_DAYS * DAY + 1);
    client.release_inheritance(&will_id, &None);
}

/// Multiple grace-period extensions made while Active must leave only the
/// final value in force: the deadline after trigger uses the last extension,
/// not any earlier one (#715).
#[test]
fn multiple_grace_extensions_use_final_value() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let start = env.ledger().timestamp();
    let will_id = create(&env, &client, &owner, &token, 90, 7);

    client.update_periods(&will_id, &owner, &None, &Some(30));
    client.update_periods(&will_id, &owner, &None, &Some(365));

    env.ledger()
        .with_mut(|l| l.timestamp = start + 90 * DAY);
    client.trigger_will(&will_id);

    assert_eq!(
        client.get_time_until_deadline(&will_id),
        Some((365 * DAY) as i64),
        "deadline must reflect the final grace extension"
    );
}
