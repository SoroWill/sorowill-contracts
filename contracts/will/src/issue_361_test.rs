#![cfg(test)]

//! Regression coverage for issue #361: `renounce_beneficiary` must reject the
//! last remaining beneficiary.
//!
//! `create_will` and `update_beneficiaries` both require between 1 and
//! `MAX_BENEFICIARIES` beneficiaries, but `assert_valid_allocations` accepts an
//! empty list, so a sole beneficiary could renounce and leave an `Active` or
//! `Triggered` will with nobody for `release_inheritance` to pay.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    errors::WillError, Allocation, Beneficiary, WillContract, WillContractClient, WillStatus,
};

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

    (env, client, owner, token_address)
}

fn create_will(
    env: &Env,
    client: &WillContractClient<'_>,
    owner: &Address,
    token_address: &Address,
    beneficiaries: &[Allocation],
) -> u64 {
    let mut list = soroban_sdk::Vec::new(env);
    for allocation in beneficiaries {
        list.push_back(Beneficiary {
            address: Address::generate(env),
            allocation: allocation.clone(),
        });
    }

    client.create_will(
        owner,
        &vec![env, (token_address.clone(), 1_000_000_i128)],
        &list,
        &30,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    )
}

#[test]
fn sole_beneficiary_cannot_renounce_an_active_will() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &[Allocation::Percentage(10_000)],
    );
    let only_beneficiary = client
        .get_will(&will_id)
        .beneficiaries
        .get(0)
        .unwrap()
        .address;

    assert_eq!(
        client.try_renounce_beneficiary(&will_id, &only_beneficiary),
        Err(Ok(WillError::TooManyBeneficiaries.into())),
    );

    // The rejected call must not have left a partial update behind: the will
    // still has its beneficiary, its allocation, and its index entry.
    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), 1);
    assert_eq!(
        will.beneficiaries.get(0).unwrap().allocation,
        Allocation::Percentage(10_000)
    );
    assert_eq!(
        client
            .get_wills_by_beneficiary(&only_beneficiary, &None, &10)
            .len(),
        1
    );
}

#[test]
fn sole_beneficiary_cannot_renounce_a_triggered_will() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &[Allocation::FixedAmount(400_000)],
    );
    let only_beneficiary = client
        .get_will(&will_id)
        .beneficiaries
        .get(0)
        .unwrap()
        .address;

    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_will_status(&will_id), WillStatus::Triggered);

    assert_eq!(
        client.try_renounce_beneficiary(&will_id, &only_beneficiary),
        Err(Ok(WillError::TooManyBeneficiaries.into())),
    );
    assert_eq!(client.get_will(&will_id).beneficiaries.len(), 1);
}

#[test]
fn renouncing_one_of_several_beneficiaries_still_succeeds() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(
        &env,
        &client,
        &owner,
        &token_address,
        &[Allocation::Percentage(4_000), Allocation::Percentage(6_000)],
    );
    let first = client
        .get_will(&will_id)
        .beneficiaries
        .get(0)
        .unwrap()
        .address;

    client.renounce_beneficiary(&will_id, &first);

    // Only the last remaining beneficiary is protected; the survivor keeps a
    // valid, fully allocated list.
    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), 1);
    assert_eq!(
        will.beneficiaries.get(0).unwrap().allocation,
        Allocation::Percentage(10_000)
    );
}
