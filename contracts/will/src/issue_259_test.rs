#![cfg(test)]

//! Regression test for issue #259: if a `cursor` was valid on a previous
//! page but the will it names was since removed from the index (e.g.
//! cancelled), `paginate_ids` must still return the remaining entries
//! rather than incorrectly concluding the list is exhausted.
//!
//! `paginate_ids`'s skip logic already compares ids numerically (`id <=
//! cursor_val`) rather than searching for an exact match on the cursor id,
//! so this test documents and locks in that the existing implementation
//! already handles a missing cursor id correctly -- no production code
//! change was needed for this specific behavior.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn pagination_skips_past_a_cursor_id_removed_from_the_index() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let mut will_ids: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&env);
    for _ in 0..5 {
        let will_id = client.create_will(
            &owner,
            &vec![&env, (token_address.clone(), 100_000_i128)],
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
            &1,
            &None,
            &0,
        );
        will_ids.push_back(will_id);
    }
    let cursor_boundary_id = will_ids.get_unchecked(2); // the 3rd will created

    // Cancel the 3rd will, removing it from the owner's index entirely.
    client.cancel_will(&cursor_boundary_id, &owner);

    // Paginate starting right at the now-missing id. The remaining, larger
    // ids must still come back -- an empty page here would mean a
    // paginating client wrongly concludes it has reached the end.
    let page = client.get_wills_by_owner(&owner, &Some(cursor_boundary_id), &10);

    assert_eq!(page.len(), 2, "expected the two wills created after the cancelled one");
    assert_eq!(page.get_unchecked(0).id, will_ids.get_unchecked(3));
    assert_eq!(page.get_unchecked(1).id, will_ids.get_unchecked(4));
}
