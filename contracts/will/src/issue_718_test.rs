#![cfg(test)]

//! Regression tests for issue #718: a `beneficiary_updated` event must be
//! published whenever a will's beneficiary list is replaced, carrying the
//! will id, the owner, the ledger timestamp, and both the old and new lists.

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Symbol, TryIntoVal, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

type BeneficiaryUpdated = (
    u64,
    Address,
    u64,
    SorobanVec<Beneficiary>,
    SorobanVec<Beneficiary>,
);

fn setup(env: &Env) -> (WillContractClient, Address, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000i128);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (client, owner, token_address)
}

fn single(env: &Env, address: Address) -> SorobanVec<Beneficiary> {
    vec![
        env,
        Beneficiary {
            address,
            allocation: Allocation::Percentage(10_000),
        },
    ]
}

fn create(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
    beneficiaries: SorobanVec<Beneficiary>,
) -> u64 {
    let tokens = vec![env, (token.clone(), 1_000i128)];
    client.create_will(
        owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![env],
        &0,
        &None,
        &0,
    )
}

/// Returns the payload of the most recent `beneficiary_updated` event for
/// `will_id`, if one was published.
fn find_beneficiary_updated(env: &Env, will_id: u64) -> Option<BeneficiaryUpdated> {
    let topic = Symbol::new(env, "beneficiary_updated");
    let events = env.events().all();
    let mut found: Option<BeneficiaryUpdated> = None;
    for event in events.all().iter() {
        if event.1.is_empty() {
            continue;
        }
        let topic0: Result<Symbol, _> = event.1.get(0).unwrap().try_into_val(env);
        if topic0 != Ok(topic.clone()) {
            continue;
        }
        let topic1: Result<u64, _> = event.1.get(1).unwrap().try_into_val(env);
        if topic1 != Ok(will_id) {
            continue;
        }
        found = Some(event.2.try_into_val(env).unwrap());
    }
    found
}

/// `update_beneficiaries` emits `beneficiary_updated` with will id, owner,
/// timestamp, and both the old and new lists (#718).
#[test]
fn update_beneficiaries_emits_beneficiary_updated() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let old_addr = Address::generate(&env);
    let new_addr = Address::generate(&env);
    let old_list = single(&env, old_addr.clone());
    let will_id = create(&env, &client, &owner, &token, old_list.clone());

    let new_list = single(&env, new_addr.clone());
    client.update_beneficiaries(&will_id, &owner, &new_list);

    let (event_will_id, event_owner, timestamp, old, new) =
        find_beneficiary_updated(&env, will_id).expect("beneficiary_updated event not found");

    assert_eq!(event_will_id, will_id);
    assert_eq!(event_owner, owner);
    assert_eq!(timestamp, env.ledger().timestamp());
    assert_eq!(old, old_list, "event must carry the previous beneficiary list");
    assert_eq!(new, new_list, "event must carry the new beneficiary list");
}

/// The beneficiary branch of `update_will_settings` emits the same event
/// (#718), so the event fires whichever entry point changed the list.
#[test]
fn update_will_settings_emits_beneficiary_updated() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let old_list = single(&env, Address::generate(&env));
    let will_id = create(&env, &client, &owner, &token, old_list.clone());

    let new_list = single(&env, Address::generate(&env));
    client.update_will_settings(
        &will_id,
        &owner,
        &Some(new_list.clone()),
        &None,
        &None,
        &None,
    );

    let (_, _, _, old, new) =
        find_beneficiary_updated(&env, will_id).expect("beneficiary_updated event not found");
    assert_eq!(old, old_list);
    assert_eq!(new, new_list);
}
