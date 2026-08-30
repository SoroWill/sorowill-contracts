#![cfg(test)]

//! Regression coverage for `get_wills` (#214): asserts the batch-by-id fetch
//! preserves input order, silently skips nonexistent ids, and panics with
//! `TooManyIds` when given more than `MAX_GET_WILLS_IDS` entries.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

#[test]
fn preserves_input_order_and_skips_missing_ids() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);

    let make_will = || {
        client.create_will(
            &owner,
            &vec![&env, (token_address.clone(), 1_000_000_i128)],
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
        )
    };

    let will_a = make_will();
    let will_b = make_will();
    let will_c = make_will();

    // Request in a deliberately reversed order with a nonexistent id
    // interleaved; the result must preserve the requested order and skip
    // the nonexistent id, not filter-then-reorder.
    let nonexistent_id = will_c + 1000;
    let result = client.get_wills(&vec![&env, will_c, nonexistent_id, will_a, will_b]);

    assert_eq!(result.len(), 3);
    assert_eq!(result.get(0).unwrap().id, will_c);
    assert_eq!(result.get(1).unwrap().id, will_a);
    assert_eq!(result.get(2).unwrap().id, will_b);
}

#[test]
fn returns_empty_for_all_nonexistent_ids() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let result = client.get_wills(&vec![&env, 999_u64, 1000_u64]);
    assert!(result.is_empty());
}

#[test]
fn duplicate_ids_produce_duplicate_result_entries() {
    // Passing the same id twice must yield two copies of the same Will, not
    // one — the function is documented to be order-preserving with no
    // deduplication. Any client-side aggregation (e.g. summing balances) is
    // the caller's responsibility to guard against double-counting.
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &2_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
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

    // Pass the same id twice.
    let result = client.get_wills(&vec![&env, will_id, will_id]);

    // Both occurrences must appear in the output, in order.
    assert_eq!(result.len(), 2);
    assert_eq!(result.get(0).unwrap().id, will_id);
    assert_eq!(result.get(1).unwrap().id, will_id);
}

#[test]
fn panics_when_ids_exceed_max_get_wills_ids() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let mut ids = soroban_sdk::Vec::new(&env);
    for i in 0..51u64 {
        ids.push_back(i);
    }

    assert_eq!(
        client.try_get_wills(&ids),
        Err(Ok(WillError::TooManyIds.into()))
    );
}
