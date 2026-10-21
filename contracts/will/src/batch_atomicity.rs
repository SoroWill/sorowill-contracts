//! All-or-nothing validation for batch entry points (#590).
//!
//! A Soroban invocation that panics discards every storage write it made, so a
//! batch is already atomic on-chain. These helpers make that explicit and
//! deterministic: every element is validated in a read-only pass *before* the
//! first write, so a bad element anywhere in the batch (including the middle)
//! is reported without any partial work being attempted.

use soroban_sdk::{Address, Env, Vec};

use crate::{assert_status, load_owned, WillError, WillStatus};

/// Validates every id in `ids` for `batch_check_in`: the will must exist
/// ([`WillError::WillNotFound`]), be owned by `owner` ([`WillError::NotOwner`])
/// and be `Active` ([`WillError::WillNotActive`]). Performs no writes.
pub fn prevalidate_check_in(env: &Env, ids: &Vec<u64>, owner: &Address) {
    for will_id in ids.iter() {
        let will = load_owned(env, will_id, owner);
        assert_status(env, &will, WillStatus::Active, WillError::WillNotActive);
    }
}
