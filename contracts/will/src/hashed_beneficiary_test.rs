#![cfg(test)]

//! Tests for hashed beneficiary functionality.
//! Issue #182: add_hashed_beneficiary validation with percentage beneficiaries
//! Issue #181: Hashed beneficiaries' share preservation during release

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus,
};

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

fn release(env: &Env, client: &WillContractClient, will_id: u64) {
    advance(env, 91);
    client.trigger_will(&will_id);
    advance(env, 8);
    client.release_inheritance(&will_id, &None);
}

/// Issue #182: Test that `add_hashed_beneficiary` works with percentage-based regular beneficiaries.
///
/// Previously, `assert_valid_percentages` would reject any hashed beneficiary percentage
/// when regular beneficiaries with percentage allocations already summed to 10,000.
/// This test verifies that hashed beneficiaries can be added with 0% when regular
/// beneficiaries use percentage allocation.
#[test]
fn hashed_beneficiary_with_percentage_regular_beneficiaries() {
    let (env, client, owner, _token, token_address) = setup();
    let reg_beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: reg_beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];
    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    let hashed_commitment = env.crypto().sha256(&soroban_sdk::Bytes::new(&env));
    let hashed_bytes = soroban_sdk::Bytes::from_array(&env, &hashed_commitment.to_array());

    // This should succeed (fixing issue #182)
    client.add_hashed_beneficiary(&will_id, &owner, &hashed_bytes, &0);

    let will = client.get_will(&will_id);
    assert_eq!(will.hashed_beneficiaries.len(), 1);
    assert_eq!(will.hashed_beneficiaries.get_unchecked(0).percentage, 0);
}

/// Issue #182: Test that `add_hashed_beneficiary` validates combined percentages correctly.
///
/// Ensures that percentage-based beneficiaries plus hashed beneficiary percentages
/// cannot exceed 10,000 basis points.
#[test]
fn hashed_beneficiary_percentage_exceeds_limit() {
    let (env, client, owner, _, token_address) = setup();
    let reg_beneficiary_a = Address::generate(&env);
    let reg_beneficiary_b = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: reg_beneficiary_a,
            allocation: Allocation::Percentage(5_000),
        },
        Beneficiary {
            address: reg_beneficiary_b,
            allocation: Allocation::Percentage(5_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];
    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    let hashed_commitment = env.crypto().sha256(&soroban_sdk::Bytes::new(&env));
    let hashed_bytes = soroban_sdk::Bytes::from_array(&env, &hashed_commitment.to_array());

    // This should be rejected with InvalidPercentages. Soroban's panic
    // message only shows the numeric error code, never the enum variant
    // name, so #[should_panic(expected = "InvalidPercentages")] can never
    // match -- use try_add_hashed_beneficiary instead.
    assert_eq!(
        client.try_add_hashed_beneficiary(&will_id, &owner, &hashed_bytes, &100),
        Err(Ok(WillError::InvalidPercentages.into())),
    );
}

/// Issue #182: Test that `add_hashed_beneficiary` works with fixed-amount beneficiaries.
///
/// Hashed beneficiaries can be added with percentage allocations when regular
/// beneficiaries use fixed amounts, since fixed amounts don't consume the
/// percentage pool.
#[test]
fn hashed_beneficiary_with_fixed_amount_beneficiaries() {
    let (env, client, owner, _token, token_address) = setup();
    let reg_beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: reg_beneficiary,
            allocation: Allocation::FixedAmount(500_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 500_000_i128)];
    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    let hashed_commitment = env.crypto().sha256(&soroban_sdk::Bytes::new(&env));
    let hashed_bytes = soroban_sdk::Bytes::from_array(&env, &hashed_commitment.to_array());

    // Add hashed beneficiary with a percentage share
    client.add_hashed_beneficiary(&will_id, &owner, &hashed_bytes, &5_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.hashed_beneficiaries.len(), 1);
    assert_eq!(will.hashed_beneficiaries.get_unchecked(0).percentage, 5_000);
}

/// Issue #182: Test that multiple hashed beneficiaries can be added.
///
/// Multiple hashed beneficiaries can coexist on the same will as long as
/// their combined percentages don't exceed the validation limit.
#[test]
fn multiple_hashed_beneficiaries() {
    let (env, client, owner, _token, token_address) = setup();
    let reg_beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: reg_beneficiary,
            allocation: Allocation::FixedAmount(500_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 500_000_i128)];
    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Add multiple hashed beneficiaries
    for _i in 0..3 {
        let hashed_commitment = env.crypto().sha256(&soroban_sdk::Bytes::new(&env));
        let hashed_bytes = soroban_sdk::Bytes::from_array(&env, &hashed_commitment.to_array());
        client.add_hashed_beneficiary(&will_id, &owner, &hashed_bytes, &2_000);
    }

    let will = client.get_will(&will_id);
    assert_eq!(will.hashed_beneficiaries.len(), 3);
}

/// Issue #181: Test that hashed beneficiaries' funds are reserved during release.
///
/// When `release_inheritance` or `guardian_trigger` distributes funds, hashed
/// beneficiaries' shares must be reserved and not paid to regular beneficiaries.
/// The will must track that funds are allocated to hashed beneficiaries even
/// if they have not yet claimed via `reveal_and_claim`.
#[test]
fn hashed_beneficiary_funds_preserved_during_release() {
    let (env, client, owner, token, token_address) = setup();
    let regular_beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: regular_beneficiary.clone(),
            allocation: Allocation::FixedAmount(800_000),
        },
    ];
    // 1,000,000 total leaves 200,000 of headroom beyond the 800,000
    // FixedAmount commitment -- exactly what the 2,000 bps (20%) hashed
    // beneficiary below reserves.
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];
    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Add hashed beneficiary with a percentage share from the remaining balance
    let hashed_commitment = env.crypto().sha256(&soroban_sdk::Bytes::new(&env));
    let hashed_bytes = soroban_sdk::Bytes::from_array(&env, &hashed_commitment.to_array());
    client.add_hashed_beneficiary(&will_id, &owner, &hashed_bytes, &2_000);

    // Trigger will and release
    release(&env, &client, will_id);

    // The will should be Released
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);

    // Regular beneficiary should receive their fixed amount
    assert_eq!(token.balance(&regular_beneficiary), 800_000);

    // The contract should still hold funds for the hashed beneficiary
    // (exact amount depends on implementation of reserve logic)
    let contract_balance = token.balance(&client.address);
    assert!(contract_balance > 0, "Hashed beneficiary share should be reserved in contract");
}
