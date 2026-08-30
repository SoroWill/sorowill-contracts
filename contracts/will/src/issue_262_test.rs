#![cfg(test)]

//! Regression test for issue #262: `create_will`, `update_guardians`, and
//! `update_will_settings` all call `assert_valid_guardians` to check the
//! owner-not-a-guardian and no-duplicate-guardian invariants, but
//! `clone_will` copied `source.guardians` into the new will without
//! re-running this check -- so a caller who happens to be one of the
//! source will's guardians could clone it as the new will's owner, ending
//! up as their own guardian, which every other creation path rejects.

use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

#[test]
fn clone_will_rejects_a_new_owner_who_is_a_source_guardian() {
    let env = Env::default();
    env.mock_all_auths();

    let original_owner = Address::generate(&env);
    let guardian = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(original_owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&original_owner, &1_000_000);
    StellarAssetClient::new(&env, &token_address).mint(&guardian, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let source_id = client.create_will(
        &original_owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    // The source's guardian now tries to clone it as themselves -- they'd
    // end up as their own will's guardian, exactly what assert_valid_guardians
    // rejects for every other creation path.
    assert_eq!(
        client.try_clone_will(&source_id, &guardian, &vec![&env, (token_address, 500_000_i128)]),
        Err(Ok(WillError::OwnerCannotBeGuardian.into()))
    );
}
