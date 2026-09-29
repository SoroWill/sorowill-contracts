#![cfg(test)]

//! Regression test: `has_guardian_voted` / `has_guardian_cancel_voted` must not
//! underflow when `now` is earlier than the recorded vote timestamp.
//!
//! Previously both helpers computed `now - record.timestamp` with a raw
//! unsigned subtraction, so a stale/incorrect caller-supplied ledger time
//! (or any host/test clock inconsistency) panicked and aborted the whole
//! transaction. They now use a checked subtraction and simply report
//! "not voted" instead.

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::storage::{
    has_guardian_cancel_voted, has_guardian_voted, set_guardian_cancel_voted, set_guardian_voted,
};
use crate::types::GuardianVoteReason;
use crate::WillContract;

#[test]
fn guardian_vote_checks_do_not_underflow_when_now_is_before_vote_timestamp() {
    let env = Env::default();
    let contract_id = env.register(WillContract, ());

    env.as_contract(&contract_id, || {
        let guardian = Address::generate(&env);
        let will_id: u64 = 1;

        // Vote recorded "in the future" relative to the `now` we will pass in.
        let vote_timestamp: u64 = 2_000_000_000;
        let stale_now: u64 = 1_000_000_000;
        let expiry_days: u64 = 30;

        set_guardian_voted(
            &env,
            will_id,
            &guardian,
            vote_timestamp,
            GuardianVoteReason::Other,
        );
        set_guardian_cancel_voted(&env, will_id, &guardian, vote_timestamp);

        // Must return a value rather than panicking on unsigned underflow.
        assert!(!has_guardian_voted(
            &env,
            will_id,
            &guardian,
            stale_now,
            expiry_days
        ));
        assert!(!has_guardian_cancel_voted(
            &env,
            will_id,
            &guardian,
            stale_now,
            expiry_days
        ));

        // Sanity: a normal (non-skewed) call still reports an active vote.
        assert!(has_guardian_voted(
            &env,
            will_id,
            &guardian,
            vote_timestamp + 60,
            expiry_days
        ));
        assert!(has_guardian_cancel_voted(
            &env,
            will_id,
            &guardian,
            vote_timestamp + 60,
            expiry_days
        ));
    });
}
