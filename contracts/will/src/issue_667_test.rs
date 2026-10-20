#![cfg(test)]

//! Regression test for issue #667: `merge_wills` validates that source and
//! destination wills have compatible, overlapping token lists and do not exceed limits.
//!
//! Merging wills with completely non-overlapping (disjoint) token sets creates
//! ambiguous fixed-amount semantics because `Allocation::FixedAmount` claims are
//! denominated in the primary token. Furthermore, merging must enforce that
//! combined token counts do not exceed `MAX_TOKENS`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

fn create_token<'a>(env: &Env, admin: &Address) -> (TokenClient<'a>, Address) {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = sac.address();
    StellarAssetClient::new(env, &addr).mint(admin, &10_000_000);
    (TokenClient::new(env, &addr), addr)
}

fn setup_env<'a>() -> (Env, WillContractClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner)
}

/// Disjoint token sets must be rejected with PrimaryTokenMismatch because
/// non-overlapping tokens create ambiguous fixed-amount semantics (#667).
#[test]
#[should_panic]
fn merge_wills_disjoint_token_sets_panics() {
    let (env, client, owner) = setup_env();
    let (_, token1) = create_token(&env, &owner);
    let (_, token2) = create_token(&env, &owner);

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let tokens_a: SorobanVec<(Address, i128)> = vec![&env, (token1, 100_000)];
    let will_id_a = client.create_will(
        &owner,
        &tokens_a,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let tokens_b: SorobanVec<(Address, i128)> = vec![&env, (token2, 100_000)];
    let will_id_b = client.create_will(
        &owner,
        &tokens_b,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    client.merge_wills(&owner, &will_id_a, &will_id_b);
}

/// Merging wills that share an overlapping primary token and include other multi-token
/// balances must succeed and combine balances for all distinct tokens.
#[test]
fn merge_wills_overlapping_token_sets_succeeds() {
    let (env, client, owner) = setup_env();
    let (_, token_primary) = create_token(&env, &owner);
    let (_, token_extra_a) = create_token(&env, &owner);
    let (_, token_extra_b) = create_token(&env, &owner);

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let tokens_a: SorobanVec<(Address, i128)> = vec![
        &env,
        (token_primary.clone(), 100_000),
        (token_extra_a.clone(), 50_000),
    ];
    let will_id_a = client.create_will(
        &owner,
        &tokens_a,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let tokens_b: SorobanVec<(Address, i128)> = vec![
        &env,
        (token_primary.clone(), 200_000),
        (token_extra_b.clone(), 80_000),
    ];
    let will_id_b = client.create_will(
        &owner,
        &tokens_b,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    client.merge_wills(&owner, &will_id_a, &will_id_b);

    let merged_will = client.get_will(&will_id_a);
    assert_eq!(merged_will.balances.get(token_primary).unwrap(), 300_000);
    assert_eq!(merged_will.balances.get(token_extra_a).unwrap(), 50_000);
    assert_eq!(merged_will.balances.get(token_extra_b).unwrap(), 80_000);
    assert_eq!(merged_will.balances.len(), 3);
}

/// Merging wills whose combined distinct tokens exceed MAX_TOKENS (10) must panic
/// with MergeWouldExceedLimits (#667).
#[test]
#[should_panic]
fn merge_wills_exceeding_max_tokens_panics() {
    let (env, client, owner) = setup_env();

    // Create 11 distinct tokens
    let mut tokens: std::vec::Vec<Address> = std::vec::Vec::new();
    for _ in 0..11 {
        let (_, addr) = create_token(&env, &owner);
        tokens.push(addr);
    }

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];

    // Will A has tokens 0..6 (6 tokens, primary is tokens[0])
    let mut tokens_a: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
    for addr in tokens[0..6].iter() {
        tokens_a.push_back((addr.clone(), 10_000));
    }
    let will_id_a = client.create_will(
        &owner,
        &tokens_a,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Will B has token 0 plus tokens 6..11 (6 tokens, primary is tokens[0])
    let mut tokens_b: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
    tokens_b.push_back((tokens[0].clone(), 10_000));
    for addr in tokens[6..11].iter() {
        tokens_b.push_back((addr.clone(), 10_000));
    }
    let will_id_b = client.create_will(
        &owner,
        &tokens_b,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Combined distinct tokens = 1 + 5 + 5 = 11 > MAX_TOKENS (10) -> panics
    client.merge_wills(&owner, &will_id_a, &will_id_b);
}
