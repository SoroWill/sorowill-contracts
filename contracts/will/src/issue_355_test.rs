#![cfg(test)]

//! `batch_check_in` bounds and de-duplicates the will ID list it is given.
//!
//! The per-call cap is [`MAX_BATCH_CHECK_IN`] (#414); duplicate rejection is
//! #355, which stops a single repeated ID from being processed once per
//! occurrence and emitting a `check_in` event for every repeat.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Symbol, TryFromVal as _,
};

use crate::batch_check_in_limit::MAX_BATCH_CHECK_IN;
use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

/// Creates `n` distinct `Active` wills owned by one address and returns
/// `(env, contract, owner, will_ids)`.
fn setup(n: u32) -> (Env, Address, Address, soroban_sdk::Vec<u64>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let mut ids = soroban_sdk::Vec::new(&env);
    for _ in 0..n {
        let will_id = client.create_will(
            &owner,
            &vec![&env, (token.clone(), 1_000_000_i128)],
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
            &0,
            &None,
            &0,
        );
        ids.push_back(will_id);
    }

    (env, contract_id, owner, ids)
}

fn ids(env: &Env, n: u32) -> soroban_sdk::Vec<u64> {
    let mut v = soroban_sdk::Vec::new(env);
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

/// A list of distinct ids is accepted and produces exactly one `check_in`
/// event per will, so the `batch_checkin` count and the event stream agree.
///
/// The size used here is bounded by `MAX_WILLS_PER_INDEX`, which the test
/// configuration lowers to 10, rather than by `MAX_BATCH_CHECK_IN`; the
/// per-call cap itself is exercised by `issue_414_test`.
#[test]
fn distinct_ids_each_produce_exactly_one_check_in_event() {
    // Must stay <= storage::MAX_WILLS_PER_INDEX (10 under `cfg(test)`), since
    // every will created here shares one owner index.
    const N: u32 = 10;
    let (env, contract_id, owner, will_ids) = setup(N);
    let client = WillContractClient::new(&env, &contract_id);

    client.batch_check_in(&will_ids, &owner);

    // The topic is `(symbol_short!("checkin"), will_id)`, so match on the
    // leading symbol to cover every will in the batch.
    let check_ins = env
        .events()
        .all()
        .iter()
        .filter(|e| {
            Symbol::try_from_val(&env, &e.1.get(0).unwrap()) == Ok(symbol_short!("checkin"))
        })
        .count();
    assert_eq!(
        check_ins, N as usize,
        "one check_in event per distinct will"
    );
}

#[test]
fn repeated_id_is_rejected() {
    let (env, contract_id, owner, will_ids) = setup(2);
    let client = WillContractClient::new(&env, &contract_id);
    let repeated = will_ids.get(0).unwrap();

    let mut dupes = soroban_sdk::Vec::new(&env);
    dupes.push_back(will_ids.get(0).unwrap());
    dupes.push_back(will_ids.get(1).unwrap());
    dupes.push_back(repeated);

    let res = client.try_batch_check_in(&dupes, &owner);
    assert_eq!(res.unwrap_err().unwrap(), WillError::DuplicateWillId.into());
}

/// A rejection must leave the wills untouched — the duplicate check runs before
/// any storage write, so a partially-applied batch cannot happen.
#[test]
fn duplicate_rejection_writes_nothing() {
    let (env, contract_id, owner, will_ids) = setup(2);
    let client = WillContractClient::new(&env, &contract_id);
    let before = client.get_will(&will_ids.get(0).unwrap());

    env.ledger().with_mut(|l| l.timestamp += 3 * DAY);
    let mut dupes = soroban_sdk::Vec::new(&env);
    dupes.push_back(will_ids.get(0).unwrap());
    dupes.push_back(will_ids.get(0).unwrap());
    assert!(client.try_batch_check_in(&dupes, &owner).is_err());

    assert_eq!(
        client.get_will(&will_ids.get(0).unwrap()).last_checkin,
        before.last_checkin
    );
}

#[test]
fn single_id_repeated_fifty_times_is_rejected() {
    let (env, contract_id, owner, will_ids) = setup(1);
    let client = WillContractClient::new(&env, &contract_id);
    let id = will_ids.get(0).unwrap();

    let mut dupes = soroban_sdk::Vec::new(&env);
    for _ in 0..MAX_BATCH_CHECK_IN {
        dupes.push_back(id);
    }

    let res = client.try_batch_check_in(&dupes, &owner);
    assert_eq!(res.unwrap_err().unwrap(), WillError::DuplicateWillId.into());
}
