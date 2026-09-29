#![cfg(test)]

//! Regression test for issue #282: `renounce_beneficiary` removes the caller
//! from `will.beneficiaries` and their reverse index entry, but nothing
//! tested that a second call to `renounce_beneficiary` by the same
//! (now-removed) address correctly fails with `WillError::BeneficiaryNotFound`
//! rather than silently succeeding as a no-op or double-redistributing shares
//! that no longer exist.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

#[test]
fn renouncing_twice_from_the_same_address_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
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
        &1,
        &None,
        &0,
    );

    // First renunciation succeeds and removes `a` from the beneficiary list.
    client.renounce_beneficiary(&will_id, &a);

    // A second renunciation from the same, now-removed address must be
    // rejected, not treated as a no-op or re-processed against a stale entry.
    assert_eq!(
        client.try_renounce_beneficiary(&will_id, &a),
        Err(Ok(WillError::BeneficiaryNotFound.into()))
    );
}
