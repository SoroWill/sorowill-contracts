#![cfg(test)]

//! Regression test for issue #183: `merge_wills` resets `will_a.guardian_votes` to 0
//! but never resets `will_a.guardian_vote_weight`, causing the next `guardian_trigger`
//! to start from an incorrect (over-counted) weight baseline.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, TokenClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, TokenClient::new(&env, &token_address), token_address)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

/// Regression test asserting a merged will's `guardian_vote_weight` starts at 0
/// and accumulates correctly on the next `guardian_trigger` vote.
#[test]
fn merged_will_guardian_vote_weight_reset() {
    let (env, client, owner, _, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let guardian1 = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

    // Create two wills
    let will_id_a =
        client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env, guardian1.clone()], &1, &None, &0);
    let will_id_b =
        client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env, guardian1.clone()], &1, &None, &0);

    // Accept guardian role on both wills
    client.accept_guardian_role(&will_id_a, &guardian1);
    client.accept_guardian_role(&will_id_b, &guardian1);

    // Merge will_a into will_b
    client.merge_wills(&owner, &will_id_a, &will_id_b);

    // Get merged will and verify guardian_vote_weight is 0
    let merged_will = client.get_will(&will_id_a);
    assert_eq!(merged_will.guardian_vote_weight, 0, "guardian_vote_weight should be 0 after merge");
    assert_eq!(merged_will.guardian_votes, 0, "guardian_votes should also be 0 after merge");

    // guardian_trigger is an early-release mechanism that requires the will
    // to still be Active (it is not gated on trigger_will/the grace period),
    // so only the guardian-list cooldown needs to elapse before voting.
    advance(&env, 8);

    // Vote should work correctly from a fresh baseline of 0
    client.guardian_trigger(&will_id_a, &guardian1, &crate::GuardianVoteReason::Deceased);

    let voted_will = client.get_will(&will_id_a);
    assert!(voted_will.guardian_vote_weight > 0, "vote weight should accumulate correctly");
}
