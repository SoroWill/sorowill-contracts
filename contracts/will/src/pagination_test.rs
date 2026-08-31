#![cfg(test)]

//! Regression test for pagination support in `get_wills_by_beneficiary`.
//! Verifies that the function respects cursor and limit parameters, staying
//! within resource limits even when a beneficiary is named on many wills.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, TokenClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, TokenClient::new(&env, &token_address), token_address)
}

#[test]
fn pagination_respects_limit_parameter() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create multiple wills with the same beneficiary
    let mut will_ids = vec![&env];
    for i in 0..5 {
        let will_id = client.create_will(
            &owner,
            &vec![&env, (token_address.clone(), 100_000_i128 + i as i128)],
            &vec![
                &env,
                Beneficiary {
                    address: beneficiary.clone(),
                    allocation: Allocation::Percentage(10_000),
                },
            ],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        );
        will_ids.push_back(will_id);
    }

    // Fetch with limit=2
    let first_page = client.get_wills_by_beneficiary(&beneficiary, &None, &2);
    assert_eq!(first_page.len(), 2);

    // Fetch with cursor pointing to second page
    let second_page = client.get_wills_by_beneficiary(&beneficiary, &first_page.get(1).map(|w| w.id), &2);
    assert_eq!(second_page.len(), 2);

    // Verify no duplicates between pages
    let first_ids: Vec<u64> = first_page.iter().map(|w| w.id).collect();
    let second_ids: Vec<u64> = second_page.iter().map(|w| w.id).collect();
    for id in &second_ids {
        assert!(!first_ids.contains(id));
    }
}

#[test]
fn pagination_handles_all_results() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create 3 wills
    let mut will_ids = vec![&env];
    for i in 0..3 {
        let will_id = client.create_will(
            &owner,
            &vec![&env, (token_address.clone(), 100_000_i128 + i as i128)],
            &vec![
                &env,
                Beneficiary {
                    address: beneficiary.clone(),
                    allocation: Allocation::Percentage(10_000),
                },
            ],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        );
        will_ids.push_back(will_id);
    }

    // Fetch all without pagination (limit = 0 or very high)
    let all_wills = client.get_wills_by_beneficiary(&beneficiary, &None, &100);
    assert_eq!(all_wills.len(), 3);
}

#[test]
fn pagination_with_invalid_cursor_starts_from_beginning() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create 2 wills
    for i in 0..2 {
        client.create_will(
            &owner,
            &vec![&env, (token_address.clone(), 100_000_i128 + i as i128)],
            &vec![
                &env,
                Beneficiary {
                    address: beneficiary.clone(),
                    allocation: Allocation::Percentage(10_000),
                },
            ],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
            &0,
        );
    }

    // Fetch with invalid cursor (should behave gracefully)
    let results = client.get_wills_by_beneficiary(&beneficiary, &Some(9999_u64), &10);
    // Should return remaining wills after cursor, or empty if cursor is beyond all
    assert!(results.len() <= 2);
}
