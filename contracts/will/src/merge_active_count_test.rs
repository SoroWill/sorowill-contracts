#![cfg(test)]

//! Regression test for issue #187: ensure that `merge_wills` decrements
//! `ProtocolStats.active_will_count` when marking `will_b` as `Cancelled`.
//!
//! Every other path that terminates a will's active lifecycle calls
//! `storage::decrement_active_will_count` — `cancel_will` and `distribute` both do.
//! `merge_wills` was missing this call, causing the active count to be permanently
//! over-counted by one for each merge operation.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

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

/// Regression test for issue #187: `merge_wills` should decrement the active
/// will count when marking `will_b` as `Cancelled`.
#[test]
fn merge_wills_decrements_active_count() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 500_000_i128)];

    let will_id_a = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    let will_id_b = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let stats_before = client.get_protocol_stats();
    assert_eq!(
        stats_before.active_will_count, 2,
        "should have 2 active wills"
    );

    client.merge_wills(&owner, &will_id_a, &will_id_b);

    let stats_after = client.get_protocol_stats();
    assert_eq!(
        stats_after.active_will_count, 1,
        "after merge, should have 1 active will (will_b should be cancelled and decremented)"
    );
}

/// Ensure the active count is accurate after multiple merges.
#[test]
fn merge_wills_multiple_decrements() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 500_000_i128)];

    let will_1 = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    let will_2 = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    let will_3 = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let stats_start = client.get_protocol_stats();
    assert_eq!(
        stats_start.active_will_count, 3,
        "should have 3 active wills"
    );

    client.merge_wills(&owner, &will_1, &will_2);
    let stats_after_first = client.get_protocol_stats();
    assert_eq!(
        stats_after_first.active_will_count, 2,
        "after first merge, should have 2 active wills"
    );

    client.merge_wills(&owner, &will_1, &will_3);
    let stats_after_second = client.get_protocol_stats();
    assert_eq!(
        stats_after_second.active_will_count, 1,
        "after second merge, should have 1 active will"
    );
}
