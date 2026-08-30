#![cfg(test)]

//! Regression test for issue #260: `create_will` used to call
//! `index_by_owner`/`index_by_beneficiary` (which can panic with
//! `WillError::TooManyWills`) only *after* the token transfer already
//! succeeded, wasting the resource budget on invocations that were always
//! going to fail. `assert_index_capacity` now runs before any transfer.
//!
//! Because a panic aborts the whole transaction, the *final on-chain state*
//! looks identical either way (the transfer would be rolled back
//! regardless of ordering) -- so this test can't distinguish the two by
//! checking balances afterward. Instead it uses a token whose `transfer`
//! always panics with a distinct failure, and confirms the observed error
//! is `TooManyWills`, not the poisoned transfer: proof the capacity check
//! ran first and the transfer was never reached.

use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, token::StellarAssetClient, vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

/// A token that answers `decimals()` like a real SEP-41 token but panics
/// unconditionally inside `transfer`, so any call that reaches the transfer
/// step is unmistakably distinguishable from one rejected earlier.
#[contract]
pub struct PoisonToken;

#[contractimpl]
impl PoisonToken {
    pub fn decimals(_env: Env) -> u32 {
        7
    }

    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {
        panic!("PoisonToken::transfer must never be reached by this test");
    }
}

#[test]
fn create_will_checks_index_capacity_before_any_token_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let real_token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &real_token).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Fill the owner's index to the test-mode-lowered MAX_WILLS_PER_INDEX
    // (5) using a real token, so these all succeed normally.
    for _ in 0..5 {
        client.create_will(
            &owner,
            &vec![&env, (real_token.clone(), 1_000_i128)],
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
    }

    // The 6th call is already doomed by the owner's index being at
    // capacity. It uses PoisonToken so that, if the transfer were ever
    // reached, we'd see PoisonToken's panic instead of TooManyWills.
    let poison_token = env.register(PoisonToken, ());
    let result = client.try_create_will(
        &owner,
        &vec![&env, (poison_token, 1_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
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

    assert_eq!(
        result,
        Err(Ok(WillError::TooManyWills.into())),
        "expected the capacity check to reject this call before PoisonToken::transfer could ever run"
    );
}
