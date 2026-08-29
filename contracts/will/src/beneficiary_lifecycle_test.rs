#![cfg(test)]

//! Regression test proving `update_beneficiaries` is not a cached/stale
//! write: a beneficiary-list change made via `update_beneficiaries` must be
//! exactly what `release_inheritance` pays out later, after the will goes
//! through `trigger_will` and the full grace period — not whatever list was
//! present at `create_will` time.
//!
//! Self-contained module (its own `setup`) so it does not depend on the
//! large pre-existing `test.rs` suite, which still constructs `Beneficiary`
//! via the old `basis_points` field.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

#[test]
fn release_pays_the_updated_beneficiary_list_not_the_original_one() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);
    let token = TokenClient::new(&env, &token_address);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let original = Address::generate(&env);
    let replacement_a = Address::generate(&env);
    let replacement_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: original.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // The owner changes their mind well before ever missing a check-in.
    client.update_beneficiaries(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: replacement_a.clone(),
                allocation: Allocation::Percentage(6_000),
            },
            Beneficiary {
                address: replacement_b.clone(),
                allocation: Allocation::Percentage(4_000),
            },
        ],
    );

    // Sanity check: the update is visible via get_will before anything else
    // happens.
    let updated = client.get_will(&will_id);
    assert_eq!(updated.beneficiaries.len(), 2);

    // Full lifecycle: missed check-in -> trigger -> grace period -> release.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    assert_eq!(
        token.balance(&original),
        0,
        "the original (renounced/replaced) beneficiary must not be paid"
    );
    assert_eq!(token.balance(&replacement_a), 600_000);
    assert_eq!(token.balance(&replacement_b), 400_000);
    assert_eq!(token.balance(&client.address), 0);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

/// The same property, but the beneficiary list is changed a second time
/// *after* the will has already been triggered (still inside the grace
/// period) — `guardian_trigger`-free early-release paths and cancellation
/// aside, an owner is allowed to keep adjusting beneficiaries up until
/// release actually happens, and the final list must win.
#[test]
fn release_pays_the_latest_of_several_beneficiary_updates() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);
    let token = TokenClient::new(&env, &token_address);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let first = Address::generate(&env);
    let second = Address::generate(&env);
    let third = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: first.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    client.update_beneficiaries(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: second.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
    );
    client.update_beneficiaries(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: third.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    assert_eq!(token.balance(&first), 0);
    assert_eq!(token.balance(&second), 0);
    assert_eq!(token.balance(&third), 1_000_000);
}

/// Verify that attempting to update beneficiaries after the will is triggered
/// (i.e., when in Triggered status) is rejected, as beneficiaries can only be
/// modified while the will is Active.
#[test]
fn update_beneficiaries_rejected_after_trigger() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);
    let _token = TokenClient::new(&env, &token_address);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let original = Address::generate(&env);
    let new_beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: original.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Advance past check-in deadline and trigger the will
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // The will is now Triggered; attempting to update beneficiaries should fail
    let result = client.try_update_beneficiaries(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: new_beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
    );

    assert!(
        result.is_err(),
        "update_beneficiaries must be rejected after trigger"
    );
}
