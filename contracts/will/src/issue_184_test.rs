#![cfg(test)]

//! Regression test for issue #184: `merge_wills` sums `will_a.balance + will_b.balance`
//! without checking both wills lock the same primary token. If wills have different
//! primary tokens, the sum becomes nonsensical.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

fn setup_with_two_tokens<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);

    // First token
    let sac1 = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address1 = sac1.address();
    StellarAssetClient::new(&env, &token_address1).mint(&owner, &1_000_000_000);

    // Second token
    let sac2 = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address2 = sac2.address();
    StellarAssetClient::new(&env, &token_address2).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (
        env.clone(),
        client,
        owner,
        TokenClient::new(&env, &token_address1),
        token_address1,
        TokenClient::new(&env, &token_address2),
        token_address2,
    )
}

/// Regression test asserting a merge attempt between wills with different
/// primary tokens is rejected.
#[test]
#[should_panic]
fn merge_wills_different_primary_token_panics() {
    let (env, client, owner, _, token_address_1, _, token_address_2) = setup_with_two_tokens();

    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    // Create will_a with token_address_1
    let tokens_1: SorobanVec<(Address, i128)> = vec![&env, (token_address_1, 1_000_000_i128)];
    let will_id_a = client.create_will(&owner, &tokens_1, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Create will_b with token_address_2
    let tokens_2: SorobanVec<(Address, i128)> = vec![&env, (token_address_2, 1_000_000_i128)];
    let will_id_b = client.create_will(&owner, &tokens_2, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Attempt to merge - this should panic because the primary tokens differ
    client.merge_wills(&owner, will_id_a, will_id_b);
}
