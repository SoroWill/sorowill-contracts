#![cfg(test)]

//! Regression coverage for issue #390: every entry point that takes a token
//! list rejects an empty or oversized list with
//! [`WillError::InvalidTokenCount`], not
//! [`WillError::TooManyBeneficiaries`].
//!
//! An empty list is not "too many" of anything, and a caller who passed eleven
//! tokens and two beneficiaries deserves an error that names the list that was
//! actually at fault. The dedicated code makes the failure self-explanatory
//! for SDK and app users without renumbering any existing error code.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::{errors::WillError, Allocation, Beneficiary, WillContract, WillContractClient};

/// `MAX_TOKENS` in `lib.rs`. A list of this length is accepted; one longer is
/// rejected.
const MAX_TOKENS: u32 = 10;

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

fn beneficiary(env: &Env) -> Beneficiary {
    Beneficiary {
        address: Address::generate(env),
        allocation: Allocation::Percentage(10_000),
    }
}

/// Builds a token list of `len` distinct tokens, each worth `1` unit.
fn tokens(env: &Env, len: u32) -> Vec<(Address, i128)> {
    let mut list: Vec<(Address, i128)> = Vec::new(env);
    for _ in 0..len {
        let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
        list.push_back((sac.address(), 1_i128));
    }
    list
}

#[test]
fn create_will_rejects_an_empty_token_list_with_invalid_token_count() {
    let (env, client, owner, _token_address) = setup();

    assert_eq!(
        client.try_create_will(
            &owner,
            &vec![&env],
            &vec![&env, beneficiary(&env)],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}

#[test]
fn create_will_rejects_an_oversized_token_list_with_invalid_token_count() {
    let (env, client, owner, _token_address) = setup();

    // MAX_TOKENS + 1 tokens, with a valid single beneficiary: the error must
    // name the token list, not the beneficiary list.
    assert_eq!(
        client.try_create_will(
            &owner,
            &tokens(&env, MAX_TOKENS + 1),
            &vec![&env, beneficiary(&env)],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}

#[test]
fn create_will_still_reports_oversized_beneficiary_lists_separately() {
    let (env, client, owner, token_address) = setup();

    // The token list is valid here, so the only thing wrong is the beneficiary
    // list — which must keep its own error code.
    let mut beneficiaries: Vec<Beneficiary> = Vec::new(&env);
    for _ in 0..crate::MAX_BENEFICIARIES + 1 {
        beneficiaries.push_back(beneficiary(&env));
    }

    assert_eq!(
        client.try_create_will(
            &owner,
            &vec![&env, (token_address, 1_000_000_i128)],
            &beneficiaries,
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        ),
        Err(Ok(WillError::TooManyBeneficiaries.into())),
    );
}

#[test]
fn split_will_rejects_an_empty_token_list_with_invalid_token_count() {
    let (env, client, owner, token_address) = setup();
    let b = beneficiary(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, b.clone()],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    assert_eq!(
        client.try_split_will(&will_id, &owner, &vec![&env, b], &vec![&env]),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}

#[test]
fn split_will_rejects_an_oversized_token_list_with_invalid_token_count() {
    let (env, client, owner, token_address) = setup();
    let b = beneficiary(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, b.clone()],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    assert_eq!(
        client.try_split_will(
            &will_id,
            &owner,
            &vec![&env, b],
            &tokens(&env, MAX_TOKENS + 1),
        ),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}

#[test]
fn clone_will_rejects_an_empty_token_list_with_invalid_token_count() {
    let (env, client, owner, token_address) = setup();
    let b = beneficiary(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, b],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    assert_eq!(
        client.try_clone_will(&will_id, &owner, &vec![&env]),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}

#[test]
fn batch_create_wills_rejects_a_spec_with_an_empty_token_list() {
    let (env, client, owner, _token_address) = setup();

    assert_eq!(
        client.try_batch_create_wills(
            &owner,
            &vec![
                &env,
                (
                    vec![&env],
                    vec![&env, beneficiary(&env)],
                    90_u64,
                    7_u64,
                    vec![&env],
                    1_u32
                )
            ],
        ),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}

#[test]
fn batch_create_wills_rejects_a_spec_with_an_oversized_token_list() {
    let (env, client, owner, _token_address) = setup();

    assert_eq!(
        client.try_batch_create_wills(
            &owner,
            &vec![
                &env,
                (
                    tokens(&env, MAX_TOKENS + 1),
                    vec![&env, beneficiary(&env)],
                    90_u64,
                    7_u64,
                    vec![&env],
                    1_u32
                )
            ],
        ),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );
}
