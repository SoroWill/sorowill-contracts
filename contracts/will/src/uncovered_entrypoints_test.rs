#![cfg(test)]

//! Minimal coverage for entry points that `entrypoint_coverage_test` found
//! had zero references anywhere in the test suite: `update_periods` and
//! `reject_guardian_role`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, GuardianVoteReason, WillContract, WillContractClient, WillError};

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

#[test]
fn update_periods_changes_checkin_and_grace_periods() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    client.update_periods(&will_id, &owner, &Some(30), &Some(14));

    let will = client.get_will(&will_id);
    assert_eq!(will.checkin_period_days, 30);
    assert_eq!(will.grace_period_days, 14);
}

#[test]
fn reject_guardian_role_marks_the_guardian_as_rejected_and_blocks_voting() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    client.reject_guardian_role(&will_id, &guardian);

    // Clear the guardian-list cooldown so the rejection below is
    // specifically GuardianNotConsented, not GuardianCooldownActive.
    env.ledger().with_mut(|l| l.timestamp += 8 * 86_400);

    // A guardian who explicitly rejected the role has not consented, so
    // guardian_trigger must refuse their vote.
    assert_eq!(
        client.try_guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Other),
        Err(Ok(WillError::GuardianNotConsented.into())),
    );
}
