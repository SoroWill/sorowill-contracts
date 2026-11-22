//! Keeps the legacy `Will::balance` mirror and the authoritative
//! `Will::balances` map in agreement (#592), and exposes the broader
//! state-consistency checks for a will (#805).
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
//!
//! ## State invariants (#805)
//!
//! The following constraints must hold for any persisted will:
//!
//! 1. **Balance mirror**: `balance == balances[token]` (0 when the primary
//!    token is absent from the map). See [`is_consistent`].
//! 2. **Non-negative balances**: every entry in `balances` is `>= 0`.
//! 3. **Beneficiary integrity**: the beneficiary list is non-empty and the
//!    per-beneficiary percentages sum to exactly 100.
//! 4. **Status/deadline coherence**: a will that has not yet been triggered
//!    must have a deadline strictly in the future relative to the current
//!    ledger time; a triggered/claimed will is not bound by that deadline.
//!
//! [`check_state`] validates all of the above and returns a
//! [`ConsistencyError`] describing the first violated invariant, so callers
//! can surface a contract error instead of silently proceeding on corrupt
//! state.

use soroban_sdk::{Address, Env, Vec};

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

/// Describes which state invariant was violated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsistencyError {
    /// `balance` disagrees with `balances[token]`.
    BalanceMismatch,
    /// A token balance is negative.
    NegativeBalance,
    /// The beneficiary list is empty.
    NoBeneficiaries,
    /// Beneficiary percentages do not sum to 100.
    InvalidAllocation,
    /// An untriggered will has a deadline that is not in the future.
    DeadlineInPast,
}

/// Validates every state invariant documented at the module level.
///
/// Returns `Ok(())` when the will is internally consistent, or the first
/// [`ConsistencyError`] encountered otherwise. Callers should map an `Err`
/// to a contract error rather than persisting or acting on the will.
pub fn check_state(env: &Env, will: &Will) -> Result<(), ConsistencyError> {
    // 1. Balance mirror.
    if !is_consistent(will) {
        return Err(ConsistencyError::BalanceMismatch);
    }

    // 2. Non-negative balances.
    let tokens: Vec<Address> = will.balances.keys();
    for token in tokens.iter() {
        if will.balances.get(token.clone()).unwrap_or(0) < 0 {
            return Err(ConsistencyError::NegativeBalance);
        }
    }

    // 3. Beneficiary integrity.
    if will.beneficiaries.is_empty() {
        return Err(ConsistencyError::NoBeneficiaries);
    }
    let mut total: u32 = 0;
    for beneficiary in will.beneficiaries.iter() {
        total = total.saturating_add(beneficiary.percentage);
    }
    if total != 100 {
        return Err(ConsistencyError::InvalidAllocation);
    }

    // 4. Status/deadline coherence: only untriggered wills are bound by the
    //    deadline. A triggered or claimed will has already passed it.
    if !will.triggered && will.deadline <= env.ledger().timestamp() {
        return Err(ConsistencyError::DeadlineInPast);
    }

    Ok(())
}
