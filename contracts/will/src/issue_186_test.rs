#![cfg(test)]

//! Regression test for issue #186: `HashedBeneficiary.percentage` is interpreted
//! on a 0-100 scale in `reveal_and_claim` (dividing by 100) but validated on the
//! contract's 0-10,000 basis-point scale everywhere else. An owner following the
//! codebase's dominant basis-point convention and passing 5000 (intended 50%) would
//! pass validation but `reveal_and_claim` would compute balance * 5000 / 100 = 50x
//! the intended payout.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Bytes, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

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

/// Regression test asserting a hashed beneficiary configured with 5000 (intended 50%)
/// pays out exactly 50% of balance. This requires standardizing on the 0-10,000
/// basis-point scale used everywhere else in the contract.
#[test]
fn hashed_beneficiary_percentage_basis_points_payout() {
    let (env, client, owner, token, token_address) = setup();

    let public_beneficiary = Address::generate(&env);
    let secret_address = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: public_beneficiary.clone(),
            allocation: Allocation::Percentage(5_000), // 50%
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address.clone(), 1_000_000_i128)];

    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Create hashed beneficiary commitment
    let preimage_bytes = [0u8; 64];
    let preimage = Bytes::from_array(&env, &preimage_bytes);
    let commitment = env.crypto().sha256(&preimage);
    let commitment_bytes = Bytes::from_array(&env, &commitment.to_array());

    // Add hashed beneficiary with 5000 basis points (50%)
    client.add_hashed_beneficiary(&will_id, &owner, &commitment_bytes, &5_000);

    // Trigger will and wait for grace period
    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);

    // Record initial balances
    let _secret_initial = token.balance(&secret_address);
    let _public_initial = token.balance(&public_beneficiary);

    // Release inheritance - both beneficiaries should receive 50% each
    client.release_inheritance(&will_id, &None);

    // Public beneficiary gets their 50%
    assert_eq!(
        token.balance(&public_beneficiary),
        500_000,
        "public beneficiary with 50% allocation should receive 500,000"
    );

    // Now reveal and claim for the secret beneficiary
    client.reveal_and_claim(&will_id, &secret_address, &preimage);

    // Secret beneficiary should also get 50% (500,000), not 50x the intended amount
    assert_eq!(
        token.balance(&secret_address),
        500_000,
        "hashed beneficiary with 5000 basis points should receive exactly 50% (500,000) of original balance"
    );
}
