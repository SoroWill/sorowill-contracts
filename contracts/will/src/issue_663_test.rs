#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    status_transition, Allocation, Beneficiary, WillContract, WillContractClient, WillError,
    WillStatus,
};

const DAY: u64 = 86_400;
const LOCKED_AMOUNT: i128 = 1_000_000;

fn setup<'a>() -> (
    Env,
    Address,
    WillContractClient<'a>,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &LOCKED_AMOUNT);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    (env, contract_id, client, owner, beneficiary, token)
}

fn create_active_will(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    beneficiary: &Address,
    token: &Address,
) -> u64 {
    client.create_will(
        owner,
        &vec![env, (token.clone(), LOCKED_AMOUNT)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env],
        &0,
        &None,
        &0,
    )
}

#[test]
fn stale_snapshot_cannot_overwrite_a_newer_status() {
    let (env, contract_id, client, owner, beneficiary, token) = setup();
    let will_id = create_active_will(&env, &client, &owner, &beneficiary, &token);
    let mut stale = client.get_will(&will_id);

    env.ledger().with_mut(|ledger| ledger.timestamp += 91 * DAY);
    client.trigger_will(&will_id, &None);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);

    let result = env.as_contract(&contract_id, || {
        status_transition::try_transition(
            &env,
            &mut stale,
            &[WillStatus::Active],
            WillStatus::Cancelled,
            WillError::WillNotActive,
        )
    });

    assert_eq!(result, Err(WillError::WillNotActive));
    assert_eq!(stale.status, WillStatus::Active);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);
}

#[test]
fn matching_snapshot_transitions_and_returns_the_previous_status() {
    let (env, contract_id, client, owner, beneficiary, token) = setup();
    let will_id = create_active_will(&env, &client, &owner, &beneficiary, &token);
    let mut snapshot = client.get_will(&will_id);

    let previous = env.as_contract(&contract_id, || {
        status_transition::try_transition(
            &env,
            &mut snapshot,
            &[WillStatus::Active],
            WillStatus::Cancelled,
            WillError::WillNotActive,
        )
    });

    assert_eq!(previous, Ok(WillStatus::Active));
    assert_eq!(client.get_will(&will_id).status, WillStatus::Cancelled);
}
