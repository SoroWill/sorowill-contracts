#![cfg(test)]

//! Regression coverage for issue #220: `get_will_history` records the full
//! lifecycle, including the transition status, actor, and action labels.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

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

    (env.clone(), client, owner, token_address)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

#[test]
fn get_will_history_records_lifecycle_transition_sequence() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 1, "creation should append one audit entry");
    let create = history.get(0).unwrap();
    assert_eq!(create.from_status, WillStatus::Active);
    assert_eq!(create.to_status, WillStatus::Active);
    assert_eq!(create.actor, owner);
    assert_eq!(create.action, symbol_short!("create"));

    advance(&env, 91);
    client.trigger_will(&will_id);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 2, "trigger should append one more transition");
    let trigger = history.get(1).unwrap();
    assert_eq!(trigger.from_status, WillStatus::Active);
    assert_eq!(trigger.to_status, WillStatus::Triggered);
    assert_eq!(trigger.actor, env.current_contract_address());
    assert_eq!(trigger.action, symbol_short!("trigger"));

    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 3, "release should append the final transition");
    let release = history.get(2).unwrap();
    assert_eq!(release.from_status, WillStatus::Triggered);
    assert_eq!(release.to_status, WillStatus::Released);
    assert_eq!(release.actor, env.current_contract_address());
    assert_eq!(release.action, symbol_short!("release"));
}
