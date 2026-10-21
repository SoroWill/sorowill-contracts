#![cfg(test)]

//! Regression tests for issue #628: a will that has been released cannot be
//! checked in again through `emergency_checkin`. Only a `Triggered` will
//! accepts an emergency check-in, and a released will is past that point.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

fn triggered_will() -> (Env, Address, Address, u64, u64) {
    let env = Env::default();
    env.mock_all_auths();
    let t0: u64 = 1_700_000_000;
    env.ledger().set_timestamp(t0);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::FixedAmount(500_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    env.ledger().set_timestamp(t0 + 91 * DAY);
    client.trigger_will(&will_id);
    let grace_deadline = (t0 + 91 * DAY) + 7 * DAY;
    (env, contract_id, owner, will_id, grace_deadline)
}

#[test]
fn emergency_checkin_is_refused_after_release() {
    let (env, contract_id, owner, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(grace_deadline + 1);
    client.release_inheritance(&will_id, &None);

    let result = client.try_emergency_checkin(&will_id, &owner);
    assert_eq!(result, Err(Ok(WillError::WillNotTriggered)));
}
