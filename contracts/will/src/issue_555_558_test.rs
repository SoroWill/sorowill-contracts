#![cfg(test)]

//! Regression tests for issues #555-#558:
//! Dust handling: refunded to owner or retained.
//!
//! Verifies that after distribution no dust is retained in contract storage:
//! - For percentage beneficiaries, rounding dust is paid to the final beneficiary.
//! - For fixed-amount-only wills, unallocated remainder is refunded to the owner.
//! - In all cases, `will.balances` is cleared and the contract retains zero units.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, WillStatus,
};

const DAY: u64 = 86_400;

#[test]
fn test_percentage_dust_not_retained_in_contract() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &token_address);
    token_client.mint(&owner, &100);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    // 100 units split across 3 equal beneficiaries (3333, 3333, 3334 bp)
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_i128)],
        &vec![
            &env,
            Beneficiary {
                address: b1.clone(),
                allocation: Allocation::Percentage(3333),
            },
            Beneficiary {
                address: b2.clone(),
                allocation: Allocation::Percentage(3333),
            },
            Beneficiary {
                address: b3.clone(),
                allocation: Allocation::Percentage(3334),
            },
        ],
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

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    // Contract balance map for this token is completely cleared (no dust retained)
    assert_eq!(will.balances.get(token_address.clone()), None);

    let token_contract = soroban_sdk::token::Client::new(&env, &token_address);
    assert_eq!(token_contract.balance(&contract_id), 0, "No dust must be retained by contract");
    assert_eq!(token_contract.balance(&b1), 33);
    assert_eq!(token_contract.balance(&b2), 33);
    assert_eq!(token_contract.balance(&b3), 34);
}

#[test]
fn test_fixed_amount_dust_refunded_to_owner_not_retained() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let token_client = StellarAssetClient::new(&env, &token_address);
    token_client.mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary = Address::generate(&env);

    // Will holds 1,000,000 but fixed beneficiary only allocated 400,000
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::FixedAmount(400_000),
            },
        ],
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

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.balances.get(token_address.clone()), None);

    let token_contract = soroban_sdk::token::Client::new(&env, &token_address);
    // Contract retains 0 dust
    assert_eq!(token_contract.balance(&contract_id), 0, "Contract must retain 0 dust");
    // Beneficiary receives 400,000
    assert_eq!(token_contract.balance(&beneficiary), 400_000);
    // Owner receives 600,000 remainder refunded
    assert_eq!(token_contract.balance(&owner), 600_000);
}
