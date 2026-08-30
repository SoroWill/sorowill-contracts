#![cfg(test)]

//! Regression test for issue #281: guardian membership is checked live
//! against `will.guardians` at vote time, which should correctly reject a
//! since-removed guardian — but no test explicitly removed a guardian via
//! `update_guardians` and then asserted that guardian's subsequent
//! `guardian_trigger` call fails with `WillError::NotGuardian`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;
const GUARDIAN_COOLDOWN_DAYS: u64 = 7;

#[test]
fn guardian_trigger_is_rejected_for_a_guardian_removed_via_update_guardians() {
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

    // Remove guardian_a from the guardian list, keeping only guardian_b.
    client.update_guardians(&will_id, &owner, &vec![&env, guardian_b]);

    // Clear the guardian-list cooldown before voting so the rejection below
    // is specifically `NotGuardian`, not `GuardianCooldownActive`.
    env.ledger()
        .with_mut(|l| l.timestamp += (GUARDIAN_COOLDOWN_DAYS + 1) * DAY);

    assert_eq!(
        client.try_guardian_trigger(
            &will_id,
            &guardian_a,
            &crate::GuardianVoteReason::Unreachable
        ),
        Err(Ok(WillError::NotGuardian.into()))
    );
}
