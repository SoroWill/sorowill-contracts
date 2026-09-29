#![cfg(test)]

//! #422: `split_will` rejects a split list that names an address twice.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
#[should_panic]
fn split_will_rejects_duplicate_addresses() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::Percentage(5_000),
            },
            Beneficiary {
                address: b,
                allocation: Allocation::Percentage(5_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );
    client.split_will(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::Percentage(2_500),
            },
            Beneficiary {
                address: a,
                allocation: Allocation::Percentage(2_500),
            },
        ],
        &vec![&env, (token, 100_i128)],
    );
}
