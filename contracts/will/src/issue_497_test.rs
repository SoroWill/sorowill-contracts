#![cfg(test)]

//! Regression coverage for issue #497: in pull-mode, `reveal_and_claim` must
//! pay a hashed beneficiary their proportional share of *every* token the
//! will holds, not just the primary token. Before this fix, a multi-token
//! will was only ever partially liquidated for a hashed-beneficiary claim --
//! secondary tokens stayed locked in the contract forever once the primary
//! token's share had been paid out and the slot marked claimed.

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

/// Builds the documented pre-image for `beneficiary`: a 32-byte address
/// fingerprint followed by a 32-byte salt, matching `preimage_address_binding`
/// in lib.rs (sha256 of the address's XDR encoding).
fn preimage_for(env: &Env, beneficiary: &Address, salt: u8) -> Bytes {
    let mut raw = [salt; PREIMAGE_LENGTH as usize];
    let digest = env.crypto().sha256(&beneficiary.clone().to_xdr(env));
    raw[..PREIMAGE_ADDRESS_LENGTH as usize].copy_from_slice(&digest.to_array());
    Bytes::from_array(env, &raw)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

#[test]
fn reveal_and_claim_pays_every_token_proportionally_not_just_the_primary() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let primary_sac = env.register_stellar_asset_contract_v2(owner.clone());
    let primary_token = primary_sac.address();
    StellarAssetClient::new(&env, &primary_token).mint(&owner, &10_000_000);

    let secondary_sac = env.register_stellar_asset_contract_v2(owner.clone());
    let secondary_token = secondary_sac.address();
    StellarAssetClient::new(&env, &secondary_token).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // A FixedAmount visible beneficiary leaves headroom (denominated in the
    // primary token only, per #384) for the hashed beneficiary's reserved
    // share; the secondary token's whole balance is available for that share.
    let visible_beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![
            &env,
            (primary_token.clone(), 1_000_000_i128),
            (secondary_token.clone(), 500_000_i128),
        ],
        &vec![
            &env,
            Beneficiary {
                address: visible_beneficiary,
                allocation: Allocation::FixedAmount(500_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let hashed_beneficiary = Address::generate(&env);
    let preimage = preimage_for(&env, &hashed_beneficiary, 0xAB);
    let commitment = env.crypto().sha256(&preimage);
    let commitment_bytes = Bytes::from_array(&env, &commitment.to_array());

    // 40% reserved for the hashed beneficiary, out of every token's balance.
    client.add_hashed_beneficiary(&will_id, &owner, &commitment_bytes, &4_000);

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    // After release, the will's balances map holds only the withheld
    // hashed-beneficiary pool for each token (see #181) -- the hashed
    // beneficiary's share is their percentage of that pool, which for a
    // single still-unclaimed hashed beneficiary is the whole pool.
    let will_before_claim = client.get_will(&will_id);
    let primary_pool = will_before_claim.balances.get(primary_token.clone()).unwrap_or(0);
    let secondary_pool = will_before_claim.balances.get(secondary_token.clone()).unwrap_or(0);
    assert!(primary_pool > 0, "primary token must have a withheld pool");
    assert!(secondary_pool > 0, "secondary token must have a withheld pool");

    let primary_client = TokenClient::new(&env, &primary_token);
    let secondary_client = TokenClient::new(&env, &secondary_token);
    let primary_before = primary_client.balance(&hashed_beneficiary);
    let secondary_before = secondary_client.balance(&hashed_beneficiary);
    assert_eq!(primary_before, 0);
    assert_eq!(secondary_before, 0);

    client.reveal_and_claim(&will_id, &hashed_beneficiary, &preimage);

    // The whole point of #497: the secondary token must actually move, not
    // just the primary one.
    let primary_after = primary_client.balance(&hashed_beneficiary);
    let secondary_after = secondary_client.balance(&hashed_beneficiary);
    assert_eq!(
        primary_after, primary_pool,
        "the sole unclaimed hashed beneficiary must receive the entire primary-token pool"
    );
    assert_eq!(
        secondary_after, secondary_pool,
        "the sole unclaimed hashed beneficiary must receive the entire secondary-token pool, \
         not have it stranded in the contract (#497)"
    );

    // And the will's own bookkeeping must reflect the full multi-token
    // payout, not just the primary token's.
    let will_after_claim = client.get_will(&will_id);
    assert_eq!(will_after_claim.balances.get(primary_token).unwrap_or(0), 0);
    assert_eq!(will_after_claim.balances.get(secondary_token).unwrap_or(0), 0);
    assert!(will_after_claim.hashed_beneficiaries.get(0).unwrap().claimed);
}

#[test]
fn reveal_and_claim_splits_a_token_proportionally_across_two_unclaimed_hashed_beneficiaries() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_a = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token_a).mint(&owner, &10_000_000);
    let token_b = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token_b).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let visible = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_a.clone(), 1_000_000_i128), (token_b.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: visible,
                allocation: Allocation::FixedAmount(100_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let claimant1 = Address::generate(&env);
    let preimage1 = preimage_for(&env, &claimant1, 0x01);
    let commitment1 = env.crypto().sha256(&preimage1);
    client.add_hashed_beneficiary(
        &will_id,
        &owner,
        &Bytes::from_array(&env, &commitment1.to_array()),
        &2_000, // 20%
    );

    let claimant2 = Address::generate(&env);
    let preimage2 = preimage_for(&env, &claimant2, 0x02);
    let commitment2 = env.crypto().sha256(&preimage2);
    client.add_hashed_beneficiary(
        &will_id,
        &owner,
        &Bytes::from_array(&env, &commitment2.to_array()),
        &3_000, // 30%
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    let pool_a = client.get_will(&will_id).balances.get(token_a.clone()).unwrap_or(0);
    let pool_b = client.get_will(&will_id).balances.get(token_b.clone()).unwrap_or(0);
    assert!(pool_a > 0 && pool_b > 0);

    // claimant1 has 2,000 of the combined 5,000 unclaimed bps -> 40% of each pool.
    client.reveal_and_claim(&will_id, &claimant1, &preimage1);
    let token_a_client = TokenClient::new(&env, &token_a);
    let token_b_client = TokenClient::new(&env, &token_b);
    assert_eq!(token_a_client.balance(&claimant1), pool_a * 2_000 / 5_000);
    assert_eq!(token_b_client.balance(&claimant1), pool_b * 2_000 / 5_000);

    // claimant2 is now the *only* remaining unclaimed hashed beneficiary, so
    // they receive everything left in both pools, regardless of their
    // percentage being 30 (not 100) of the *original* total.
    client.reveal_and_claim(&will_id, &claimant2, &preimage2);
    let will_final = client.get_will(&will_id);
    assert_eq!(will_final.balances.get(token_a).unwrap_or(0), 0);
    assert_eq!(will_final.balances.get(token_b).unwrap_or(0), 0);
}
