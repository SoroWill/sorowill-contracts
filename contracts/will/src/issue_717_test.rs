#![cfg(test)]

//! Regression tests for issue #717: a guardian list must not contain the same
//! address twice, because each duplicate would count toward quorum again.
//!
//! The rule is enforced by `assert_valid_guardians`, which every guardian
//! entry point calls and which reports [`WillError::DuplicateGuardian`]. These
//! tests pin that behaviour on the creation and update entry points.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, GuardianSpec, WillContract, WillContractClient};

fn setup(env: &Env) -> (WillContractClient, Address, Address) {
    env.mock_all_auths();

    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000i128);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (client, owner, token_address)
}

fn beneficiaries(env: &Env) -> soroban_sdk::Vec<Beneficiary> {
    vec![
        env,
        Beneficiary {
            address: Address::generate(env),
            allocation: Allocation::Percentage(10_000),
        },
    ]
}

fn create_with_guardians(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
    guardians: soroban_sdk::Vec<Address>,
    threshold: u32,
) -> u64 {
    let tokens = vec![env, (token.clone(), 1_000i128)];
    client.create_will(
        owner,
        &tokens,
        &beneficiaries(env),
        &90,
        &7,
        &guardians,
        &threshold,
        &None,
        &0,
    )
}

/// Creating a will with the same guardian listed twice is rejected (#717).
#[test]
fn create_will_rejects_duplicate_guardians() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let guardian = Address::generate(&env);
    let guardians = vec![&env, guardian.clone(), guardian.clone()];

    let result = client.try_create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000i128)],
        &beneficiaries(&env),
        &90,
        &7,
        &guardians,
        &1,
        &None,
        &0,
    );
    assert!(result.is_err(), "duplicate guardian must be rejected at creation");
}

/// A guardian list of distinct addresses is accepted (control case for #717).
#[test]
fn create_will_accepts_distinct_guardians() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let guardians = vec![
        &env,
        Address::generate(&env),
        Address::generate(&env),
    ];
    let will_id = create_with_guardians(&env, &client, &owner, &token, guardians, 2);
    assert_eq!(client.get_will(&will_id).guardians.len(), 2);
}

/// `update_guardians` rejects a replacement list containing a duplicate (#717).
#[test]
fn update_guardians_rejects_duplicates() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let original = vec![&env, Address::generate(&env)];
    let will_id = create_with_guardians(&env, &client, &owner, &token, original, 1);

    let guardian = Address::generate(&env);
    let duplicated = vec![&env, guardian.clone(), guardian.clone()];
    let result = client.try_update_guardians(&will_id, &owner, &duplicated);
    assert!(result.is_err(), "duplicate guardian must be rejected on update");
}

/// `update_guardians_weighted` rejects a weighted list containing a duplicate
/// address, even when the entries carry different weights (#717).
#[test]
fn update_guardians_weighted_rejects_duplicates() {
    let env = Env::default();
    let (client, owner, token) = setup(&env);

    let original = vec![&env, Address::generate(&env)];
    let will_id = create_with_guardians(&env, &client, &owner, &token, original, 1);

    let guardian = Address::generate(&env);
    let specs = vec![
        &env,
        GuardianSpec {
            address: guardian.clone(),
            weight: 1,
        },
        GuardianSpec {
            address: guardian.clone(),
            weight: 2,
        },
    ];
    let result = client.try_update_guardians_weighted(&will_id, &owner, &specs, &None);
    assert!(
        result.is_err(),
        "duplicate guardian with differing weights must be rejected"
    );
}
