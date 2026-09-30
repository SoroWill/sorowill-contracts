#![cfg(test)]

//! Regression coverage for issue #499: `top_up` on a token the will already
//! holds a balance for must accumulate, not create a duplicate entry or
//! overwrite the existing balance.
//!
//! `will.balances` is a `Map<Address, i128>`, so a duplicate *key* is
//! structurally impossible -- `.set()` on an existing key always replaces
//! that key's single value. What this test actually guards is the
//! *value* `top_up` computes before that `.set()`: it must be
//! `existing + amount`, not `amount` alone (which would silently discard
//! the will's prior balance for that token).

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, token_address, contract_id)
}

#[test]
fn topping_up_a_token_the_will_already_holds_accumulates_the_balance() {
    let (env, client, owner, token_address, _contract_id) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
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

    // Top up the exact same token that was already part of create_will's
    // token list -- not a distinct-token addition, a re-deposit.
    client.top_up(&will_id, &owner, &token_address, &50_000);

    let will = client.get_will(&will_id);
    assert_eq!(
        will.balances.get(token_address.clone()),
        Some(150_000),
        "top_up on an existing token must accumulate (100_000 + 50_000), not overwrite"
    );
    assert_eq!(
        will.balances.len(),
        1,
        "topping up an existing token must not add a second entry for it"
    );

    // A second top-up of the same token must keep accumulating.
    client.top_up(&will_id, &owner, &token_address, &25_000);
    let will = client.get_will(&will_id);
    assert_eq!(will.balances.get(token_address), Some(175_000));
}

#[test]
fn topping_up_a_genuinely_new_token_adds_a_second_entry_without_disturbing_the_first() {
    let (env, client, owner, token_address, _contract_id) = setup();
    let beneficiary = Address::generate(&env);
    let sac_2 = env.register_stellar_asset_contract_v2(owner.clone());
    let second_token = sac_2.address();
    StellarAssetClient::new(&env, &second_token).mint(&owner, &1_000_000_000);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
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

    client.top_up(&will_id, &owner, &second_token, &20_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.balances.get(token_address), Some(100_000), "original token's balance must be untouched");
    assert_eq!(will.balances.get(second_token), Some(20_000), "the new token gets its own entry");
    assert_eq!(will.balances.len(), 2);
}
