//! Keeper bounty rounding (#595).
//!
//! # Direction
//!
//! The bounty is `floor(balance * keeper_bounty_bps / 10_000)`, computed in
//! integer arithmetic through [`crate::proportional_share`] (no floating
//! point, no overflow of the intermediate product). Rounding is therefore
//! always **down**, in favour of the beneficiaries: the keeper never receives
//! more than the configured fraction.
//!
//! # Dust
//!
//! The fractional remainder (strictly less than one token unit) is never
//! lost or sent to the keeper. It stays in the amount available to the
//! beneficiaries; the last percentage beneficiary absorbs the integer-division
//! remainder, and a FixedAmount-only will refunds the leftover to the owner
//! (#383). When `balance * bps < 10_000` the bounty is 0 and the whole
//! balance is distributed.

/// Returns the keeper bounty for `balance` at `bps` basis points, rounded
/// down.
pub fn keeper_bounty(balance: i128, bps: u32) -> i128 {
    crate::proportional_share(balance, bps)
}
