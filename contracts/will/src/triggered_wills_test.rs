#![cfg(test)]

//! Regression coverage for `get_triggered_wills` (#212): asserts a will's id
//! appears in the triggered-wills index after `trigger_will`, and is
//! correctly removed after each of `emergency_checkin`, `release_inheritance`,
//! and `guardian_cancel_trigger`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup(env: &Env) -> (WillContractClient<'_>, Address, Address) {
    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);
    (client, owner, token_address)
}

fn create_will(
    env: &Env,
    client: &WillContractClient<'_>,
    owner: &Address,
    token_address: &Address,
    beneficiary: &Address,
    guardians: soroban_sdk::Vec<Address>,
    guardian_threshold: u32,
) -> u64 {
    client.create_will(
        owner,
        &vec![env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &guardians,
        &guardian_threshold,
        &None,
        &0,
    )
}

#[test]
fn trigger_will_adds_id_to_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    assert!(client.get_triggered_wills().is_empty());

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    let triggered = client.get_triggered_wills();
    assert_eq!(triggered.len(), 1);
    assert_eq!(triggered.get(0).unwrap(), will_id);
}

#[test]
fn emergency_checkin_removes_id_from_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills().len(), 1);

    client.emergency_checkin(&will_id, &owner);
    assert!(client.get_triggered_wills().is_empty());
}

#[test]
fn release_inheritance_removes_id_from_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env],
        0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills().len(), 1);

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);
    assert!(client.get_triggered_wills().is_empty());
}

#[test]
fn guardian_cancel_trigger_removes_id_from_triggered_index() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let (client, owner, token_address) = setup(&env);
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &beneficiary,
        vec![&env, guardian.clone()],
        1,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_triggered_wills().len(), 1);

    client.accept_guardian_role(&will_id, &guardian);
    client.guardian_cancel_trigger(&will_id, &guardian);
    assert!(client.get_triggered_wills().is_empty());
}
