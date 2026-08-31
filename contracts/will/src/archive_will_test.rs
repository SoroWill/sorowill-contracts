#![cfg(test)]

//! Regression coverage for issue #221: `archive_will` removes settled wills from
//! active owner and beneficiary indexes while rejecting in-flight lifecycle states.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

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

    (env.clone(), client, owner, token_address)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

#[test]
fn archive_will_removes_released_will_from_owner_and_beneficiary_indexes() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
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

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    client.archive_will(&will_id);

    let owner_wills = client.get_wills_by_owner(&owner, &None, &10);
    assert!(owner_wills.is_empty(), "released wills must be removed from owner index");

    let beneficiary_wills = client.get_wills_by_beneficiary(&beneficiary, &None, &10);
    assert!(beneficiary_wills.is_empty(), "released wills must be removed from beneficiary index");

    let archived = client.get_will(&will_id);
    assert_eq!(archived.status, WillStatus::Released);
}

#[test]
fn archive_will_removes_cancelled_will_from_active_indexes() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
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

    client.cancel_will(&will_id, &owner);
    client.archive_will(&will_id);

    assert!(client.get_wills_by_owner(&owner, &None, &10).is_empty());
    assert!(client.get_wills_by_beneficiary(&beneficiary, &None, &10).is_empty());
}

/// Regression test for issue #331: archiving a will that is currently in
/// `Triggered` status must remove its id from the global `TriggeredWills`
/// index so that keepers iterating `get_triggered_wills()` never see a
/// dangling id pointing at an entry that no longer resolves.
#[test]
fn archive_triggered_will_removes_id_from_triggered_index() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
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

    // Advance past the check-in deadline and trigger the will.
    advance(&env, 91);
    client.trigger_will(&will_id);

    // The will's id must now appear in the triggered-wills index.
    assert_eq!(
        client.get_triggered_wills().len(),
        1,
        "triggered will must be in index before archival"
    );

    // Archive the triggered will (simulate keeper or owner taking manual
    // action; no grace-period check is required for archive_will itself).
    client.archive_will(&will_id);

    // After archival the triggered-wills index must be empty — no dangling id.
    assert!(
        client.get_triggered_wills().is_empty(),
        "archiving a triggered will must remove its id from the triggered-wills index"
    );
}

#[test]
#[should_panic]
fn archive_will_rejects_active_will() {
    let (env, client, owner, token_address) = setup();
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
        &vec![&env],
        &2,
        &None,
        &0,
    );

    client.archive_will(&will_id);
}
