//! Confirmation-deadline precision (#593).
//!
//! The confirmation window is stored as a duration in seconds and turned into
//! an absolute `confirmation_deadline` at `create_will` time, while
//! `confirm_will` compares it with `env.ledger().timestamp()`. Ledger close
//! times are not continuous: the timestamp only moves in steps of one ledger
//! interval (about 5 seconds on Stellar), so an owner who submits at the last
//! moment can see a ledger timestamp a few seconds past a deadline they
//! computed from wall-clock time.
//!
//! To absorb that rounding, `confirm_will` accepts calls up to
//! [`CONFIRMATION_DEADLINE_BUFFER_SECONDS`] after `confirmation_deadline`.
//! The stored deadline itself is unchanged; the buffer is applied only when
//! comparing.
//!
//! # Minimum confirmation window
//!
//! A confirmation window should comfortably exceed
//! [`LEDGER_INTERVAL_SECONDS`]; windows of a few seconds are only usable
//! thanks to the buffer and should not be relied on. Practical windows are
//! hours to days.

/// Approximate time between ledger closes, in seconds.
pub const LEDGER_INTERVAL_SECONDS: u64 = 5;

/// Grace added to `confirmation_deadline` when checking expiry: one ledger
/// interval.
pub const CONFIRMATION_DEADLINE_BUFFER_SECONDS: u64 = LEDGER_INTERVAL_SECONDS;

/// Returns whether a confirmation attempt at ledger time `now` is too late
/// for `deadline`, allowing the buffer. Saturates instead of overflowing.
pub fn confirmation_expired(now: u64, deadline: u64) -> bool {
    now > deadline.saturating_add(CONFIRMATION_DEADLINE_BUFFER_SECONDS)
}
