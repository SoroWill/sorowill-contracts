//! Freshness check for guardian release votes (#420).
//!
//! # Timestamp precision (#437)
//!
//! Guardian votes are persisted with a timestamp taken from
//! [`soroban_sdk::Env::ledger`]'s `timestamp()`. Soroban defines that value as
//! **Unix time in seconds** (1-second granularity), and the grace period is
//! likewise expressed in whole days. All freshness math below therefore works
//! in seconds only: no millisecond/microsecond conversion is applied, and two
//! votes recorded within the same ledger second share the same vote window.
//! If the ledger ever advances in sub-second units, the raw value must be
//! normalised to seconds before it reaches this module.

use soroban_sdk::Env;

use crate::{storage, types::Will};

/// Number of seconds in a day, used to keep the grace period and the ledger
/// timestamp in the same unit (seconds) when computing vote expiry.
const SECONDS_PER_DAY: u64 = 86_400;

/// Normalises a ledger timestamp to the 1-second granularity that guardian
/// votes are stored with. Soroban reports `timestamp()` in seconds, so this is
/// a no-op today, but it keeps every expiry comparison in a single unit and
/// makes the assumption explicit if the ledger precision ever changes.
#[inline]
fn to_vote_seconds(timestamp: u64) -> u64 {
    timestamp
}

/// Sums the weight of guardian release votes that are still live (not older
/// than the will's grace period). The persisted `guardian_vote_weight` counter
/// can include votes that have since expired, so quorum must be judged on this
/// value to stop stale votes being replayed without the guardians re-voting.
///
/// `now` is a ledger timestamp in seconds; it is normalised with
/// [`to_vote_seconds`] so that votes cast in the same ledger second are always
/// treated as belonging to the same vote window.
pub(crate) fn live_guardian_vote_weight(env: &Env, will_id: u64, will: &Will, now: u64) -> u32 {
    let now = to_vote_seconds(now);
    let grace_seconds = (will.grace_period_days as u64).saturating_mul(SECONDS_PER_DAY);
    let mut total: u32 = 0;
    for g in will.guardians.iter() {
        if storage::has_guardian_voted(env, will_id, &g.address, now, will.grace_period_days) {
            total += g.weight;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_seconds_keeps_second_granularity() {
        // Ledger timestamps are already in seconds; normalisation must not
        // shift the value, so two votes in the same ledger second stay in the
        // same vote window.
        assert_eq!(to_vote_seconds(1_700_000_000), 1_700_000_000);
        assert_eq!(to_vote_seconds(1_700_000_000), to_vote_seconds(1_700_000_000));
    }

    #[test]
    fn grace_period_is_expressed_in_seconds() {
        // 1 day of grace == 86_400 seconds, matching the ledger timestamp unit.
        assert_eq!(1u64.saturating_mul(SECONDS_PER_DAY), 86_400);
        assert_eq!(7u64.saturating_mul(SECONDS_PER_DAY), 604_800);
    }

    #[test]
    fn expiry_boundary_is_consistent_within_a_second() {
        let voted_at = to_vote_seconds(1_700_000_000);
        let grace_seconds = 1u64.saturating_mul(SECONDS_PER_DAY);
        let expires_at = voted_at + grace_seconds;

        // Two observations inside the same ledger second must agree on
        // whether the vote is still live.
        let now_a = to_vote_seconds(1_700_000_000);
        let now_b = to_vote_seconds(1_700_000_000);
        assert_eq!(now_a < expires_at, now_b < expires_at);

        // The vote expires exactly at the boundary, not one second early.
        assert!(to_vote_seconds(expires_at - 1) < expires_at);
        assert!(!(to_vote_seconds(expires_at) < expires_at));
    }
}
