#![cfg(test)]

//! Regression test for issue #263: `GuardianVoteRecord` had no public
//! accessor, so an SDK/app had no way to query whether a specific guardian
//! has voted, when, or why, without replaying `guardian_voted` events
//! off-chain. `get_guardian_vote_status` now exposes that directly.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient,
};

const DAY: u64 = 86_400;

#[test]
fn get_guardian_vote_status_reflects_a_cast_vote() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &2,
        &None,
        &0,
    );

    // Before voting: no status for either guardian.
    assert_eq!(client.get_guardian_vote_status(&will_id, &guardian_a), None);
    assert_eq!(client.get_guardian_vote_status(&will_id, &guardian_b), None);

    // Clear the guardian-list cooldown, then guardian_a votes.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    let vote_time = env.ledger().timestamp();
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Incapacitated);

    let status = client
        .get_guardian_vote_status(&will_id, &guardian_a)
        .expect("guardian_a has an active vote");
    assert_eq!(status.timestamp, vote_time);
    assert_eq!(status.reason, GuardianVoteReason::Incapacitated);

    // The other guardian still has no vote recorded.
    assert_eq!(client.get_guardian_vote_status(&will_id, &guardian_b), None);
}

#[test]
fn get_guardian_vote_status_returns_none_once_the_vote_expires() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // grace_period_days = 7, so a vote expires 7 days after it's cast.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &2,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Unreachable);
    assert!(client.get_guardian_vote_status(&will_id, &guardian_a).is_some());

    // Past the 7-day grace-period expiry, the vote no longer counts as active.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    assert_eq!(client.get_guardian_vote_status(&will_id, &guardian_a), None);
}
