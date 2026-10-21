#![cfg(test)]

//! Regression tests for issue #626: pagination by owner stays consistent when a
//! will is cancelled between two page requests.
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn a_cancel_between_pages_does_not_repeat_a_will() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token).mint(&owner, &10_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let mut ids = vec![&env];
    for _ in 0..3 {
        let id = client.create_will(
            &owner,
            &vec![&env, (token.clone(), 1_000_i128)],
            &vec![
                &env,
                Beneficiary { address: Address::generate(&env), allocation: Allocation::FixedAmount(500) },
            ],
            &90,
            &7,
            &vec![&env],
            &0,
            &None,
            &0,
        );
        ids.push_back(id);
    }

    let first = client.get_wills_by_owner(&owner, &None, &1);
    let first_id = first.get(0).unwrap().id;

    client.cancel_will(&ids.get(1).unwrap(), &owner);

    let second = client.get_wills_by_owner(&owner, &Some(first_id), &2);
    for will in second.iter() {
        assert_ne!(will.id, first_id, "the page after the cursor must not repeat it");
    }
}
