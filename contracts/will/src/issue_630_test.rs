#![cfg(test)]

//! Regression tests for issue #630: when the status history reaches
//! `MAX_HISTORY_ENTRIES`, trimming drops the oldest entries and keeps the
//! newest, so the trail always ends with the most recent transition.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::storage::MAX_HISTORY_ENTRIES;
use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

#[test]
fn history_trim_keeps_the_newest_entries() {
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

    // Each check-in appends one entry, so this overfills the trail by ten.
    let mut last_timestamp = t0;
    for i in 1..=(MAX_HISTORY_ENTRIES as u64 + 10) {
        last_timestamp = t0 + i * DAY;
        env.ledger().set_timestamp(last_timestamp);
        client.check_in(&will_id, &owner);
    }

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
    assert_eq!(
        history.get_unchecked(history.len() - 1).timestamp,
        last_timestamp
    );
    // The creation entry at `t0` is the oldest and must have been dropped.
    assert!(history.get_unchecked(0).timestamp > t0);
}
