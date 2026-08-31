#![cfg(test)]

//! Regression test for issue #299: `top_up` supports adding a token that was
//! not locked at `create_will` time (new-token path via `will.balances.get(token).unwrap_or(0)`).
//!
//! Acceptance criteria (from the issue):
//! 1. A will is created with a single token (token_a).
//! 2. `top_up` is called with a second, distinct token (token_b) never seen by
//!    that will before.
//! 3. `get_will` reflects both token balances.
//! 4. A subsequent `release_inheritance` correctly distributes both tokens to
//!    the beneficiary.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;

#[test]
fn top_up_with_new_token_is_reflected_in_get_will_and_released() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);

    // Set up token_a (the original will token).
    let sac_a = env.register_stellar_asset_contract_v2(owner.clone());
    let token_a_address = sac_a.address();
    StellarAssetClient::new(&env, &token_a_address).mint(&owner, &1_000_000);
    let token_a = TokenClient::new(&env, &token_a_address);

    // Set up token_b (a brand-new token, not present at create_will).
    let sac_b = env.register_stellar_asset_contract_v2(owner.clone());
    let token_b_address = sac_b.address();
    StellarAssetClient::new(&env, &token_b_address).mint(&owner, &500_000);
    let token_b = TokenClient::new(&env, &token_b_address);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Step 1: create a will locked only with token_a.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_a_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // Step 2: top up with token_b (never seen by this will before).
    client.top_up(&will_id, &owner, &token_b_address, &500_000);

    // Step 3: get_will must reflect both token balances.
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(
        will.balances.get(token_a_address.clone()).unwrap(),
        1_000_000_i128,
        "token_a balance must remain 1_000_000 after top_up with token_b"
    );
    assert_eq!(
        will.balances.get(token_b_address.clone()).unwrap(),
        500_000_i128,
        "token_b balance must be 500_000 after top_up"
    );

    // Step 4: release_inheritance must distribute both tokens to the beneficiary.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    assert_eq!(
        token_a.balance(&beneficiary),
        1_000_000,
        "beneficiary must receive the full token_a balance"
    );
    assert_eq!(
        token_b.balance(&beneficiary),
        500_000,
        "beneficiary must receive the full token_b balance"
    );
    // Contract must hold no residual balance of either token.
    assert_eq!(token_a.balance(&client.address), 0);
    assert_eq!(token_b.balance(&client.address), 0);

    let released_will = client.get_will(&will_id);
    assert_eq!(released_will.status, WillStatus::Released);
}
