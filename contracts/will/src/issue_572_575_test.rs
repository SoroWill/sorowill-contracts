#![cfg(test)]

//! Regression coverage for issues #572-#575:
//! Test: remove guardian with vote, add them back, verify vote doesn't count.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillStatus,
};

const DAY: u64 = 86_400;

#[test]
fn test_remove_guardian_with_vote_add_back_verify_vote_does_not_count() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone(), g3.clone()],
        &2, // threshold = 2
        &None,
        &0,
    );

    // Accept guardian roles
    client.accept_guardian_role(&will_id, &g1);
    client.accept_guardian_role(&will_id, &g2);
    client.accept_guardian_role(&will_id, &g3);

    // Advance past guardian list cooldown (7 days) and check-in period (90 days)
    env.ledger().with_mut(|l| l.timestamp += 95 * DAY);

    // Guardian 1 casts a trigger vote
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Unreachable);
    assert!(client.get_guardian_vote_status(&will_id, &g1).is_some());

    let will_before = client.get_will(&will_id);
    assert_eq!(will_before.guardian_votes, 1);
    assert_eq!(will_before.status, WillStatus::Active); // threshold is 2, so not triggered yet

    // Owner updates guardian list removing g1 (leaving g2 and g3)
    client.update_guardians(&will_id, &owner, &vec![&env, g2.clone(), g3.clone()]);

    let will_after_remove = client.get_will(&will_id);
    assert_eq!(will_after_remove.guardian_votes, 0);
    assert!(client.get_guardian_vote_status(&will_id, &g1).is_none());

    // Owner adds g1 back to the guardian list
    client.update_guardians(&will_id, &owner, &vec![&env, g1.clone(), g2.clone(), g3.clone()]);

    // Verify g1's vote does not count anymore
    assert!(
        client.get_guardian_vote_status(&will_id, &g1).is_none(),
        "Removed and re-added guardian must not have an active vote"
    );
    let will_after_readd = client.get_will(&will_id);
    assert_eq!(
        will_after_readd.guardian_votes, 0,
        "Vote accumulator must remain zero after re-adding guardian"
    );
}
