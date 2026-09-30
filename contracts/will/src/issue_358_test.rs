#![cfg(test)]

//! Regression coverage for issue #358: `top_up` must not grow a will past
//! `MAX_TOKENS` distinct tokens.
//!
//! `create_will` rejects a `tokens` list longer than `MAX_TOKENS`, but
//! `top_up` accepted any new token address. `distribute` and `cancel_will`
//! iterate every entry of `will.balances`, so the map could be grown one token
//! at a time until a release or refund no longer fits in a transaction and the
//! will's funds become unreachable.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::{errors::WillError, Allocation, Beneficiary, WillContract, WillContractClient};

/// `MAX_TOKENS` in `lib.rs`: a will may hold at most this many distinct tokens.
const MAX_TOKENS: u32 = 10;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner)
}

/// Registers a fresh SEP-41 token and mints `amount` of it to `owner`.
fn new_token(env: &Env, owner: &Address, amount: i128) -> Address {
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let address = sac.address();
    StellarAssetClient::new(env, &address).mint(owner, &amount);
    address
}

/// Creates a will holding `MAX_TOKENS` distinct tokens of `100` units each and
/// returns its id together with the token addresses in `balances` order.
fn will_at_token_cap<'a>(
    env: &Env,
    client: &WillContractClient<'a>,
    owner: &Address,
) -> (u64, Vec<Address>) {
    let mut addresses: Vec<Address> = Vec::new(env);
    let mut tokens: Vec<(Address, i128)> = Vec::new(env);
    for _ in 0..MAX_TOKENS {
        let address = new_token(env, owner, 1_000);
        tokens.push_back((address.clone(), 100_i128));
        addresses.push_back(address);
    }

    let will_id = client.create_will(
        owner,
        &tokens,
        &vec![
            env,
            Beneficiary {
                address: Address::generate(env),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );

    assert_eq!(client.get_will(&will_id).balances.len(), MAX_TOKENS);
    (will_id, addresses)
}

#[test]
fn top_up_rejects_an_eleventh_distinct_token() {
    let (env, client, owner) = setup();
    let (will_id, addresses) = will_at_token_cap(&env, &client, &owner);

    // One more distinct token — the will is already at `MAX_TOKENS`.
    let extra = new_token(&env, &owner, 1_000);

    assert_eq!(
        client.try_top_up(&will_id, &owner, &extra, &10),
        Err(Ok(WillError::InvalidTokenCount.into())),
    );

    // The rejected call must not have mutated the will: the map still holds
    // exactly the original tokens, and none of them changed.
    let will = client.get_will(&will_id);
    assert_eq!(will.balances.len(), MAX_TOKENS);
    assert!(will.balances.get(extra).is_none());
    for address in addresses.iter() {
        assert_eq!(will.balances.get(address).unwrap(), 100);
    }
}

#[test]
fn top_up_of_an_existing_token_still_works_at_the_cap() {
    let (env, client, owner) = setup();
    let (will_id, addresses) = will_at_token_cap(&env, &client, &owner);
    let existing = addresses.get_unchecked(0);

    // Topping up a token the will already holds does not grow the map, so it
    // stays allowed at the cap.
    client.top_up(&will_id, &owner, &existing, &50);

    let will = client.get_will(&will_id);
    assert_eq!(will.balances.len(), MAX_TOKENS);
    assert_eq!(will.balances.get(existing).unwrap(), 150);
}
