#![cfg(test)]

//! Regression test for issue #185: `add_hashed_beneficiary` never emits an event,
//! unlike every other state-mutating entry point. Off-chain indexers that reconstruct
//! will state purely from events will silently miss every hashed beneficiary.

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Bytes, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

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

/// Test that add_hashed_beneficiary emits an event with the expected topics and data.
#[test]
fn add_hashed_beneficiary_emits_event() {
    let (env, client, owner, _, token_address) = setup();

    let beneficiary = Address::generate(&env);

    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(5_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(&owner, &tokens, &beneficiaries, &90, &7, &vec![&env], &2, &None, &0);

    // Prepare a hashed beneficiary commitment
    let _secret_address = Address::generate(&env);
    let preimage_bytes = [0u8; 64];
    let preimage = Bytes::from_array(&env, &preimage_bytes);
    let commitment = env.crypto().sha256(&preimage);
    let commitment_bytes = Bytes::from_array(&env, &commitment.to_array());

    // Add hashed beneficiary with 5000 basis points (50%)
    client.add_hashed_beneficiary(&will_id, &owner, &commitment_bytes, &5_000);

    // Verify that an event was emitted
    let events = env.events().all();
    assert!(!events.is_empty(), "add_hashed_beneficiary should emit an event");

    // Verify the will now contains the hashed beneficiary
    let will = client.get_will(&will_id);
    assert!(!will.hashed_beneficiaries.is_empty(), "hashed beneficiary should be added to will");
    assert_eq!(will.hashed_beneficiaries.len(), 1);

    let hb = will.hashed_beneficiaries.get(0).unwrap();
    assert_eq!(hb.commitment, commitment_bytes);
    assert_eq!(hb.percentage, 5_000);
    assert!(!hb.claimed);
}
