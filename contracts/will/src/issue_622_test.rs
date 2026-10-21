#![cfg(test)]

//! Regression tests for issue #622: the grace deadline second belongs to the
//! owner. `release_inheritance` must refuse it, and `emergency_checkin` must
//! accept it, so the two never both succeed or both fail at the same timestamp.

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
fn release_is_refused_at_the_deadline_second() {
    let (env, contract_id, _owner, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);
    env.ledger().set_timestamp(grace_deadline);
    let result = client.try_release_inheritance(&will_id, &None);
    assert_eq!(result, Err(Ok(WillError::GracePeriodNotExpired)));
}

#[test]
fn owner_check_in_is_accepted_at_the_deadline_second() {
    let (env, contract_id, owner, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);
    env.ledger().set_timestamp(grace_deadline);
    client.emergency_checkin(&will_id, &owner);
}

#[test]
fn release_succeeds_one_second_after_the_deadline() {
    let (env, contract_id, _owner, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);
    env.ledger().set_timestamp(grace_deadline + 1);
    client.release_inheritance(&will_id, &None);
}
