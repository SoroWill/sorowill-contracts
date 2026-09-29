//! Freshness check for guardian release votes (#420).

use soroban_sdk::Env;

use crate::{storage, types::Will};

/// Sums the weight of guardian release votes that are still live (not older
/// than the will's grace period). The persisted `guardian_vote_weight` counter
/// can include votes that have since expired, so quorum must be judged on this
/// value to stop stale votes being replayed without the guardians re-voting.
pub(crate) fn live_guardian_vote_weight(env: &Env, will_id: u64, will: &Will, now: u64) -> u32 {
    let mut total: u32 = 0;
    for g in will.guardians.iter() {
        if storage::has_guardian_voted(env, will_id, &g.address, now, will.grace_period_days) {
            total += g.weight;
        }
    }
    total
}
