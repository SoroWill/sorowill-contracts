#![cfg(test)]

//! Regression tests for issue #623: a pre-image whose salt is all zero bytes is
//! refused, because such a salt is the easiest one to guess.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, xdr::ToXdr, Address, Bytes, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, WillError, PREIMAGE_ADDRESS_LENGTH,
    PREIMAGE_LENGTH,
};

const DAY: u64 = 86_400;

fn preimage_for(env: &Env, claimant: &Address, salt: &[u8]) -> Bytes {
    let mut raw = [0u8; PREIMAGE_LENGTH as usize];
    raw[PREIMAGE_ADDRESS_LENGTH as usize..].copy_from_slice(salt);
    let digest = env.crypto().sha256(&claimant.clone().to_xdr(env));
    raw[..PREIMAGE_ADDRESS_LENGTH as usize].copy_from_slice(&digest.to_array());
    Bytes::from_array(env, &raw)
}

fn setup_released<'a>(
    salt: &[u8],
) -> (Env, WillContractClient<'a>, Address, Bytes, Address, u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(owner.clone()).address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let claimant = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary { address: Address::generate(&env), allocation: Allocation::FixedAmount(500_000) },
    ];
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
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
    client.add_hashed_beneficiary(&will_id, &owner, &Bytes::from_array(&env, &commitment.to_array()), &5_000);
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);
    (env, client, claimant, preimage, token, will_id)
}

#[test]
fn an_all_zero_salt_is_refused() {
    let (_env, client, claimant, preimage, _token, will_id) = setup_released(&[0u8; 32]);
    let result = client.try_reveal_and_claim(&will_id, &claimant, &preimage);
    assert_eq!(result, Err(Ok(WillError::WeakPreimageSalt)));
}

#[test]
fn a_salt_with_some_nonzero_bytes_still_claims() {
    let mut salt = [0u8; 32];
    salt[31] = 1;
    let (env, client, claimant, preimage, _token, will_id) = setup_released(&salt);
    client.reveal_and_claim(&will_id, &claimant, &preimage);
    let _ = env;
}
