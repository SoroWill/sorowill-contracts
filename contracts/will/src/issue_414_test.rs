#![cfg(test)]

//! `batch_check_in` rejects more than `MAX_BATCH_CHECK_IN` will IDs (#414).

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

use crate::batch_check_in_limit::MAX_BATCH_CHECK_IN;
use crate::{WillContract, WillContractClient, WillError};

fn ids(env: &Env, n: u32) -> Vec<u64> {
    let mut v = Vec::new(env);
    for i in 0..n {
        v.push_back(i as u64 + 1);
    }
    v
}

#[test]
fn batch_check_in_over_limit_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));
    let owner = Address::generate(&env);

    let res = client.try_batch_check_in(&ids(&env, MAX_BATCH_CHECK_IN + 1), &owner);
    assert_eq!(res.unwrap_err().unwrap(), WillError::BatchTooLarge.into());
}
