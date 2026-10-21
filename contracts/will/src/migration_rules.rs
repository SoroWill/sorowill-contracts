//! Structural invariants checked after every schema migration step (#596).
//!
//! `migration::upgrade` runs each `migrate_vN_to_vN+1` step and then calls
//! [`validate_step`], so a corrupted or partially migrated entry is rejected
//! with [`WillError::MigrationInvariantViolation`] instead of being persisted.

use soroban_sdk::{panic_with_error, Env};

use crate::errors::WillError;
use crate::types::{Allocation, Will, WillStatus};
use crate::{MAX_BENEFICIARIES, MAX_TOKENS};

/// Returns whether `will` satisfies the structural invariants that must hold
/// after migrating from `previous_version`.
pub fn is_valid_migration(will: &Will, previous_version: u32) -> bool {
    // The step must advance the schema by exactly one version.
    if previous_version.checked_add(1) != Some(will.schema_version) {
        return false;
    }
    // Status reachability: a Triggered will can only have come from an Active
    // will whose deadline passed, so it always carries a trigger time.
    if will.status == WillStatus::Triggered && will.trigger_time.is_none() {
        return false;
    }
    // A vote tally can never exceed the guardians that could have cast votes.
    if will.guardian_votes > will.guardians.len()
        || will.guardian_cancel_votes > will.guardians.len()
    {
        return false;
    }
    if will.beneficiaries.len() > MAX_BENEFICIARIES || will.balances.len() > MAX_TOKENS {
        return false;
    }
    let mut percentage_total: u64 = 0;
    for beneficiary in will.beneficiaries.iter() {
        if let Allocation::Percentage(bp) = beneficiary.allocation {
            percentage_total += bp as u64;
        }
    }
    percentage_total <= 10_000
}

/// Panics with [`WillError::MigrationInvariantViolation`] if `will`, the
/// result of migrating from `previous_version`, breaks an invariant.
pub fn validate_step(env: &Env, will: &Will, previous_version: u32) {
    if !is_valid_migration(will, previous_version) {
        panic_with_error!(env, WillError::MigrationInvariantViolation);
    }
}
