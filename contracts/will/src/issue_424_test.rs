#![cfg(test)]

//! Regression tests for issue #424: `get_time_until_deadline` must return a
//! negative `i64` (not a huge wrapped `u64`) when a will's next deadline is
//! already in the past.
//!
//! The implementation returns `Option<i64>`, so an overdue deadline is
//! expressed as a negative number of seconds rather than wrapping to a large
//! positive value.  Callers should treat any non-positive value as
//! "actionable now".

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
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

/// An `Active` will whose check-in deadline has passed but which has not yet
/// been triggered must return a negative number of seconds, not a large
/// positive wrapped value.
#[test]
fn overdue_active_will_returns_negative_seconds() {
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
        &30, // 30-day check-in period
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Advance 31 days — the check-in deadline is now one day in the past.
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);

    let result = client.get_time_until_deadline(&will_id);
    assert!(result.is_some(), "Active will must always return Some");

    let seconds = result.unwrap();
    assert!(
        seconds < 0,
        "An overdue will must return a negative value (got {}), not a large positive one",
        seconds
    );

    // The magnitude should be approximately one day (±a few seconds for any
    // ledger-time rounding).
    let expected_overdue_secs: i64 = -(DAY as i64);
    let diff = (seconds - expected_overdue_secs).abs();
    assert!(
        diff < 100,
        "Overdue by ~1 day, expected ≈{} seconds, got {}",
        expected_overdue_secs,
        seconds
    );
}

/// A `Triggered` will whose grace period has expired must also return a
/// negative number of seconds — it is past-due for release, not future-due.
#[test]
fn overdue_triggered_will_returns_negative_seconds() {
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
        &30, // 30-day check-in period
        &7,  // 7-day grace period
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Advance past the check-in deadline and trigger.
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);
    client.trigger_will(&will_id);

    // Advance past the grace period too — the will is overdue for release.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    let result = client.get_time_until_deadline(&will_id);
    assert!(result.is_some(), "Triggered will must always return Some");

    let seconds = result.unwrap();
    assert!(
        seconds < 0,
        "An overdue triggered will must return a negative value (got {})",
        seconds
    );
}
