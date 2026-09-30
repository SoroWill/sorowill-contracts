#![cfg(test)]

//! Regression tests for issue #363: `create_will` must not persist a stray
//! `guardian_threshold` when the will has no guardians.
//!
//! `create_will`'s rustdoc has always said the argument is ignored for a
//! guardian-less will, and the range check is indeed skipped for an empty
//! `guardians` list -- but the raw argument was stored on the will anyway. The
//! stored value then leaks into a completely different call: `update_guardians`
//! (and the guardian branch of `update_will_settings`) reject a new non-empty
//! guardian list shorter than the will's stored threshold, so a caller who
//! passed `5` or `9` to `create_will` with no guardians was met with
//! `InvalidGuardianThreshold` the moment they added two or three guardians --
//! a failure whose cause is invisible from the call being made.
//!
//! The fix normalises the stored threshold to `0` when no guardians are given,
//! so the very next `update_guardians` call succeeds as the owner expects.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, GuardianSpec, WillContract, WillContractClient, WillError};

/// Creates a will with **no** guardians and the given (stray) threshold.
/// Returns `(env, contract_address, owner, will_id)`.
fn setup_without_guardians(threshold: u32) -> (Env, Address, Address, u64) {
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
        &vec![&env], // no guardians
        &threshold,
        &None,
        &0,
    );

    (env, contract_id, owner, will_id)
}

/// A guardian-less will must store a threshold of 0 no matter what the caller
/// passed, so the value can never describe an unreachable quorum (#363).
#[test]
fn create_will_normalizes_the_threshold_when_there_are_no_guardians() {
    for stray in [1u32, 5, 9, 30, u32::MAX] {
        let (env, contract_id, _owner, will_id) = setup_without_guardians(stray);
        let client = WillContractClient::new(&env, &contract_id);

        let will = client.get_will(&will_id);
        assert_eq!(will.guardians.len(), 0);
        assert_eq!(
            will.guardian_threshold, 0,
            "a guardian-less will must normalise the stray threshold {stray} to 0, \
             not persist it"
        );
    }
}

/// The user-visible consequence of #363: creating a will with no guardians and
/// threshold 9, then adding two guardians, must succeed. Before the fix the
/// stored 9 exceeded the new list length of 2 and `update_guardians` rejected
/// the call with `InvalidGuardianThreshold`.
#[test]
fn update_guardians_succeeds_after_a_guardian_less_will_with_threshold_nine() {
    let (env, contract_id, owner, will_id) = setup_without_guardians(9);
    let client = WillContractClient::new(&env, &contract_id);

    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);

    client.update_guardians(&will_id, &owner, &vec![&env, g1, g2]);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardians.len(), 2);
    assert_eq!(
        will.guardian_threshold, 0,
        "update_guardians does not change the threshold; the normalised 0 is kept"
    );
}

/// Three guardians -- still below the stray 9 -- must work too.
#[test]
fn update_guardians_accepts_three_guardians_after_a_stray_threshold() {
    let (env, contract_id, owner, will_id) = setup_without_guardians(9);
    let client = WillContractClient::new(&env, &contract_id);

    client.update_guardians(
        &will_id,
        &owner,
        &vec![
            &env,
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ],
    );

    assert_eq!(client.get_will(&will_id).guardians.len(), 3);
}

/// The same guard applies to the composite `update_will_settings` entry point,
/// which enforces the identical "new list must not be shorter than the stored
/// threshold" invariant.
#[test]
fn update_will_settings_accepts_guardians_after_a_guardian_less_will() {
    let (env, contract_id, owner, will_id) = setup_without_guardians(9);
    let client = WillContractClient::new(&env, &contract_id);

    client.update_will_settings(
        &will_id,
        &owner,
        &None,
        &Some(vec![&env, Address::generate(&env), Address::generate(&env)]),
        &None,
        &None,
    );

    assert_eq!(client.get_will(&will_id).guardians.len(), 2);
}

/// `update_guardians_weighted` is the only entry point that can set a real
/// threshold (#365) and must keep working after the normalisation -- including
/// its own `InvalidGuardianThreshold` rejection, which is unchanged.
#[test]
fn update_guardians_weighted_still_sets_and_validates_a_real_threshold() {
    let (env, contract_id, owner, will_id) = setup_without_guardians(9);
    let client = WillContractClient::new(&env, &contract_id);

    client.update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: Address::generate(&env),
                weight: 1,
            },
            GuardianSpec {
                address: Address::generate(&env),
                weight: 1,
            },
        ],
        &Some(2),
    );
    assert_eq!(client.get_will(&will_id).guardian_threshold, 2);

    // A threshold above the two guardians' combined weight of 2 is still
    // rejected by the same error.
    let g3 = Address::generate(&env);
    let result = client.try_update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: g3,
                weight: 1,
            },
        ],
        &Some(5),
    );
    assert_eq!(result, Err(Ok(WillError::InvalidGuardianThreshold.into())));
}
