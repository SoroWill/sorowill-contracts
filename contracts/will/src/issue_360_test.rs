#![cfg(test)]

//! Regression coverage for issue #360: `get_wills_by_owner_and_status` must
//! return an empty page for `limit == 0`, like `get_wills_by_owner` and
//! `get_wills_by_beneficiary` (which page through `storage::paginate_ids`).
//!
//! The status-filtered query used to push a matching will and *then* compare
//! the page length against the page size, so a `limit` of `0` still returned
//! one will — a client paginating with a computed limit that reached zero saw
//! inconsistent results between the two query families.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

/// Creates `count` `Active` wills for `owner` and returns their ids in index
/// (ascending) order.
fn create_active_wills(
    env: &Env,
    client: &WillContractClient<'_>,
    owner: &Address,
    token_address: &Address,
) -> soroban_sdk::Vec<u64> {
    let mut ids = soroban_sdk::Vec::new(env);
    for i in 0..3 {
        ids.push_back(client.create_will(
            owner,
            &vec![env, (token_address.clone(), 100_000_i128 + i as i128)],
            &vec![
                env,
                Beneficiary {
                    address: Address::generate(env),
                    allocation: Allocation::Percentage(10_000),
                },
            ],
            &90,
            &7,
            &vec![env],
            &2,
            &None,
            &0,
        ));
    }
    ids
}

#[test]
fn limit_zero_returns_an_empty_page() {
    let (env, client, owner, token_address) = setup();
    let ids = create_active_wills(&env, &client, &owner, &token_address);
    assert_eq!(ids.len(), 3);

    let page = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &0);
    assert!(
        page.is_empty(),
        "limit 0 must produce an empty page, got {} wills",
        page.len()
    );

    // Parity with the id-based queries, which already behave this way.
    assert!(client.get_wills_by_owner(&owner, &None, &0).is_empty());

    // The cursor does not change that: either way the page is empty.
    let after_first = client.get_wills_by_owner_and_status(
        &owner,
        &WillStatus::Active,
        &Some(ids.get_unchecked(0)),
        &0,
    );
    assert!(after_first.is_empty());
}

#[test]
fn nonzero_limits_still_page_normally() {
    let (env, client, owner, token_address) = setup();
    let ids = create_active_wills(&env, &client, &owner, &token_address);

    let page = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &1);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0).unwrap().id, ids.get_unchecked(0));

    let next = client.get_wills_by_owner_and_status(
        &owner,
        &WillStatus::Active,
        &Some(ids.get_unchecked(0)),
        &5,
    );
    assert_eq!(next.len(), 2);
    assert_eq!(next.get(0).unwrap().id, ids.get_unchecked(1));

    let all = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &100);
    assert_eq!(all.len(), 3);
}
