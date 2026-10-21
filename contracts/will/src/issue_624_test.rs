#![cfg(test)]

//! Regression tests for issue #624: two check-ins in the same ledger second store
//! the same timestamp, so the deadline does not depend on which one ran first.
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

#[test]
fn two_check_ins_in_one_second_give_the_same_deadline() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary { address: Address::generate(&env), allocation: Allocation::FixedAmount(500_000) },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    env.ledger().set_timestamp(1_700_000_000 + 10 * DAY);
    client.check_in(&will_id, &owner);
    let first = client.get_time_until_deadline(&will_id);
    client.check_in(&will_id, &owner);
    let second = client.get_time_until_deadline(&will_id);
    assert_eq!(first, second);
}
