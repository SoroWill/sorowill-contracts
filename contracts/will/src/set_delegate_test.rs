#![cfg(test)]

//! Unit tests for the `set_delegate` function and its interaction with `check_in`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

fn setup<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (
        env.clone(),
        client,
        owner,
        TokenClient::new(&env, &token_address),
        token_address,
    )
}

/// Test asserting a delegate set via `set_delegate` can successfully call
/// `check_in` on the owner's behalf.
#[test]
fn delegate_can_check_in() {
    let (env, client, owner, _token, token_address) = setup();
    let delegate = Address::generate(&env);
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
        &2,
        &None,
        &0,
    );

    // Set the delegate
    client.set_delegate(&will_id, &owner, &Some(delegate.clone()));

    // Verify delegate is set
    assert_eq!(client.get_will(&will_id).delegate, Some(delegate.clone()));

    // Advance time to near the deadline
    env.ledger().with_mut(|l| l.timestamp += 80 * DAY);

    // Delegate calls check_in on behalf of the owner
    client.check_in(&will_id, &delegate);

    // Will should still be Active
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

/// Test asserting clearing the delegate (`None`) removes that permission.
#[test]
fn clearing_delegate_removes_permission() {
    let (env, client, owner, _token, token_address) = setup();
    let delegate = Address::generate(&env);
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
        &2,
        &None,
        &0,
    );

    // Set the delegate
    client.set_delegate(&will_id, &owner, &Some(delegate.clone()));

    // Clear the delegate
    client.set_delegate(&will_id, &owner, &None);

    // Verify delegate is cleared
    assert_eq!(client.get_will(&will_id).delegate, None);

    // Advance time to near the deadline
    env.ledger().with_mut(|l| l.timestamp += 80 * DAY);

    // Former delegate should not be able to check_in
    assert_eq!(
        client.try_check_in(&will_id, &delegate),
        Err(Ok(WillError::NotOwner.into()))
    );
}

/// Test asserting a non-owner/non-delegate caller is rejected.
#[test]
fn non_owner_non_delegate_cannot_check_in() {
    let (env, client, owner, _token, token_address) = setup();
    let delegate = Address::generate(&env);
    let random_address = Address::generate(&env);
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
        &2,
        &None,
        &0,
    );

    // Set a delegate
    client.set_delegate(&will_id, &owner, &Some(delegate.clone()));

    // Advance time to near the deadline
    env.ledger().with_mut(|l| l.timestamp += 80 * DAY);

    // Random address should not be able to check_in
    assert_eq!(
        client.try_check_in(&will_id, &random_address),
        Err(Ok(WillError::NotOwner.into()))
    );
}
