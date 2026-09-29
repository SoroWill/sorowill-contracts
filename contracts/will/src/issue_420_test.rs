#![cfg(test)]

//! #420: stale guardian release votes cannot be replayed toward quorum.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillStatus,
};

const DAY: u64 = 86_400;

#[test]
fn expired_guardian_vote_does_not_count_toward_quorum() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
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
    client.accept_guardian_role(&will_id, &guardian_a);
    client.accept_guardian_role(&will_id, &guardian_b);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Incapacitated);
    // A's vote goes stale (grace period is 7 days), then B votes.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Incapacitated);

    assert_eq!(
        client.get_will(&will_id).status,
        WillStatus::Active,
        "a stale vote must not count toward the guardian quorum"
    );
}
