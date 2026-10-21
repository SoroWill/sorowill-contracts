#![cfg(test)]

//! Regression tests for issue #586: concurrent `release_inheritance` calls
//! are rejected while the per-will release lock is held.

use soroban_sdk::Env;

use crate::{release_lock, WillContract, WillContractClient, WillError};

#[test]
fn release_inheritance_rejected_while_lock_held() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    env.as_contract(&contract_id, || release_lock::acquire(&env, 999));
    let res = client.try_release_inheritance(&999, &None);
    assert_eq!(res, Err(Ok(WillError::ReleaseInProgress)));
}

#[test]
fn release_lock_is_per_will_and_releasable() {
    let env = Env::default();
    let contract_id = env.register(WillContract, ());
    env.as_contract(&contract_id, || {
        release_lock::acquire(&env, 1);
        assert!(release_lock::is_held(&env, 1));
        assert!(!release_lock::is_held(&env, 2));
        release_lock::release(&env, 1);
        assert!(!release_lock::is_held(&env, 1));
    });
}
