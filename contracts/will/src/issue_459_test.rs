#![cfg(test)]

//! Regression coverage for issue #459: a SEP-41 token transfer that fails
//! mid-`distribute` must not roll back (or block) any other beneficiary's
//! payout in the same call.
//!
//! Investigation showed the *literal* bug report ("earlier beneficiaries are
//! paid while later ones receive nothing") cannot happen: Soroban's
//! cross-contract call model is atomic across the whole invocation, so a
//! panic anywhere rolls back the *entire* transaction, including every
//! transfer that already succeeded earlier in the same `distribute` loop and
//! the state `distribute` itself committed (`will.status = Released`, etc).
//! That was confirmed empirically with a token that panics on transfer to a
//! chosen recipient: the call fails, and the beneficiary paid *before* the
//! poisoned one keeps a balance of zero, with the will left `Triggered`.
//!
//! The real, opposite problem this issue's title points at is a design gap
//! `distribute`'s prior all-or-nothing `transfer` calls left open: because
//! the contract's own state is already committed once `distribute` starts
//! its transfers, and the same poisoned transfer is reached on every retry,
//! *one* permanently-unreachable recipient (frozen, unauthorized, a paused
//! token, ...) blocked *every* beneficiary's inheritance forever, not just
//! their own. These tests cover the fix: `distribute` now uses
//! `try_transfer` per payout, records a failure via `retry_failed_payout`
//! instead of panicking, and lets every other transfer in the same call
//! proceed normally.

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

/// Minimal SEP-41 token whose `transfer` panics for one configured
/// recipient, simulating a token that (for reasons outside this contract's
/// control) refuses a specific transfer — a frozen or unauthorized holder, a
/// compliance hold, etc.
#[contracttype]
#[derive(Clone)]
enum DataKey {
    Balance(Address),
    Poison,
}

#[contract]
pub struct PoisonToken;

#[contractimpl]
impl PoisonToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = DataKey::Balance(to);
        let balance: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(balance + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    /// `create_will` probes every token with a read-only `decimals()` call
    /// before transferring, so this mock must answer it like a real SEP-41
    /// token to be usable as a will's locked asset.
    pub fn decimals(_env: Env) -> u32 {
        7
    }

    /// Every transfer *to* `addr` panics from this point on.
    pub fn set_poison(env: Env, addr: Address) {
        env.storage().instance().set(&DataKey::Poison, &addr);
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        if let Some(poison) = env.storage().instance().get::<_, Address>(&DataKey::Poison) {
            if poison == to {
                panic!("recipient cannot receive this token");
            }
        }
        let from_key = DataKey::Balance(from);
        let from_balance: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&from_key, &(from_balance - amount));
        let to_key = DataKey::Balance(to);
        let to_balance: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&to_key, &(to_balance + amount));
    }
}

fn setup<'a>() -> (
    Env,
    WillContractClient<'a>,
    PoisonTokenClient<'a>,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let token_id = env.register(PoisonToken, ());
    let token_client = PoisonTokenClient::new(&env, &token_id);
    token_client.mint(&owner, &1_000_000);

    let will_contract_id = env.register(WillContract, ());
    let will_client = WillContractClient::new(&env, &will_contract_id);

    let will_id = will_client.create_will(
        &owner,
        &vec![&env, (token_id.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                allocation: Allocation::Percentage(5_000),
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                allocation: Allocation::Percentage(5_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 91 * DAY);
    will_client.trigger_will(&will_id);
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 8 * DAY);

    (
        env,
        will_client,
        token_client,
        owner,
        beneficiary_a,
        beneficiary_b,
    )
}

#[test]
fn one_poisoned_beneficiary_does_not_block_the_other() {
    let (_env, will_client, token_client, _owner, beneficiary_a, beneficiary_b) = setup();

    // Poison beneficiary_b; beneficiary_a's transfer must still succeed in
    // the same `distribute` call.
    token_client.set_poison(&beneficiary_b);

    let will_id = 1;
    will_client.release_inheritance(&will_id, &None);

    assert_eq!(token_client.balance(&beneficiary_a), 500_000);
    assert_eq!(token_client.balance(&beneficiary_b), 0);

    // The will still transitions to Released -- the whole call does not
    // panic just because one payout failed.
    let will = will_client.get_will(&will_id);
    assert_eq!(will.status, crate::WillStatus::Released);
}

#[test]
fn failed_payout_is_delivered_later_via_retry() {
    let (env, will_client, token_client, _owner, _beneficiary_a, beneficiary_b) = setup();
    let token_id = token_client.address.clone();

    token_client.set_poison(&beneficiary_b);
    let will_id = 1;
    will_client.release_inheritance(&will_id, &None);
    assert_eq!(token_client.balance(&beneficiary_b), 0);

    // Un-poison (by pointing the poison at an address nobody uses) and
    // retry: the recorded amount is delivered.
    let harmless = Address::generate(&env);
    token_client.set_poison(&harmless);

    will_client.retry_failed_payout(&will_id, &token_id, &beneficiary_b);

    assert_eq!(token_client.balance(&beneficiary_b), 500_000);
}

#[test]
fn retry_without_a_recorded_failure_is_rejected() {
    let (_env, will_client, token_client, _owner, beneficiary_a, _beneficiary_b) = setup();
    let token_id = token_client.address.clone();

    // Nothing failed for beneficiary_a -- there is nothing to retry.
    let res = will_client.try_retry_failed_payout(&1, &token_id, &beneficiary_a);
    assert_eq!(res.unwrap_err().unwrap(), WillError::NoFailedPayout.into());
}

#[test]
fn retry_is_independent_of_the_will_being_archived() {
    let (env, will_client, token_client, _owner, _beneficiary_a, beneficiary_b) = setup();
    let token_id = token_client.address.clone();

    token_client.set_poison(&beneficiary_b);
    let will_id = 1;
    will_client.release_inheritance(&will_id, &None);

    // Archive the (Released) will before retrying.
    will_client.archive_will(&will_id);

    let harmless = Address::generate(&env);
    token_client.set_poison(&harmless);
    will_client.retry_failed_payout(&will_id, &token_id, &beneficiary_b);

    assert_eq!(token_client.balance(&beneficiary_b), 500_000);
}
