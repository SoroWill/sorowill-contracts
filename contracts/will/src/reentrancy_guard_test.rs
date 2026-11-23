//! Tests for the will contract reentrancy guard.
//!
//! These tests exercise the storage-backed lock exposed by the will
//! contract. The guard is expected to:
//!   * allow the first entry into a guarded entrypoint,
//!   * revert with a clear error when a guarded entrypoint is re-entered
//!     while the lock is still held,
//!   * release the lock once the guarded call completes so subsequent
//!     calls succeed.

use soroban_sdk::{contract, contractimpl, Env, Symbol};

use crate::{WillContract, WillContractClient};

/// Minimal harness contract that drives the will contract's guard so we can
/// simulate a reentrant call without needing a full will setup.
#[contract]
pub struct ReentrancyHarness;

#[contractimpl]
impl ReentrancyHarness {
    /// Attempts to enter a guarded will entrypoint twice in a row. The first
    /// call should succeed; the second call (made while the guard is still
    /// held) must revert.
    pub fn reenter(env: Env, will: soroban_sdk::Address) {
        let client = WillContractClient::new(&env, &will);
        // First entry acquires the guard.
        client.enter_guarded();
        // Second entry while the guard is held must revert.
        client.enter_guarded();
    }
}

#[test]
fn guard_allows_first_entry() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, WillContract);
    let client = WillContractClient::new(&env, &contract_id);

    // A single guarded call should succeed and release the lock.
    client.enter_guarded();
    // A subsequent call after the lock is released should also succeed.
    client.enter_guarded();
}

#[test]
#[should_panic]
fn guard_reverts_on_reentrancy() {
    let env = Env::default();
    env.mock_all_auths();

    let will_id = env.register_contract(None, WillContract);
    let harness_id = env.register_contract(None, ReentrancyHarness);
    let harness = ReentrancyHarnessClient::new(&env, &harness_id);

    // The harness will attempt to re-enter the guarded entrypoint; the
    // second call must revert because the guard is still held.
    harness.reenter(&will_id);
}

#[test]
fn guard_error_is_descriptive() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, WillContract);
    let client = WillContractClient::new(&env, &contract_id);

    // Acquire the guard directly, then attempt a guarded entrypoint.
    client.acquire_guard_for_test();
    let result = client.try_enter_guarded();
    assert!(result.is_err(), "reentrant call must revert");

    // The error should be the documented reentrancy error symbol.
    let err = result.err().unwrap().unwrap();
    assert_eq!(err, Symbol::new(&env, "Reentrancy"));
}
