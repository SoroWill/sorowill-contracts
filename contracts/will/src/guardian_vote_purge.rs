//! Unconditional purge of guardian vote records when the guardian list is
//! replaced (#598).
//!
//! `storage::reset_guardian_votes` skips its removals when the live tally is
//! zero, leaving expired records behind as "inert". They are only inert while
//! the vote-expiry window stays the same: a later `update_will_settings` that
//! widens it, or a guardian who is removed and then re-added, could revive an
//! old record. Replacing the list therefore removes every record outright.

use soroban_sdk::{Env, Vec};

use crate::storage::DataKey;
use crate::types::{Guardian, Will};

/// Removes the stored `GuardianVote` record of every guardian currently on
/// `will`, whether or not it is still live. Call before the list is replaced.
pub fn purge_guardian_vote_records(env: &Env, will: &Will) {
    for guardian in will.guardians.iter() {
        let key = DataKey::GuardianVote(will.id, guardian.address.clone());
        env.storage().persistent().remove(&key);
    }
}

/// Removes the release-vote and cancel-vote records of every guardian that is
/// present in `old_guardians` but absent from `new_guardians`.
///
/// This is required in addition to `reset_guardian_votes`: that helper skips
/// its removals when the live tally is zero, so a removed guardian's record
/// would otherwise survive in storage. If the guardian is later re-added, or
/// the vote-expiry window is widened by `update_will_settings`, the stale
/// record could be revived and counted toward the quorum (#658). Purging here
/// guarantees a removed guardian's votes can never inflate the tally.
pub fn purge_removed_guardian_votes(
    env: &Env,
    will_id: u64,
    old_guardians: &Vec<Guardian>,
    new_guardians: &Vec<Guardian>,
) {
    for old_guardian in old_guardians.iter() {
        let still_present = new_guardians
            .iter()
            .any(|g| g.address == old_guardian.address);
        if still_present {
            continue;
        }
        let key = DataKey::GuardianVote(will_id, old_guardian.address.clone());
        env.storage().persistent().remove(&key);
    }
}