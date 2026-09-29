#![cfg(test)]

//! Regression tests for issue #369: `reveal_and_claim` must bind the pre-image
//! to `claimant`.
//!
//! The doc comment for `reveal_and_claim` has always said the first 32 bytes of
//! the 64-byte pre-image are the beneficiary's address and that the funds go to
//! the revealed address — but the code never parsed the pre-image. It hashed
//! whatever it was handed, matched the commitment, and paid whichever
//! `claimant` the caller supplied.
//!
//! That is exploitable because a pre-image is not secret once used: it appears
//! verbatim in the transaction arguments, in simulation results, and in the
//! mempool while the claim is pending. A third party who saw it could replay it
//! with *their own* address as `claimant` and take the reserved share before
//! the real beneficiary got a transaction confirmed.
//!
//! `reveal_and_claim` now decodes the leading `PREIMAGE_ADDRESS_LENGTH` bytes
//! back into an `Address` and requires it to equal `claimant`.

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

/// Builds the documented pre-image for `beneficiary`: a 32-byte address
/// fingerprint followed by a 32-byte salt. The fingerprint is
/// `sha256(xdr(beneficiary))`, the same derivation `reveal_and_claim` performs
/// on `claimant`.
fn preimage_for(env: &Env, beneficiary: &Address, salt: u8) -> Bytes {
    let mut raw = [salt; PREIMAGE_LENGTH as usize];
    let digest = env.crypto().sha256(&beneficiary.clone().to_xdr(env));
    raw[..PREIMAGE_ADDRESS_LENGTH as usize].copy_from_slice(&digest.to_array());
    Bytes::from_array(env, &raw)
}

/// Registers a released will with one hashed-beneficiary slot committed to a
/// freshly generated beneficiary, and returns the env, client, token client,
/// that beneficiary, and the will id.
fn setup() -> (
    Env,
    WillContractClient<'static>,
    TokenClient<'static>,
    Address,
    u64,
) {
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
    let token = TokenClient::new(&env, &token_address);

    // A `FixedAmount` list leaves headroom for the hashed beneficiary's share;
    // a `Percentage` list must already sum to 10,000.
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

    let beneficiary = Address::generate(&env);
    let preimage = preimage_for(&env, &beneficiary, 0x11);
    let commitment = env.crypto().sha256(&preimage);
    client.add_hashed_beneficiary(
        &will_id,
        &owner,
        &Bytes::from_array(&env, &commitment.to_array()),
        &5_000,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    (env, client, token, beneficiary, will_id)
}

/// The happy path still works: the beneficiary whose address is in the
/// pre-image claims their reserved share.
#[test]
fn the_address_in_the_preimage_can_still_claim() {
    let (env, client, token, beneficiary, will_id) = setup();
    let preimage = preimage_for(&env, &beneficiary, 0x11);

    client.reveal_and_claim(&will_id, &beneficiary, &preimage);

    assert!(
        token.balance(&beneficiary) > 0,
        "the share must be paid out"
    );
    assert!(
        client
            .get_will(&will_id)
            .hashed_beneficiaries
            .get(0)
            .unwrap()
            .claimed
    );
}

/// Issue #369, acceptance criterion: a *valid* pre-image replayed with a
/// different claimant is rejected with a clear error, and the slot stays
/// unclaimed so the real beneficiary can still claim.
#[test]
fn a_stolen_preimage_with_a_different_claimant_is_rejected() {
    let (env, client, token, beneficiary, will_id) = setup();
    let preimage = preimage_for(&env, &beneficiary, 0x11);
    let thief = Address::generate(&env);

    assert_eq!(
        client.try_reveal_and_claim(&will_id, &thief, &preimage),
        Err(Ok(WillError::PreimageAddressMismatch.into())),
        "a pre-image naming someone else must not pay out"
    );

    assert_eq!(token.balance(&thief), 0, "the thief must receive nothing");
    assert!(
        !client
            .get_will(&will_id)
            .hashed_beneficiaries
            .get(0)
            .unwrap()
            .claimed,
        "a rejected claim must not burn the slot"
    );

    // ...and the rightful owner can still claim afterwards.
    client.reveal_and_claim(&will_id, &beneficiary, &preimage);
    assert!(token.balance(&beneficiary) > 0);
}

/// A pre-image with the correct layout but bound to the attacker passes the
/// address check and is then refused by the commitment lookup: the address
/// half alone never authorises a claim.
#[test]
fn a_rewritten_preimage_for_the_thief_matches_no_commitment() {
    let (env, client, _token, _beneficiary, will_id) = setup();
    let thief = Address::generate(&env);

    let forged = preimage_for(&env, &thief, 0x11);
    assert_eq!(
        client.try_reveal_and_claim(&will_id, &thief, &forged),
        Err(Ok(WillError::InvalidPreimage.into()))
    );
}

/// A pre-image whose first 32 bytes are not anyone's fingerprint is rejected
/// with the same clear error.
#[test]
fn a_preimage_with_an_unrelated_address_half_is_rejected() {
    let (env, client, _token, beneficiary, will_id) = setup();

    let mut raw = [0xFFu8; PREIMAGE_LENGTH as usize];
    raw[PREIMAGE_ADDRESS_LENGTH as usize] = 0x01; // keep the tail non-uniform
    let garbage = Bytes::from_array(&env, &raw);

    assert_eq!(
        client.try_reveal_and_claim(&will_id, &beneficiary, &garbage),
        Err(Ok(WillError::PreimageAddressMismatch.into()))
    );
}

/// The binding is checked after the length check but before the commitment
/// lookup, so a short pre-image still reports `InvalidPreimageLength`.
#[test]
fn the_length_check_still_takes_precedence_over_the_binding_check() {
    let (env, client, _token, beneficiary, will_id) = setup();

    let short = Bytes::from_array(&env, &[7u8; PREIMAGE_LENGTH as usize - 1]);
    assert_eq!(
        client.try_reveal_and_claim(&will_id, &beneficiary, &short),
        Err(Ok(WillError::InvalidPreimageLength.into()))
    );
}
