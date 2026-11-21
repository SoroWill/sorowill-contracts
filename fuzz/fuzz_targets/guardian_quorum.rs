//! Coverage-guided fuzzing of guardian quorum voting and cancellation scenarios.
//!
//! Exercises `accept_guardian_role`, `reject_guardian_role`, `guardian_trigger`,
//! `guardian_cancel_trigger`, `trigger_will`, `emergency_checkin`, and
//! `update_guardians_weighted` across arbitrary guardian lists, weights,
//! thresholds, and voting sequences.
//!
//! Run with:
//!
//! ```sh
//! cargo +nightly fuzz run guardian_quorum
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use will::fuzz_harness::{run_guardian_quorum, GuardianQuorumInput};

fuzz_target!(|input: GuardianQuorumInput| {
    run_guardian_quorum(&input);
});
