#![cfg(test)]

//! Regression test for issue #278: `confirm_will` sets `will.last_checkin =
//! now` (the confirmation time), which should mean the check-in deadline is
//! computed from confirmation, not creation — but nothing asserted the
//! resulting deadline is anchored to the confirmation timestamp rather than
//! the original creation timestamp.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

#[test]
fn confirm_will_anchors_the_checkin_deadline_to_confirmation_time_not_creation_time() {
    let env = Env::default();
    env.mock_all_auths();
    let creation_time = 1_700_000_000;
    env.ledger().set_timestamp(creation_time);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let checkin_period_days = 90;
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
        &checkin_period_days,
        &7,
        &vec![&env],
        &1,
        &None,
        &(60 * DAY),
    );

    // Wait partway through the confirmation window, then confirm.
    env.ledger().with_mut(|l| l.timestamp += 40 * DAY);
    let confirmation_time = env.ledger().timestamp();
    client.confirm_will(&will_id, &owner);

    // `last_checkin` must be the confirmation time, not the creation time.
    let will = client.get_will(&will_id);
    assert_eq!(will.last_checkin, confirmation_time);
    assert_ne!(will.last_checkin, creation_time);

    // The deadline is `last_checkin + checkin_period_days`. One day before it
    // (measured from confirmation) trigger_will must still be rejected as
    // premature; this would already have elapsed if the clock had wrongly
    // started at creation.
    env.ledger()
        .with_mut(|l| l.timestamp += checkin_period_days * DAY - 1);
    assert_eq!(
        client.try_trigger_will(&will_id),
        Err(Ok(WillError::CheckinNotDue.into()))
    );

    // One day later (i.e. exactly `checkin_period_days` after confirmation),
    // the deadline has passed and trigger_will succeeds.
    env.ledger().with_mut(|l| l.timestamp += DAY);
    client.trigger_will(&will_id);
}
