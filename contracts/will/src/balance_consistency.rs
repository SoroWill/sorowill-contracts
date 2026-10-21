//! Keeps the legacy `Will::balance` mirror and the authoritative
//! `Will::balances` map in agreement (#592).
//!
//! A will tracks its primary token twice: `balance` (legacy, single-token
//! helpers such as `merge_wills`, `split_will` and `reveal_and_claim` still
//! read it) and `balances[token]` (the multi-token source of truth that
//! `distribute` pays from). Soroban executes each invocation atomically and
//! serially, so a `top_up` can never interleave with an `emergency_checkin`
//! on the same will; the real risk is a code path that updates one
//! representation and persists the will without the other. Every writer that
//! touches `balances` therefore goes through [`record_balance`], and every
//! writer that persists a will after a state transition calls
//! [`sync_primary_balance`], so the invariant
//! `balance == balances[primary_token]` holds whenever a will is saved.

use soroban_sdk::Address;

use crate::types::Will;

/// Re-derives `will.balance` from `will.balances[will.token]` (0 when the
/// primary token is absent from the map). The map is authoritative.
pub fn sync_primary_balance(will: &mut Will) {
    will.balance = will.balances.get(will.token.clone()).unwrap_or(0);
}

/// Sets `balances[token] = new_balance` and refreshes the legacy mirror in
/// the same step, so the two representations cannot be left disagreeing.
pub fn record_balance(will: &mut Will, token: &Address, new_balance: i128) {
    will.balances.set(token.clone(), new_balance);
    sync_primary_balance(will);
}

/// Returns whether the mirror currently matches the map.
pub fn is_consistent(will: &Will) -> bool {
    will.balance == will.balances.get(will.token.clone()).unwrap_or(0)
}
