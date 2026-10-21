//! Per-will lock held for the duration of `release_inheritance` (#586).
//!
//! A release moves every token balance out of the contract through external
//! token calls. Soroban invocations are atomic, but a token contract invoked
//! mid-release can call back into the will contract; an explicit lock makes
//! that re-entrant release fail with [`WillError::ReleaseInProgress`] instead
//! of relying only on the status re-check.

use soroban_sdk::{panic_with_error, symbol_short, Env, Symbol};

use crate::errors::WillError;

const LOCK_KEY: Symbol = symbol_short!("rel_lock");

/// Returns `true` while a release of `will_id` is in progress.
pub(crate) fn is_held(env: &Env, will_id: u64) -> bool {
    env.storage().temporary().has(&(LOCK_KEY, will_id))
}

/// Takes the lock for `will_id`, panicking with
/// [`WillError::ReleaseInProgress`] if it is already held.
pub(crate) fn acquire(env: &Env, will_id: u64) {
    if is_held(env, will_id) {
        panic_with_error!(env, WillError::ReleaseInProgress);
    }
    env.storage().temporary().set(&(LOCK_KEY, will_id), &true);
}

/// Drops the lock for `will_id`.
pub(crate) fn release(env: &Env, will_id: u64) {
    env.storage().temporary().remove(&(LOCK_KEY, will_id));
}
