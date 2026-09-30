#![cfg(test)]

//! Regression tests for issue #370: `reveal_and_claim` must validate that the
//! pre-image is exactly [`PREIMAGE_LENGTH`] bytes, as its documentation
//! requires.
//!
//! The entry point used to hash whatever it was handed — including an empty
//! `Bytes` — so any byte string whose SHA-256 happened to match a stored
//! commitment was accepted and every malformed input fell through to the
//! generic `InvalidPreimage` after a wasted hash. The length is now checked
//! first and reported as the dedicated `InvalidPreimageLength` error.
//!
//! This file was found disconnected from the crate's module tree (no `mod
//! issue_370_test;` anywhere in `lib.rs`, so it never compiled or ran as part
//! of `cargo test`) and, independently, out of date with issue #369's later
//! address-binding requirement: its pre-images were not derived from the
//! claimant address `reveal_and_claim` now checks them against, so several
//! cases that meant to exercise the length/commitment checks actually hit
//! `PreimageAddressMismatch` first. Rewired and updated to build pre-images
//! the same way `issue_369_test.rs` does.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec,
    xdr::ToXdr,
    Address, Bytes, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, WillError, PREIMAGE_ADDRESS_LENGTH,
    PREIMAGE_LENGTH,
};

const DAY: u64 = 86_400;

/// Builds the documented pre-image for `claimant`: `PREIMAGE_ADDRESS_LENGTH`
/// bytes of `sha256(xdr(claimant))` followed by a salt filling the rest —
/// the same derivation `reveal_and_claim` recomputes internally (#369).
fn preimage_for(env: &Env, claimant: &Address, salt: u8) -> Bytes {
    let mut raw = [salt; PREIMAGE_LENGTH as usize];
    let digest = env.crypto().sha256(&claimant.clone().to_xdr(env));
    raw[..PREIMAGE_ADDRESS_LENGTH as usize].copy_from_slice(&digest.to_array());
    Bytes::from_array(env, &raw)
}

/// Creates a funded, released will holding one hashed-beneficiary slot
/// committed to `sha256(preimage_for(claimant, salt))`. Returns the env, a
/// client, the claimant, the matching pre-image, the token address, and the
/// will id.
fn setup_with_released_will<'a>(
    salt: u8,
) -> (Env, WillContractClient<'a>, Address, Bytes, Address, u64) {
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
    let claimant = Address::generate(&env);

    // A `FixedAmount` list leaves headroom for the hashed beneficiary's share
    // (a `Percentage` list must already sum to 10,000).
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: Address::generate(&env),
            allocation: Allocation::FixedAmount(500_000),
        },
    ];
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &beneficiaries,
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let preimage = preimage_for(&env, &claimant, salt);
    let commitment = env.crypto().sha256(&preimage);
    let commitment_bytes = Bytes::from_array(&env, &commitment.to_array());
    client.add_hashed_beneficiary(&will_id, &owner, &commitment_bytes, &5_000);

    // Trigger and release so `reveal_and_claim` is reachable.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    (env, client, claimant, preimage, token_address, will_id)
}

/// The happy path must keep working: a correctly-bound, 64-byte pre-image
/// still claims the reserved share.
#[test]
fn a_well_formed_preimage_still_claims() {
    let (_env, client, claimant, preimage, token_address, will_id) = setup_with_released_will(0xAB);
    let token = TokenClient::new(&_env, &token_address);

    client.reveal_and_claim(&will_id, &claimant, &preimage);

    assert!(
        token.balance(&claimant) > 0,
        "the reserved share must be paid out"
    );
    assert!(
        client
            .get_will(&will_id)
            .hashed_beneficiaries
            .get(0)
            .unwrap()
            .claimed,
        "the slot must be marked claimed"
    );
}

/// Issue #370, acceptance criterion: empty, 63-byte and 65-byte pre-images are
/// each rejected with `InvalidPreimageLength` — not the generic
/// `InvalidPreimage` (or, now that #369 also exists, `PreimageAddressMismatch`)
/// they might otherwise fall through to. The length check runs first
/// regardless of what the bytes actually contain.
#[test]
fn wrong_length_preimages_are_rejected() {
    let (env, client, claimant, preimage, _token_address, will_id) = setup_with_released_will(7);

    for len in [
        0usize,
        1,
        32,
        PREIMAGE_LENGTH as usize - 1,
        PREIMAGE_LENGTH as usize + 1,
        128,
    ] {
        let bytes: std::vec::Vec<u8> = std::vec![7u8; len];
        let bytes = Bytes::from_slice(&env, &bytes);
        assert_eq!(
            client.try_reveal_and_claim(&will_id, &claimant, &bytes),
            Err(Ok(WillError::InvalidPreimageLength.into())),
            "a {len}-byte pre-image must be rejected as a wrong length"
        );
    }

    // The slot is untouched by the rejected attempts, and the real pre-image
    // still claims it.
    assert!(
        !client
            .get_will(&will_id)
            .hashed_beneficiaries
            .get(0)
            .unwrap()
            .claimed
    );
    client.reveal_and_claim(&will_id, &claimant, &preimage);
    assert!(
        client
            .get_will(&will_id)
            .hashed_beneficiaries
            .get(0)
            .unwrap()
            .claimed
    );
}

/// A correctly-sized, correctly-bound pre-image that simply does not match
/// any stored commitment (wrong salt) still gets the generic
/// `InvalidPreimage` — the length and binding checks must not swallow it.
#[test]
fn a_correct_length_but_wrong_preimage_still_reports_invalid_preimage() {
    let (env, client, claimant, _preimage, _token_address, will_id) = setup_with_released_will(7);

    let wrong = preimage_for(&env, &claimant, 9);
    assert_eq!(
        client.try_reveal_and_claim(&will_id, &claimant, &wrong),
        Err(Ok(WillError::InvalidPreimage.into()))
    );
}

/// Truncating or padding the *real* pre-image is rejected on length, even
/// though the owner can no longer produce a matching reveal for it.
#[test]
fn a_truncated_or_padded_real_preimage_is_rejected() {
    let (env, client, claimant, preimage, _token_address, will_id) = setup_with_released_will(7);

    let raw: std::vec::Vec<u8> = preimage.iter().collect();
    let raw: std::vec::Vec<u8> = raw.to_vec();

    let truncated: std::vec::Vec<u8> = raw[..raw.len() - 1].to_vec();
    assert_eq!(
        client.try_reveal_and_claim(&will_id, &claimant, &Bytes::from_slice(&env, &truncated)),
        Err(Ok(WillError::InvalidPreimageLength.into()))
    );

    let mut padded = raw;
    padded.push(0);
    assert_eq!(
        client.try_reveal_and_claim(&will_id, &claimant, &Bytes::from_slice(&env, &padded)),
        Err(Ok(WillError::InvalidPreimageLength.into()))
    );
}

/// The length check runs before the status check is even reachable in a
/// meaningful way here -- but regardless of ordering, an unreleased will
/// still reports `WillNotReleased` for a malformed pre-image, since
/// `assert_status` runs first in `reveal_and_claim`.
#[test]
fn the_status_check_still_takes_precedence_over_the_length_check() {
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
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
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
        &0,
        &None,
        &0,
    );

    assert_eq!(
        client.try_reveal_and_claim(&will_id, &Address::generate(&env), &Bytes::new(&env)),
        Err(Ok(WillError::WillNotReleased.into()))
    );
}
