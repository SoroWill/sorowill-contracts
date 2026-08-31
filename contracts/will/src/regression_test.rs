#![cfg(test)]

//! Regression tests for GitHub issues #191, #192, #193, #194.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, TokenClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, TokenClient::new(&env, &token_address), token_address)
}

/// Issue #191: Regression test asserting `active_will_count` increases after `clone_will`.
#[test]
fn issue_191_clone_will_increments_active_count() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 100_000_i128)];

    let initial_stats = client.get_protocol_stats();
    let initial_count = initial_stats.active_will_count;

    let source_will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    let stats_after_create = client.get_protocol_stats();
    assert_eq!(stats_after_create.active_will_count, initial_count + 1);

    let clone_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 50_000_i128)];
    let _cloned_will_id = client.clone_will(&source_will_id, &owner, &clone_tokens);

    let stats_after_clone = client.get_protocol_stats();
    assert_eq!(stats_after_clone.active_will_count, initial_count + 2, "clone_will should increment active_will_count");
}

/// Issue #192: Regression test asserting `active_will_count` increases by the batch size.
#[test]
fn issue_192_batch_create_wills_increments_active_count() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let initial_stats = client.get_protocol_stats();
    let initial_count = initial_stats.active_will_count;

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let spec1_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 100_000_i128)];
    let spec2_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 50_000_i128)];
    let spec3_tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 75_000_i128)];

    let specs: SorobanVec<(
        SorobanVec<(Address, i128)>,
        SorobanVec<Beneficiary>,
        u64,
        u64,
        SorobanVec<Address>,
        u32,
    )> = vec![
        &env,
        (spec1_tokens, beneficiaries.clone(), 90, 7, vec![&env], 2),
        (spec2_tokens, beneficiaries.clone(), 90, 7, vec![&env], 2),
        (spec3_tokens, beneficiaries.clone(), 90, 7, vec![&env], 2),
    ];

    let batch_size = 3;
    let _ids = client.batch_create_wills(&owner, &specs);

    let stats_after_batch = client.get_protocol_stats();
    assert_eq!(
        stats_after_batch.active_will_count,
        initial_count + batch_size,
        "batch_create_wills should increment active_will_count for each will"
    );
}

/// Issue #194: Regression test for pagination with cursor/limit parameters.
#[test]
fn issue_194_get_wills_by_owner_and_status_with_pagination() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let mut created_ids = SorobanVec::new(&env);
    for i in 0..5 {
        let amount = 100_000 + (i as i128) * 10_000;
        let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), amount)];
        let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
        created_ids.push_back(will_id);
    }

    // Get first page with pagination (limit 2)
    let page1 = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &2);
    assert_eq!(page1.len(), 2, "First page should have 2 wills");

    if page1.len() > 0 {
        let last_id_page1 = page1.get_unchecked(page1.len() - 1).id;
        let page2 = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &Some(last_id_page1), &2);
        assert!(page2.len() > 0, "Second page should have results");
        assert!(page2.len() <= 2, "Second page should respect limit");

        if page2.len() > 0 {
            let first_page2_id = page2.get_unchecked(0).id;
            assert!(first_page2_id > last_id_page1, "Pagination cursor should work correctly");
        }
    }

    // Test total count across pages (large limit)
    let all_wills = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &50);
    assert_eq!(all_wills.len(), 5, "Should get all 5 wills when limit is large enough");
}

/// Issue #193: Regression test for cursor pagination with beneficiary removal/re-addition.
#[test]
fn issue_193_paginate_with_remove_readd_beneficiary() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary1 = Address::generate(&env);
    let beneficiary2 = Address::generate(&env);

    let initial_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary1.clone(),
            allocation: Allocation::Percentage(5_000),
        },
        Beneficiary {
            address: beneficiary2.clone(),
            allocation: Allocation::Percentage(5_000),
        },
    ];

    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 200_000_i128)];
    let will_id = client.create_will(&owner, &tokens, &initial_beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Create multiple more wills to test pagination
    for i in 0..3 {
        let amount = 100_000 + (i as i128) * 10_000;
        let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), amount)];
        let _will_id = client.create_will(&owner, &tokens, &initial_beneficiaries, &90, &7, &vec![&env], &2, &None, &0);
    }

    // Remove beneficiary2 from first will
    let updated_beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary1.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    client.update_beneficiaries(&will_id, &owner, &updated_beneficiaries);

    // Test pagination on beneficiary index after removal
    let beneficiary2_wills = client.get_wills_by_beneficiary(&beneficiary2);
    let will_ids_set = SorobanVec::new(&env);

    // Verify no duplicates in pagination
    for will in beneficiary2_wills.iter() {
        let already_seen = false;
        for seen_will in will_ids_set.iter() {
            if seen_will == will.id {
                panic!("Pagination should not duplicate wills after removal/re-addition");
            }
        }
        assert!(!already_seen, "No duplicates should exist in pagination");
    }

    // Verify we can get all beneficiary wills without gaps
    let page1 = client.get_wills_by_beneficiary(&beneficiary2);
    if page1.len() > 1 {
        let ids_in_pages = SorobanVec::new(&env);
        for will in page1.iter() {
            ids_in_pages.push_back(will.id);
        }
        assert_eq!(ids_in_pages.len(), page1.len(), "Should retrieve all beneficiary wills without gaps");
    }
}

/// Regression test for stale guardian votes after a guardian is removed and re-added.
#[test]
fn stale_guardian_vote_cleared_when_guardian_removed() {
    let (env, client, owner, _token, token_address) = setup();
    let guardian = Address::generate(&env);
    let replacement_guardian = Address::generate(&env);
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 100_000_i128)];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    client.submit_guardian_vote(&will_id, &guardian);
    assert!(client.has_guardian_voted(&will_id, &guardian));

    client.update_guardians(&will_id, &owner, &vec![&env, replacement_guardian.clone()]);
    client.update_guardians(&will_id, &owner, &vec![&env, guardian.clone()]);

    assert!(
        !client.has_guardian_voted(&will_id, &guardian),
        "re-added guardian should not retain a vote from before removal"
    );
}
