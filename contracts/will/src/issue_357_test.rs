#![cfg(test)]

//! #357: `update_periods` must publish the deadline `trigger_will` actually
//! enforces (`last_checkin + checkin_period_days`), not `now + checkin_period_days`.
//!
//! `update_periods` never touches `last_checkin`, so the two differ by however long
//! it has been since the owner's last check-in. An event carrying the `now`-based
//! value tells off-chain indexers to schedule a reminder arbitrarily later than the
//! real one.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, IntoVal,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

/// Creates an `Active` will with a 90-day check-in period and 7-day grace period
/// at a fixed timestamp. Returns `(env, contract, owner, will_id, created_at)`.
fn setup() -> (Env, Address, Address, u64, u64) {
    let env = Env::default();
    env.mock_all_auths();
    let created_at = 1_700_000_000;
    env.ledger().set_timestamp(created_at);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token, 1_000_000_i128)],
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
        &0,
        &None,
        &0,
    );

    (env, contract_id, owner, will_id, created_at)
}

/// Extracts the `next_deadline` field (4th element) of the `periodu` event that
/// `update_periods` just emitted.
fn emitted_next_deadline(env: &Env) -> u64 {
    let events = env.events().all();
    let last = events.last().expect("update_periods must emit an event");
    assert_eq!(last.1, (symbol_short!("periodu"), 1_u64).into_val(env));
    // Payload order matches `events::periods_updated`:
    // (owner, checkin_period_days, grace_period_days, next_deadline).
    let (_, _, _, next_deadline): (Address, u64, u64, u64) = last.2.clone().into_val(env);
    next_deadline
}

#[test]
fn periods_updated_event_reports_deadline_from_last_checkin() {
    let (env, contract_id, owner, will_id, created_at) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // 40 days pass, then the owner checks in, then another 40 days pass without a
    // check-in. `last_checkin` is now 40 days stale.
    env.ledger().with_mut(|l| l.timestamp += 40 * DAY);
    client.check_in(&will_id, &owner);
    let last_checkin = created_at + 40 * DAY;
    env.ledger().with_mut(|l| l.timestamp += 40 * DAY);

    // Shorten the period to 30 days. The enforced deadline is `last_checkin + 30d`
    // which is 10 days *before* now, so the will is already triggerable.
    client.update_periods(&will_id, &owner, &Some(30), &Some(7));

    let expected = last_checkin + 30 * DAY;
    assert_eq!(
        emitted_next_deadline(&env),
        expected,
        "the event must carry the last_checkin-based deadline, not now + period"
    );

    // The event value equals what `trigger_will` enforces, and it is strictly less
    // than the old `now`-based value that used to be published.
    assert!(expected < env.ledger().timestamp());
    client.trigger_will(&will_id);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);
}

#[test]
fn update_will_settings_period_change_keeps_deadline_anchored_to_last_checkin() {
    let (env, contract_id, owner, will_id, created_at) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|l| l.timestamp += 5 * DAY);

    // The composite entry point changes only the period; `last_checkin` must not
    // move, so the enforced deadline is still anchored to the original check-in.
    client.update_will_settings(&will_id, &owner, &None, &None, &Some(45), &Some(7));

    let expected = created_at + 45 * DAY;
    assert_eq!(
        client.get_time_until_deadline(&will_id),
        Some(expected as i64 - env.ledger().timestamp() as i64)
    );
    assert_eq!(client.get_will(&will_id).last_checkin, created_at);
}

/// A caller that relies purely on the event can still trigger on schedule: at the
/// emitted timestamp `trigger_will` succeeds, and one second earlier it does not.
#[test]
fn emitted_deadline_matches_the_instant_trigger_will_becomes_available() {
    let (env, contract_id, owner, will_id, created_at) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|l| l.timestamp += 3 * DAY);
    client.update_periods(&will_id, &owner, &Some(10), &Some(7));
    let deadline = emitted_next_deadline(&env);
    assert_eq!(deadline, created_at + 10 * DAY);

    env.ledger().set_timestamp(deadline - 1);
    let err = client.try_trigger_will(&will_id).unwrap_err().unwrap();
    assert_eq!(err, WillError::CheckinNotDue.into());

    env.ledger().set_timestamp(deadline);
    client.trigger_will(&will_id);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);
}
