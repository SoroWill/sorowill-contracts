//! Regression tests for issue #356: guardian weight summation must be
//! overflow-checked so callers get a typed `WillError` instead of a trap.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    Allocation, Beneficiary, GuardianSpec, WillContract, WillContractClient, WillError,
    MAX_GUARDIAN_WEIGHT,
};

/// Creates an `Active` will owned by a freshly generated owner, with no
/// guardians and a threshold of 1.
fn setup() -> (Env, Address, Address, u64) {
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
        &vec![&env],
        &1,
        &None,
        &0,
    );

    (env, contract_id, owner, will_id)
}

/// Weights whose sum exceeds `u32::MAX` must return a typed error rather than
/// aborting with an arithmetic overflow trap.
#[test]
fn weights_summing_past_u32_max_return_typed_error() {
    let (env, contract_id, owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    // Two guardians at u32::MAX / 2 + 1: the sum is representable, the
    // individual weights are not valid, and the third pushes the total over.
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);
    let huge = u32::MAX;

    let result = client.try_update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: g1,
                weight: huge,
            },
            GuardianSpec {
                address: g2,
                weight: huge,
            },
            GuardianSpec {
                address: g3,
                weight: huge,
            },
        ],
        &Some(1),
    );

    assert_eq!(
        result.unwrap_err().unwrap(),
        WillError::InvalidGuardianThreshold.into(),
        "an unrepresentable weight total must be a typed WillError, not a trap"
    );
}

/// A single weight above the documented per-guardian cap is rejected with the
/// same typed error.
#[test]
fn weight_above_max_guardian_weight_is_rejected() {
    let (env, contract_id, owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    let result = client.try_update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: Address::generate(&env),
                weight: MAX_GUARDIAN_WEIGHT + 1,
            },
        ],
        &Some(1),
    );

    assert_eq!(
        result.unwrap_err().unwrap(),
        WillError::InvalidGuardianThreshold.into()
    );
}

/// The cap itself is still accepted, and the stored weights are used for quorum.
#[test]
fn weight_at_max_guardian_weight_is_accepted() {
    let (env, contract_id, owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    client.update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: g1.clone(),
                weight: MAX_GUARDIAN_WEIGHT,
            },
            GuardianSpec {
                address: g2,
                weight: 1,
            },
        ],
        &Some(MAX_GUARDIAN_WEIGHT + 1),
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_threshold, MAX_GUARDIAN_WEIGHT + 1);
    assert_eq!(will.guardians.len(), 2);
    assert_eq!(will.guardians.get(0).unwrap().weight, MAX_GUARDIAN_WEIGHT);
}

/// A weight of zero is still normalised to one, as before.
#[test]
fn zero_weight_is_normalised_to_one() {
    let (env, contract_id, owner, will_id) = setup();
    let client = WillContractClient::new(&env, &contract_id);

    client.update_guardians_weighted(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianSpec {
                address: Address::generate(&env),
                weight: 0,
            },
        ],
        &Some(1),
    );

    assert_eq!(
        client.get_will(&will_id).guardians.get(0).unwrap().weight,
        1
    );
}
