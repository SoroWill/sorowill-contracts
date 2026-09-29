#![cfg(test)]

//! Regression tests for issue #426: `distribute` must produce consistent
//! results regardless of the order in which `Allocation::Percentage` and
//! `Allocation::FixedAmount` beneficiaries appear in the list.
//!
//! The `distribute` function always processes `FixedAmount` entries first
//! (for the primary token), then splits whatever remains among `Percentage`
//! beneficiaries.  This ordering is enforced in code, not delegated to the
//! caller-supplied list order, so a will with `[Percentage, FixedAmount]`
//! produces the same payouts as one with `[FixedAmount, Percentage]`.
//!
//! These tests document that invariant and assert it across both orderings.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
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

    (
        env.clone(),
        client,
        owner,
        TokenClient::new(&env, &token_address),
        token_address,
    )
}

fn release(env: &Env, client: &WillContractClient, will_id: u64) {
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);
}

/// `[FixedAmount, Percentage]` ordering — the "natural" order where the fixed
/// payout appears first in the list.
#[test]
fn fixed_first_then_percentage_distributes_correctly() {
    let (env, client, owner, token, token_address) = setup();
    let fixed_beneficiary = Address::generate(&env);
    let pct_beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: fixed_beneficiary.clone(),
                allocation: Allocation::FixedAmount(200_000),
            },
            Beneficiary {
                address: pct_beneficiary.clone(),
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

    release(&env, &client, will_id);

    assert_eq!(
        token.balance(&fixed_beneficiary),
        200_000,
        "fixed beneficiary (first) must receive their exact amount"
    );
    assert_eq!(
        token.balance(&pct_beneficiary),
        800_000,
        "percentage beneficiary (second) must receive the remainder"
    );
    assert_eq!(token.balance(&client.address), 0);
}

/// `[Percentage, FixedAmount]` ordering — the fixed beneficiary comes last in
/// the list.  The payouts must be identical to the `[FixedAmount, Percentage]`
/// case above, because `distribute` always pays fixed amounts before
/// percentages regardless of list order.
#[test]
fn percentage_first_then_fixed_distributes_consistently() {
    let (env, client, owner, token, token_address) = setup();
    let pct_beneficiary = Address::generate(&env);
    let fixed_beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            // Percentage listed FIRST this time.
            Beneficiary {
                address: pct_beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
            // FixedAmount listed SECOND.
            Beneficiary {
                address: fixed_beneficiary.clone(),
                allocation: Allocation::FixedAmount(200_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    release(&env, &client, will_id);

    // The fixed beneficiary must still receive exactly 200_000, and the
    // percentage beneficiary must still receive the remaining 800_000 — the
    // same split as when the order is reversed.
    assert_eq!(
        token.balance(&fixed_beneficiary),
        200_000,
        "fixed beneficiary (second) must receive their exact amount regardless of list order"
    );
    assert_eq!(
        token.balance(&pct_beneficiary),
        800_000,
        "percentage beneficiary (first) must receive the remainder regardless of list order"
    );
    assert_eq!(token.balance(&client.address), 0);
}

/// Multiple fixed-amount beneficiaries listed after one percentage-based
/// beneficiary.  The sum of all fixed amounts is subtracted from the balance
/// before any percentage split occurs, even though the fixed entries come last.
#[test]
fn multiple_fixed_after_percentage_distributes_correctly() {
    let (env, client, owner, token, token_address) = setup();
    let pct_beneficiary = Address::generate(&env);
    let fixed_a = Address::generate(&env);
    let fixed_b = Address::generate(&env);

    // Total: 1_000_000. Fixed: 150_000 + 250_000 = 400_000.
    // Percentage remainder: 600_000.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: pct_beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
            Beneficiary {
                address: fixed_a.clone(),
                allocation: Allocation::FixedAmount(150_000),
            },
            Beneficiary {
                address: fixed_b.clone(),
                allocation: Allocation::FixedAmount(250_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    release(&env, &client, will_id);

    assert_eq!(token.balance(&fixed_a), 150_000);
    assert_eq!(token.balance(&fixed_b), 250_000);
    assert_eq!(
        token.balance(&pct_beneficiary),
        600_000,
        "percentage beneficiary gets the remainder after both fixed amounts are deducted"
    );
    assert_eq!(token.balance(&client.address), 0);
}
