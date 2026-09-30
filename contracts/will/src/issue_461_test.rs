#![cfg(test)]

//! Regression coverage for issue #461: a hashed beneficiary claiming from a
//! multi-token will must be paid its share of *every* locked token, not just
//! the primary one.
//!
//! `reveal_and_claim` already loops over every entry in `will.balances` (not
//! just `will.token`) and computes `total * percentage / unclaimed_bps` for
//! each token independently -- see the "Mirrors distribute()'s multi-token
//! payout" comment directly above that loop in `lib.rs`. This test exercises
//! that path directly with a two-token will, since no test in the codebase
//! covered a hashed beneficiary claiming across more than one token before
//! this file.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec,
    xdr::ToXdr,
    Address, Bytes, Env,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, PREIMAGE_ADDRESS_LENGTH,
    PREIMAGE_LENGTH,
};

const DAY: u64 = 86_400;

fn preimage_for(env: &Env, claimant: &Address, salt: u8) -> Bytes {
    let mut raw = [salt; PREIMAGE_LENGTH as usize];
    let digest = env.crypto().sha256(&claimant.clone().to_xdr(env));
    raw[..PREIMAGE_ADDRESS_LENGTH as usize].copy_from_slice(&digest.to_array());
    Bytes::from_array(env, &raw)
}

#[test]
fn hashed_beneficiary_claims_its_share_of_every_token() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let claimant = Address::generate(&env);
    let visible_beneficiary = Address::generate(&env);

    let token_a = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_a).mint(&owner, &1_000_000);
    StellarAssetClient::new(&env, &token_b).mint(&owner, &2_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // token_a is the primary token; token_b is a second, larger balance.
    let will_id = client.create_will(
        &owner,
        &vec![
            &env,
            (token_a.clone(), 1_000_000_i128),
            (token_b.clone(), 2_000_000_i128),
        ],
        // `FixedAmount` (not `Percentage`) so the visible list does not need
        // to sum to 10,000 on its own -- the hashed beneficiary's 5,000 bps
        // is reserved separately, the same way issue_369_test.rs and
        // issue_370_test.rs set up their visible beneficiary.
        &vec![
            &env,
            Beneficiary {
                address: visible_beneficiary,
                allocation: Allocation::FixedAmount(100_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // The hashed beneficiary reserves the other 5,000 bps.
    let preimage = preimage_for(&env, &claimant, 0x42);
    let commitment = env.crypto().sha256(&preimage);
    let commitment_bytes = Bytes::from_array(&env, &commitment.to_array());
    client.add_hashed_beneficiary(&will_id, &owner, &commitment_bytes, &5_000);

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    client.reveal_and_claim(&will_id, &claimant, &preimage);

    let token_a_client = TokenClient::new(&env, &token_a);
    let token_b_client = TokenClient::new(&env, &token_b);

    // The hashed beneficiary is the only unclaimed slot, so it draws the
    // *entire* withheld reserve on each token -- half of token_a's original
    // 1,000,000 and half of token_b's original 2,000,000.
    assert_eq!(
        token_a_client.balance(&claimant),
        500_000,
        "must be paid its share of the primary token"
    );
    assert_eq!(
        token_b_client.balance(&claimant),
        1_000_000,
        "must be paid its share of the second token too, not just the primary one"
    );
}
