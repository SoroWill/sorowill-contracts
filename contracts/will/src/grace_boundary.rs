//! Single definition of the grace-period boundary (#588).
//!
//! The grace deadline second belongs to the owner. `emergency_checkin` and
//! `guardian_cancel_trigger` accept `now <= deadline`; the funds may only be
//! released once `now > deadline`. Both sides are expressed through this one
//! predicate so that at any timestamp exactly one of "owner can still cancel"
//! and "estate is releasable" holds: never both, never neither.

/// Returns `true` when the grace period has strictly elapsed, i.e. `now` is
/// after `deadline`. At `now == deadline` this is `false`.
pub fn grace_elapsed(now: u64, deadline: u64) -> bool {
    now > deadline
}

/// Returns `true` while the owner may still cancel the trigger. Always the
/// exact negation of [`grace_elapsed`].
pub fn owner_window_open(now: u64, deadline: u64) -> bool {
    !grace_elapsed(now, deadline)
}
