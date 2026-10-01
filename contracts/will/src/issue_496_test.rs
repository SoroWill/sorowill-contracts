#![cfg(test)]

//! Regression coverage for issue #496: percentage-sum validation must be
//! agnostic to the order `Allocation::Percentage` and `Allocation::FixedAmount`
//! entries appear in, not assume every `Percentage` entry comes before every
//! `FixedAmount` entry.
//!
//! `assert_valid_allocations` (the function actually used by `create_will`,
//! `update_beneficiaries`, and friends) already accumulates `percentage_total`
//! and `fixed_total` in a single pass over the list in whatever order it was
//! given, via one `match` per entry — it was never ordering-dependent to begin
//! with. These tests pin that against regression: beneficiaries alternating
//! `Percentage`/`FixedAmount`/`Percentage`/... must validate (and be rejected)
//! identically to the same set of entries in sorted order.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    (env.clone(), WillContractClient::new(&env, &env.register(WillContract, ())), owner, token_address)
}

#[test]
fn interleaved_percentage_and_fixed_amount_entries_that_sum_correctly_are_accepted() {
    let (env, client, owner, token_address) = setup();

    // Alternating Percentage / FixedAmount / Percentage / FixedAmount, summing
    // to exactly 10,000 bps across the Percentage entries, with the
    // FixedAmount entries' total well under the primary-token balance.
    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(3_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(50_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(7_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(25_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), 4);
}

#[test]
fn interleaved_entries_that_sum_incorrectly_are_still_rejected() {
    let (env, client, owner, token_address) = setup();

    // Same interleaving shape, but the Percentage entries sum to 9,000, not
    // 10,000 -- must still be caught regardless of where the FixedAmount
    // entries sit in the list.
    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(10_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(3_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(10_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(6_000),
        },
    ];

    let result = client.try_create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    assert_eq!(result, Err(Ok(WillError::InvalidPercentages.into())));
}

/// Same shape as above, exercised through `update_beneficiaries` instead of
/// `create_will`, since it runs the identical validator against a different
/// call site and acceptance criteria asks for allocation processing in
/// general to be order-agnostic, not just at creation.
#[test]
fn update_beneficiaries_accepts_interleaved_ordering_too() {
    let (env, client, owner, token_address) = setup();

    let initial = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &initial,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let updated = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(100_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(4_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(50_000),
        },
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(6_000),
        },
    ];

    client.update_beneficiaries(&will_id, &owner, &updated);

    let will = client.get_will(&will_id);
    assert_eq!(will.beneficiaries.len(), 4);
}
