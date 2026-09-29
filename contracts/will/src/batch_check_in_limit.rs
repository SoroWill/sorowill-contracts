//! Upper bound on the size of, and a no-duplicate rule for, the will ID list
//! accepted by `batch_check_in` (#414, #355).

use soroban_sdk::{panic_with_error, Env, Vec};

use crate::errors::WillError;

/// Maximum number of will IDs a single `batch_check_in` call may process.
pub const MAX_BATCH_CHECK_IN: u32 = 50;

/// Panics with [`WillError::BatchTooLarge`] if `len` exceeds [`MAX_BATCH_CHECK_IN`].
pub fn assert_within_limit(env: &Env, len: u32) {
    if len > MAX_BATCH_CHECK_IN {
        panic_with_error!(env, WillError::BatchTooLarge);
    }
}

/// Panics with [`WillError::DuplicateWillId`] if `ids` names the same will more
/// than once.
///
/// Without this check a caller can name one will fifty times: each occurrence
/// reloads the will, rewrites `last_checkin` to the same `now`, writes the same
/// record back and emits its own `check_in` event, so a single will produces up
/// to fifty indistinguishable events in one transaction while `batch_checkin`
/// reports a count of 50 (#355). Rejecting the input keeps the event stream at
/// one event per will and makes the summary count meaningful.
///
/// The scan is O(n²) over at most [`MAX_BATCH_CHECK_IN`] entries — cheaper than
/// the storage writes and events it protects, and it needs no extra on-chain
/// state.
pub fn assert_no_duplicates(env: &Env, ids: &Vec<u64>) {
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            if ids.get(i) == ids.get(j) {
                panic_with_error!(env, WillError::DuplicateWillId);
            }
        }
    }
}
