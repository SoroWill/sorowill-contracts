//! Unconditional purge of guardian vote records when the guardian list is
//! replaced (#598).
//!
//! `storage::reset_guardian_votes` skips its removals when the live tally is
//! zero, leaving expired records behind as "inert". They are only inert while
//! the vote-expiry window stays the same: a later `update_will_settings` that
//! widens it, or a guardian who is removed and then re-added, could revive an
//! old record. Replacing the list therefore removes every record outright.

use soroban_sdk::Env;

use crate::storage::DataKey;
use crate::types::Will;

/// Removes the stored `GuardianVote` record of every guardian currently on
/// `will`, whether or not it is still live. Call before the list is replaced.
pub fn purge_guardian_vote_records(env: &Env, will: &Will) {
    for guardian in will.guardians.iter() {
        let key = DataKey::GuardianVote(will.id, guardian.address.clone());
        env.storage().persistent().remove(&key);
    }
}
