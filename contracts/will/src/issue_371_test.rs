#![cfg(test)]

//! Regression tests for issue #371: `add_hashed_beneficiary` must reject
//! commitments that can never be claimed.
//!
//! The entry point used to push whatever commitment and percentage the owner
//! supplied, validating only that the combined total did not exceed 10,000 bps.
//! That accepted three shapes which permanently strand the reserved share:
//! a duplicate commitment (the second slot is unreachable, because
//! `reveal_and_claim` always matches the first), a percentage of 0 (reserves
//! nothing but still occupies a slot and dilutes the other hashed
//! beneficiaries), and a commitment that is not a 32-byte SHA-256 digest (no
//! pre-image can ever hash to it).

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Bytes, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

/// Creates a funded `Active` will whose single visible beneficiary takes a
/// `FixedAmount`, leaving headroom for hashed shares to be reserved against.
fn setup<'a>() -> (Env, WillContractClient<'a>, Address, u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(500_000),
        },
    ];
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    (env, client, owner, will_id)
}

/// A well-formed 32-byte SHA-256 commitment over a 64-byte pre-image.
fn valid_commitment(env: &Env, salt: u8) -> Bytes {
    let preimage = Bytes::from_array(env, &[salt; 64]);
    let digest = env.crypto().sha256(&preimage);
    Bytes::from_array(env, &digest.to_array())
}

/// Issue #371, acceptance criterion 1: a commitment that is not exactly 32
/// bytes can never match any pre-image, so it must be rejected.
#[test]
fn a_commitment_that_is_not_32_bytes_is_rejected() {
    let (env, client, owner, will_id) = setup();

    for len in [0usize, 1, 20, 31, 33, 64, 65] {
        let raw: std::vec::Vec<u8> = std::vec![3u8; len];
        let commitment = Bytes::from_slice(&env, &raw);
        assert_eq!(
            client.try_add_hashed_beneficiary(&will_id, &owner, &commitment, &2_000),
            Err(Ok(WillError::InvalidCommitmentLength.into())),
            "a {len}-byte commitment must be rejected"
        );
    }

    assert_eq!(client.get_will(&will_id).hashed_beneficiaries.len(), 0);
}

/// Exactly 32 bytes is accepted, and a 32-byte commitment that happens to be
/// all zeroes (a valid `Bytes`, just an unlikely digest) is still accepted —
/// the check is on length, not on the digest's value.
#[test]
fn a_32_byte_commitment_is_accepted() {
    let (env, client, owner, will_id) = setup();

    client.add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 1), &2_000);
    client.add_hashed_beneficiary(
        &will_id,
        &owner,
        &Bytes::from_array(&env, &[0u8; 32]),
        &2_000,
    );

    assert_eq!(client.get_will(&will_id).hashed_beneficiaries.len(), 2);
}

/// Issue #371, acceptance criterion 2: the same commitment twice on one will
/// is rejected — `reveal_and_claim` always matches the first slot, so the second
/// would be permanently unclaimable.
#[test]
fn a_duplicate_commitment_on_the_same_will_is_rejected() {
    let (env, client, owner, will_id) = setup();

    let commitment = valid_commitment(&env, 7);
    client.add_hashed_beneficiary(&will_id, &owner, &commitment, &2_000);

    assert_eq!(
        client.try_add_hashed_beneficiary(&will_id, &owner, &commitment, &2_000),
        Err(Ok(WillError::DuplicateCommitment.into())),
    );
    assert_eq!(
        client.get_will(&will_id).hashed_beneficiaries.len(),
        1,
        "the rejected duplicate must not have been appended"
    );

    // Distinct commitments are still fine, whatever order they arrive in.
    client.add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 8), &2_000);
    client.add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 9), &2_000);
    assert_eq!(client.get_will(&will_id).hashed_beneficiaries.len(), 3);
}

/// Issue #371, acceptance criterion 3: a percentage of 0 is rejected. It
/// reserves nothing, yet `unclaimed_hashed_bps` counts it in the denominator of
/// every other hashed beneficiary's share, so it silently dilutes them.
#[test]
fn a_zero_percentage_is_rejected() {
    let (env, client, owner, will_id) = setup();

    assert_eq!(
        client.try_add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 1), &0),
        Err(Ok(WillError::InvalidPercentages.into())),
    );
    assert_eq!(client.get_will(&will_id).hashed_beneficiaries.len(), 0);

    // A zero percentage is rejected even when the commitment is a duplicate or
    // malformed — every rule is independent.
    assert_eq!(
        client.try_add_hashed_beneficiary(
            &will_id,
            &owner,
            &Bytes::from_array(&env, &[1u8; 31]),
            &0
        ),
        Err(Ok(WillError::InvalidCommitmentLength.into())),
    );
}

/// The pre-existing total check still applies: two valid 6,000 bps shares
/// exceed 10,000 and are rejected with `InvalidPercentages`.
#[test]
fn the_existing_total_percentage_check_still_applies() {
    let (env, client, owner, will_id) = setup();

    client.add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 1), &6_000);
    assert_eq!(
        client.try_add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 2), &6_000),
        Err(Ok(WillError::InvalidPercentages.into())),
    );
    // ...and exactly 10,000 in total is still fine.
    client.add_hashed_beneficiary(&will_id, &owner, &valid_commitment(&env, 2), &4_000);
    assert_eq!(client.get_will(&will_id).hashed_beneficiaries.len(), 2);
}

/// The owner and status checks keep their existing precedence: a non-owner or
/// a non-`Active` will is rejected before any commitment validation runs.
#[test]
fn owner_and_status_checks_still_take_precedence() {
    let (env, client, owner, will_id) = setup();
    let bad_commitment = Bytes::from_array(&env, &[1u8; 31]);

    assert_eq!(
        client.try_add_hashed_beneficiary(
            &will_id,
            &Address::generate(&env),
            &bad_commitment,
            &0
        ),
        Err(Ok(WillError::NotOwner.into())),
    );

    // Release the will, then confirm the status error wins over the length one.
    env.ledger().with_mut(|l| l.timestamp += 91 * 86_400);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * 86_400);
    client.release_inheritance(&will_id, &None);

    assert_eq!(
        client.try_add_hashed_beneficiary(&will_id, &owner, &bad_commitment, &2_000),
        Err(Ok(WillError::WillNotActive.into())),
    );
}
