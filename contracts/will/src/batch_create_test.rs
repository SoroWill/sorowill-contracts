#![cfg(test)]

//! Tests for `batch_create_wills` (#217).
//!
//! Covers creating several wills in one call, all-or-nothing rejection when a
//! single spec is invalid, and the `BATCH_MAX` boundary.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

/// One `batch_create_wills` spec: tokens, beneficiaries, check-in period,
/// grace period, guardians, guardian threshold.
#[allow(clippy::type_complexity)]
type WillSpec = (
    Vec<(Address, i128)>,
    Vec<Beneficiary>,
    u64,
    u64,
    Vec<Address>,
    u32,
);

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &100_000_000);

    let contract_id = env.register(WillContract, ());

    (env, contract_id, owner, token_address)
}

/// A valid single-beneficiary spec locking `amount` of `token`.
fn spec(env: &Env, token: &Address, amount: i128, percentage_bps: u32) -> WillSpec {
    (
        vec![env, (token.clone(), amount)],
        vec![
            env,
            Beneficiary {
                address: Address::generate(env),
                allocation: Allocation::Percentage(percentage_bps),
            },
        ],
        90,
        7,
        vec![env],
        1,
    )
}

#[test]
fn batch_creates_every_will_in_one_call() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    let specs = vec![
        &env,
        spec(&env, &token, 100_000, 10_000),
        spec(&env, &token, 200_000, 10_000),
        spec(&env, &token, 300_000, 10_000),
    ];

    let ids = client.batch_create_wills(&owner, &specs);

    assert_eq!(ids.len(), 3);
    for i in 0..ids.len() {
        let will = client.get_will(&ids.get_unchecked(i));
        assert_eq!(will.owner, owner);
        assert_eq!(will.status, WillStatus::Active);
        assert_eq!(will.checkin_period_days, 90);
        assert_eq!(will.grace_period_days, 7);
        assert_eq!(will.beneficiaries.len(), 1);
        // Ids are distinct and allocated in order.
        if i > 0 {
            assert!(ids.get_unchecked(i) > ids.get_unchecked(i - 1));
        }
    }

    assert_eq!(client.get_will(&ids.get_unchecked(0)).balance, 100_000);
    assert_eq!(client.get_will(&ids.get_unchecked(1)).balance, 200_000);
    assert_eq!(client.get_will(&ids.get_unchecked(2)).balance, 300_000);
}

#[test]
fn batch_is_rejected_entirely_when_one_spec_is_invalid() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // The middle spec's percentages do not sum to 10,000 bps.
    let specs = vec![
        &env,
        spec(&env, &token, 100_000, 10_000),
        spec(&env, &token, 200_000, 9_000),
        spec(&env, &token, 300_000, 10_000),
    ];

    assert_eq!(
        client.try_batch_create_wills(&owner, &specs),
        Err(Ok(WillError::InvalidPercentages.into()))
    );

    // No will from the batch — not even the valid first one — may survive.
    assert_eq!(client.get_wills_by_owner(&owner, &None, &10).len(), 0);
}

#[test]
fn batch_accepts_batch_max_and_rejects_one_more() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // BATCH_MAX is 10.
    let mut at_max: Vec<WillSpec> = Vec::new(&env);
    for _ in 0..10 {
        at_max.push_back(spec(&env, &token, 100_000, 10_000));
    }
    assert_eq!(client.batch_create_wills(&owner, &at_max).len(), 10);

    let mut over_max: Vec<WillSpec> = Vec::new(&env);
    for _ in 0..11 {
        over_max.push_back(spec(&env, &token, 100_000, 10_000));
    }
    assert_eq!(
        client.try_batch_create_wills(&owner, &over_max),
        Err(Ok(WillError::TooManyBeneficiaries.into()))
    );
}

#[test]
fn batch_rejects_a_spec_exceeding_the_beneficiary_cap() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // MAX_BENEFICIARIES is 10; 11 entries must be rejected.
    let mut beneficiaries: Vec<Beneficiary> = Vec::new(&env);
    for _ in 0..11 {
        beneficiaries.push_back(Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::Percentage(1_000),
        });
    }
    let specs = vec![
        &env,
        (
            vec![&env, (token, 100_000_i128)],
            beneficiaries,
            90_u64,
            7_u64,
            vec![&env],
            1_u32,
        ),
    ];

    assert_eq!(
        client.try_batch_create_wills(&owner, &specs),
        Err(Ok(WillError::TooManyBeneficiaries.into()))
    );
    assert_eq!(client.get_wills_by_owner(&owner, &None, &10).len(), 0);
}

#[test]
fn batch_create_wills_atomicity_rolls_back_token_transfers() {
    let (env, contract_id, owner, token) = setup();
    let client = WillContractClient::new(&env, &contract_id);
    let token_client = soroban_sdk::token::Client::new(&env, &token);

    let initial_balance = token_client.balance(&owner);

    // Spec 1 is valid (locks 100_000). Spec 2 is invalid (percentage = 9_000 != 10_000).
    let specs = vec![
        &env,
        spec(&env, &token, 100_000, 10_000),
        spec(&env, &token, 200_000, 9_000),
    ];

    let res = client.try_batch_create_wills(&owner, &specs);
    assert!(res.is_err());

    // Owner's token balance must be completely unchanged (first spec's transfer was rolled back)
    assert_eq!(token_client.balance(&owner), initial_balance);
}
