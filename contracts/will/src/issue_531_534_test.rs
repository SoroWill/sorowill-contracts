#![cfg(test)]

//! Regression and edge-case coverage for issues #531-#534:
//! Testing distribution behavior with small balances and many percentage beneficiaries.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (
        env.clone(),
        client,
        owner,
        TokenClient::new(&env, &token_address),
        token_address,
    )
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

fn release(env: &Env, client: &WillContractClient, will_id: u64) {
    advance(env, 91);
    client.trigger_will(&will_id);
    advance(env, 8);
    client.release_inheritance(&will_id, &None);
}

#[test]
fn test_small_balance_with_many_percentage_beneficiaries() {
    let (env, client, owner, token, token_address) = setup();

    // Setup 5 percentage beneficiaries with unequal shares totaling 10,000 bps
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);
    let b4 = Address::generate(&env);
    let b5 = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: b1.clone(),
            allocation: Allocation::Percentage(2000), // 20%
        },
        Beneficiary {
            address: b2.clone(),
            allocation: Allocation::Percentage(2000), // 20%
        },
        Beneficiary {
            address: b3.clone(),
            allocation: Allocation::Percentage(2000), // 20%
        },
        Beneficiary {
            address: b4.clone(),
            allocation: Allocation::Percentage(2000), // 20%
        },
        Beneficiary {
            address: b5.clone(),
            allocation: Allocation::Percentage(2000), // 20%
        },
    ];

    // Create will with a tiny balance of 7 units (which is smaller than 10,000 basis points)
    let small_balance: i128 = 7;
    let will_id = client.create_will(
        &owner,
        &token_address,
        &small_balance,
        &beneficiaries,
        &vec![&env],
        &90,
        &7,
        &0,
    );

    // Release will and verify conservation of funds
    release(&env, &client, will_id);

    let bal1 = token.balance(&b1);
    let bal2 = token.balance(&b2);
    let bal3 = token.balance(&b3);
    let bal4 = token.balance(&b4);
    let bal5 = token.balance(&b5);

    let total_distributed = bal1 + bal2 + bal3 + bal4 + bal5;
    assert_eq!(
        total_distributed, small_balance,
        "Total distributed across all percentage beneficiaries must equal small initial balance"
    );
    assert!(bal1 >= 0 && bal2 >= 0 && bal3 >= 0 && bal4 >= 0 && bal5 >= 0);
}

#[test]
fn test_ten_percentage_beneficiaries_with_balance_smaller_than_count() {
    let (env, client, owner, token, token_address) = setup();

    let mut bens: SorobanVec<Beneficiary> = SorobanVec::new(&env);
    let mut addrs: std::vec::Vec<Address> = std::vec::Vec::new();
    for _ in 0..10 {
        let a = Address::generate(&env);
        addrs.push(a.clone());
        bens.push_back(Beneficiary {
            address: a,
            allocation: Allocation::Percentage(1000), // 10% each = 10,000 bps
        });
    }

    // Balance of 4 units among 10 beneficiaries
    let balance: i128 = 4;
    let will_id = client.create_will(
        &owner,
        &token_address,
        &balance,
        &bens,
        &vec![&env],
        &90,
        &7,
        &0,
    );

    release(&env, &client, will_id);

    let mut sum: i128 = 0;
    for a in addrs.iter() {
        let b = token.balance(a);
        assert!(b >= 0);
        sum += b;
    }
    assert_eq!(sum, balance, "Exact sum must equal balance without dust loss");
}
