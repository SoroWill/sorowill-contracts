//! Storage schema versioning for persisted `Will` entries (issue #446).
//!
//! Soroban encodes a `#[contracttype]` struct as an XDR map keyed by field
//! name, and decoding is strict: a stored entry that is missing a field the
//! current `Will` declares (or carries one it no longer declares) fails to
//! convert. Adding a field to `Will` would therefore make every will written
//! by an earlier contract version unreadable.
//!
//! This module holds the two pieces that keep old entries loadable:
//!
//! 1. [`decode_will`] — every read of a `DataKey::Will` entry goes through
//!    here. It tries the current layout first and then each known legacy
//!    layout, lifting a legacy entry into the current `Will` shape while
//!    keeping its original `schema_version`.
//! 2. [`upgrade`] — applies the per-version migration steps in order until
//!    the will reaches [`CURRENT_SCHEMA_VERSION`]. Called by
//!    `WillContract::migrate_will`, which persists the result.
//!
//! See the "Storage schema versioning" section of CONTRIBUTING.md for the
//! step-by-step process of introducing a new version.

use soroban_sdk::{Env, TryFromVal, Val};

use crate::errors::WillError;
use crate::types::{Will, WillStatus};

/// Minimum legacy schema version supported by [`upgrade`] and
/// [`migrate_to_version`].
pub const MIN_SUPPORTED_SCHEMA_VERSION: u32 = 0;

/// Schema version written into every newly created or migrated will.
///
/// Bump this whenever the persisted `Will` layout changes, and add both a
/// legacy decode arm in [`decode_will`] and a migration step in [`upgrade`].
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Returns `true` if `version` is within the supported schema range
/// `[MIN_SUPPORTED_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION]`.
pub const fn is_supported_version(version: u32) -> bool {
    version >= MIN_SUPPORTED_SCHEMA_VERSION && version <= CURRENT_SCHEMA_VERSION
}

/// Returns `true` if `will` is at an older schema version and requires
/// migration to reach [`CURRENT_SCHEMA_VERSION`].
pub const fn needs_migration(will: &Will) -> bool {
    will.schema_version < CURRENT_SCHEMA_VERSION
}

/// Decodes a raw persisted will, accepting the current layout and every known
/// legacy layout.
///
/// A legacy entry is converted into the current `Will` shape with defaults
/// for fields it did not have; its `schema_version` is left at the version it
/// was written with so [`upgrade`] can run the remaining steps.
///
/// Returns [`WillError::UnsupportedSchemaVersion`] if no known layout matches.
pub fn decode_will(env: &Env, raw: &Val) -> Result<Will, WillError> {
    if let Ok(will) = Will::try_from_val(env, raw) {
        return Ok(will);
    }

    // Legacy layouts are tried here, newest first, once a v2 exists, e.g.:
    //
    //     if let Ok(old) = legacy::WillV1::try_from_val(env, raw) {
    //         return Ok(old.into_current(env));
    //     }

    Err(WillError::UnsupportedSchemaVersion)
}

/// Runs every migration step between `will.schema_version` and
/// `target_version`, in order.
///
/// Returns `Ok(will)` unchanged when `will.schema_version == target_version`.
/// Returns `Err(WillError::UnsupportedSchemaVersion)` if `target_version`
/// exceeds [`CURRENT_SCHEMA_VERSION`], if `will.schema_version > target_version`
/// (downgrades are not supported), or if an intermediate version has no
/// registered migration step.
pub fn migrate_to_version(
    env: &Env,
    mut will: Will,
    target_version: u32,
) -> Result<Will, WillError> {
    if target_version > CURRENT_SCHEMA_VERSION || will.schema_version > target_version {
        return Err(WillError::UnsupportedSchemaVersion);
    }

    while will.schema_version < target_version {
        let previous = will.schema_version;
        will = match will.schema_version {
            0 => {
                let upgraded = migrate_v0_to_v1(env, will);
                assert_migration_invariants(env, &upgraded);
                upgraded
            }
            // Every version below CURRENT_SCHEMA_VERSION must have a step.
            _ => return Err(WillError::UnsupportedSchemaVersion),
        };
        // Each step must advance the schema by exactly one version and leave
        // a structurally consistent will behind (#596).
        crate::migration_rules::validate_step(env, &will, previous);
    }
    Ok(will)
}

/// Runs every migration step between `will.schema_version` and
/// [`CURRENT_SCHEMA_VERSION`], in order. A will already at (or beyond) the
/// current version is returned unchanged.
pub fn upgrade(env: &Env, will: Will) -> Will {
    if will.schema_version >= CURRENT_SCHEMA_VERSION {
        return will;
    }
    match migrate_to_version(env, will, CURRENT_SCHEMA_VERSION) {
        Ok(upgraded) => upgraded,
        Err(err) => soroban_sdk::panic_with_error!(env, err),
    }
}

/// Invariant checks run after each migration step to guarantee that the upgraded
/// `Will` satisfies core consistency invariants.
pub fn assert_migration_invariants(env: &Env, will: &Will) {
    // 1. Primary token balance mirror agreement: balances[token] == balance
    if let Some(token_bal) = will.balances.get(will.token.clone()) {
        if token_bal != will.balance {
            soroban_sdk::panic_with_error!(env, WillError::DistributionMismatch);
        }
    }

    // 2. All balances must be non-negative
    for (_, bal) in will.balances.iter() {
        if bal < 0 {
            soroban_sdk::panic_with_error!(env, WillError::ZeroAmount);
        }
    }

    // 3. Check-in and grace periods must be valid (> 0)
    if will.checkin_period_days == 0 || will.grace_period_days == 0 {
        soroban_sdk::panic_with_error!(env, WillError::InvalidPeriod);
    }

    // 4. Beneficiaries must not be empty and must not exceed limit
    if will.beneficiaries.is_empty() || will.beneficiaries.len() > 10 {
        soroban_sdk::panic_with_error!(env, WillError::TooManyBeneficiaries);
    }

    // 5. Guardian threshold reachability when guardians are present
    if !will.guardians.is_empty() {
        if will.guardian_threshold == 0 || will.guardian_threshold > will.guardians.len() {
            soroban_sdk::panic_with_error!(env, WillError::InvalidGuardianThreshold);
        }
    }

    // 6. Beneficiary percentages must not exceed 10,000 basis points
    let mut total_pct: u32 = 0;
    for b in will.beneficiaries.iter() {
        if let crate::types::Allocation::Percentage(p) = b.allocation {
            total_pct = match total_pct.checked_add(p) {
                Some(s) => s,
                None => soroban_sdk::panic_with_error!(env, WillError::InvalidPercentages),
            };
        }
    }
    if total_pct > 10_000 {
        soroban_sdk::panic_with_error!(env, WillError::InvalidPercentages);
    }
}

/// v0 → v1: v0 wills predate the `schema_version` field being populated; the
/// layout is otherwise identical, so only the version number changes.
fn migrate_v0_to_v1(_env: &Env, mut will: Will) -> Will {
    will.schema_version = 1;
    will
}
