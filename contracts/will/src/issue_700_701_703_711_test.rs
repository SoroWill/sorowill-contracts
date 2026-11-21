#![cfg(test)]

//! Regression tests for issues #700, #701, #703, and #711.

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec,
};

use crate::{
    proportional_share, proportional_share_with_denom, Allocation, Beneficiary, WillContract,
    WillContractClient, WillError, MAX_BENEFICIARIES,
};

const DAY: u64 = 86_400;

/// Dummy contract that implements no SEP-41 methods.
#[contract]
pub struct NotAToken;

#[contractimpl]
impl NotAToken {
    pub fn ping(_env: Env) -> u32 {
        1
    }
}

fn setup(env: &Env) -> (WillContractClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    (client, owner, token_address, contract_id)
}

// ── #711: Beneficiary address validation ────────────────────────────────────

#[test]
fn create_will_rejects_contract_self_as_beneficiary() {
    let env = Env::default();
    let (client, owner, token_address, contract_id) = setup(&env);

    let result = client.try_create_will(
        &owner,
        &vec![&env, (token_address, 1_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: contract_id,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    assert_eq!(result, Err(Ok(WillError::InvalidBeneficiaryAddress)));
}

#[test]
fn update_beneficiaries_rejects_contract_self() {
    let env = Env::default();
    let (client, owner, token_address, contract_id) = setup(&env);
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let result = client.try_update_beneficiaries(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: contract_id,
                allocation: Allocation::Percentage(10_000),
            },
        ],
    );

    assert_eq!(result, Err(Ok(WillError::InvalidBeneficiaryAddress)));
}

// ── #703: SEP-41 token validation across entry points ──────────────────────

#[test]
fn top_up_clone_and_batch_reject_non_sep41_token() {
    let env = Env::default();
    let (client, owner, token_address, _contract_id) = setup(&env);
    let beneficiary = Address::generate(&env);
    let fake_token = env.register(NotAToken, ());

    let beneficiaries = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    assert_eq!(
        client.try_top_up(&will_id, &owner, &fake_token, &500_i128),
        Err(Ok(WillError::InvalidToken))
    );

    assert_eq!(
        client.try_clone_will(&will_id, &owner, &vec![&env, (fake_token.clone(), 500_i128)]),
        Err(Ok(WillError::InvalidToken))
    );

    let batch_specs = vec![
        &env,
        (
            vec![&env, (fake_token, 500_i128)],
            beneficiaries,
            90_u64,
            7_u64,
            vec![&env],
            0_u32,
            None,
            0_u64,
        ),
    ];
    assert_eq!(
        client.try_batch_create_wills(&owner, &batch_specs),
        Err(Ok(WillError::InvalidToken))
    );
}

// ── #701: Maximum beneficiary list handling ────────────────────────────────

#[test]
fn max_beneficiaries_full_lifecycle_and_overflow_rejection() {
    let env = Env::default();
    let (client, owner, token_address, _contract_id) = setup(&env);

    let mut beneficiaries: Vec<Beneficiary> = Vec::new(&env);
    let mut addrs: Vec<Address> = Vec::new(&env);
    let even_bp = 10_000 / MAX_BENEFICIARIES;
    let mut allocated = 0_u32;

    for i in 0..MAX_BENEFICIARIES {
        let addr = Address::generate(&env);
        let bp = if i == MAX_BENEFICIARIES - 1 {
            10_000 - allocated
        } else {
            even_bp
        };
        allocated += bp;
        addrs.push_back(addr.clone());
        beneficiaries.push_back(Beneficiary {
            address: addr,
            allocation: Allocation::Percentage(bp),
        });
    }

    // Exceeding MAX_BENEFICIARIES must be rejected.
    let mut oversized = beneficiaries.clone();
    oversized.push_back(Beneficiary {
        address: Address::generate(&env),
        allocation: Allocation::FixedAmount(1),
    });
    assert_eq!(
        client.try_create_will(
            &owner,
            &vec![&env, (token_address.clone(), 100_000_i128)],
            &oversized,
            &90,
            &7,
            &vec![&env],
            &0,
            &None,
            &0,
        ),
        Err(Ok(WillError::TooManyBeneficiaries))
    );

    // Exactly MAX_BENEFICIARIES succeeds through create, trigger, and release.
    let total_amount = 100_000_i128;
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), total_amount)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    let token = TokenClient::new(&env, &token_address);
    let mut paid_sum = 0_i128;
    for addr in addrs.iter() {
        let bal = token.balance(&addr);
        assert_eq!(bal, 10_000_i128);
        paid_sum += bal;
    }
    assert_eq!(paid_sum, total_amount);
}

// ── #700: Overflow-safe percentage math on large balances ──────────────────

#[test]
fn percentage_math_does_not_overflow_on_i128_max() {
    // Direct proportional_share at i128::MAX
    let half = proportional_share(i128::MAX, 5_000);
    assert_eq!(half, i128::MAX / 2);

    // Non-10_000 denominator (used by reveal_and_claim) at i128::MAX
    let claim = proportional_share_with_denom(i128::MAX, 2_500, 5_000);
    assert_eq!(claim, i128::MAX / 2);

    let full = proportional_share_with_denom(i128::MAX, 7_500, 7_500);
    assert_eq!(full, i128::MAX);
}
