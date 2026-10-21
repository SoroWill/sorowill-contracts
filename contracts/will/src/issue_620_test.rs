#![cfg(test)]

//! Regression tests for issue #620: guardian release-vote weights must be
//! summed with an overflow check, so a total that does not fit in a `u32`
//! is rejected instead of wrapping around to a small value that could reach
//! the quorum threshold.

use crate::guardian_vote_freshness::add_guardian_weight;
use soroban_sdk::Env;

#[test]
fn weights_that_fit_are_summed() {
    let env = Env::default();
    assert_eq!(add_guardian_weight(&env, 1, 2), 3);
    assert_eq!(add_guardian_weight(&env, 0, u32::MAX), u32::MAX);
}

#[test]
#[should_panic]
fn a_total_that_overflows_u32_is_rejected() {
    let env = Env::default();
    // Wrapping would give 0 here; the check must refuse instead.
    add_guardian_weight(&env, u32::MAX, 1);
}
