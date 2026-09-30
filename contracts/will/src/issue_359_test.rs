#![cfg(test)]

//! Regression coverage for issue #359: `get_guardian_vote_status` must not
//! panic when the ledger time is earlier than a stored vote timestamp.
//!
//! The query computed `now - record.timestamp` with a raw `u64` subtraction,
//! so a rewound/incorrect ledger clock underflowed and trapped the read-only
//! call. It now shares `storage::vote_is_live` with `has_guardian_voted` and
//! `has_guardian_cancel_voted`, so the expiry rule has a single implementation.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::storage;
use crate::{Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

/// Creates a will with two guardians and a threshold of two, then casts a
/// single trigger vote for `guardian_a` (so the will stays `Active`).
fn will_with_one_guardian_vote<'a>(
    env: &Env,
    client: &WillContractClient<'a>,
    owner: &Address,
    token_address: &Address,
    guardian_a: &Address,
    guardian_b: &Address,
) -> u64 {
    let will_id = client.create_will(
        owner,
        &vec![env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: Address::generate(env),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env, guardian_a.clone(), guardian_b.clone()],
        &2,
        &None,
        &0,
    );

    client.accept_guardian_role(&will_id, guardian_a);
    client.accept_guardian_role(&will_id, guardian_b);

    // The guardian-list cooldown has to elapse before a trigger vote counts.
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + 8 * DAY);
    client.guardian_trigger(&will_id, guardian_a, &GuardianVoteReason::Other);

    will_id
}

#[test]
fn guardian_vote_status_does_not_underflow_when_now_precedes_the_vote() {
    let (env, client, owner, token_address) = setup();
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);
    let will_id = will_with_one_guardian_vote(
        &env,
        &client,
        &owner,
        &token_address,
        &guardian_a,
        &guardian_b,
    );

    let vote_time = env.ledger().timestamp();
    let recorded = client
        .get_guardian_vote_status(&will_id, &guardian_a)
        .expect("the freshly cast vote must be reported");
    assert_eq!(recorded.timestamp, vote_time);

    // Rewind the ledger to before the vote was cast. The query must return the
    // stored record instead of trapping on `now - timestamp`.
    let stale_now = vote_time - DAY;
    env.ledger().set_timestamp(stale_now);

    let reported = client.get_guardian_vote_status(&will_id, &guardian_a);
    assert_eq!(
        reported,
        Some(recorded),
        "a vote timestamped after `now` must be reported as-is, not panic"
    );
}

#[test]
fn vote_expiry_rule_is_shared_with_the_quorum_helpers() {
    let (env, client, owner, token_address) = setup();
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);
    let will_id = will_with_one_guardian_vote(
        &env,
        &client,
        &owner,
        &token_address,
        &guardian_a,
        &guardian_b,
    );

    let vote_time = env.ledger().timestamp();
    let contract_id = client.address.clone();
    let grace_period_days = client.get_will(&will_id).grace_period_days;

    // `has_guardian_voted` / `has_guardian_cancel_voted` must keep reporting a
    // future-dated record as "not voted" — a quorum may not be reachable from a
    // vote that has not been cast yet — while the read-only query above reports
    // the record itself. Both paths now share `vote_is_live`.
    env.as_contract(&contract_id, || {
        assert!(!storage::has_guardian_voted(
            &env,
            will_id,
            &guardian_a,
            vote_time - DAY,
            grace_period_days
        ));
        assert!(!storage::has_guardian_cancel_voted(
            &env,
            will_id,
            &guardian_a,
            vote_time - DAY,
            grace_period_days
        ));

        // Sanity: a normal, non-skewed clock still counts the vote.
        assert!(storage::has_guardian_voted(
            &env,
            will_id,
            &guardian_a,
            vote_time + 60,
            grace_period_days
        ));
    });
}
