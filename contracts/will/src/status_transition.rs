//! Atomic lifecycle status check-and-update for existing wills (#663).
//!
//! A caller can only transition from a status that it checked in its loaded
//! snapshot if storage still has that same status. Soroban serializes
//! transactions, but a stale snapshot within an invocation must not overwrite
//! a newer lifecycle state.

use soroban_sdk::{panic_with_error, Env};

use crate::errors::WillError;
use crate::storage;
use crate::types::{Will, WillStatus};

/// Moves `will` from one of `allowed_from` to `to` after verifying storage
/// still matches the caller's snapshot.
///
/// Returns the status the will transitioned from. If either the snapshot is
/// not in `allowed_from` or storage no longer matches it, returns `err`
/// without writing. Storage errors are propagated unchanged.
pub(crate) fn try_transition(
    env: &Env,
    will: &mut Will,
    allowed_from: &[WillStatus],
    to: WillStatus,
    err: WillError,
) -> Result<WillStatus, WillError> {
    let from = will.status;
    if !allowed_from.contains(&from) {
        return Err(err);
    }

    let stored = storage::load_will(env, will.id)?;
    if stored.status != from {
        return Err(err);
    }

    will.status = to;
    storage::save_will_guardians_unchanged(env, will);
    Ok(from)
}

/// Panicking wrapper used by lifecycle entry points.
pub(crate) fn transition(
    env: &Env,
    will: &mut Will,
    allowed_from: &[WillStatus],
    to: WillStatus,
    err: WillError,
) -> WillStatus {
    match try_transition(env, will, allowed_from, to, err) {
        Ok(from) => from,
        Err(e) => panic_with_error!(env, e),
    }
}
