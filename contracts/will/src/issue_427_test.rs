#![cfg(test)]

//! Regression tests for issue #427: `get_protocol_stats().total_locked_by_token`
//! must stay consistent across will creation, top-up, cancellation, and release.
//!
//! ## Invariant: total_locked_by_token is the running sum of locked funds
//!
//! `storage::adjust_locked_value` is the single write path for the
//! `total_locked_by_token` counter:
//!
//! - **Incremented** by the locked amount at `create_will` time and at `top_up`
//!   time (one call per token).
//! - **Decremented** by the same amount at `cancel_will` time (one call per
//!   token the will held) and at `release_inheritance` / `guardian_trigger`
//!   time (one call per token, via `distribute`).
//!
//! **Assumption: wills are never "lost".** Every will that increments the
//! counter will eventually either be cancelled or released, which decrements it
//! back.  If a will were somehow dropped from storage before reaching a
//! terminal state (e.g. through a future bug or a manual ledger operation
//! outside the contract), its contribution would remain in the counter forever
//! with no repair path.  The `archive_will` entry point, which is the only
//! supported way to remove a will from storage, requires the will to be
//! `Released` or `Cancelled` first — i.e. after the counter has already been
//! decremented.  The counter therefore cannot be inflated by `archive_will`.
//!
//! There is currently no on-chain repair endpoint.  Clients that need to audit
//! the counter against the actual set of live wills must do so off-chain, by
//! cross-referencing the `created` and `released`/`cancelled` event streams.
//!
//! The tests below verify: `sum(created) - sum(cancelled) = stored_total` for
//! a variety of create/cancel patterns, and that `top_up` and `release` also
//! adjust the counter correctly.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

fn locked_for(client: &WillContractClient, token: &Address) -> i128 {
    client
        .get_protocol_stats()
        .total_locked_by_token
        .iter()
        .find(|e| &e.token == token)
        .map(|e| e.total_locked)
        .unwrap_or(0)
}

fn create_will(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
    amount: i128,
) -> u64 {
    let beneficiary = Address::generate(env);
    client.create_will(
        owner,
        &vec![env, (token.clone(), amount)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env],
        &1,
        &None,
        &0,
    )
}

/// `sum(created) - sum(cancelled) = stored_total`.
/// Creates several wills, cancels some, verifies the counter at each step.
#[test]
fn create_minus_cancel_equals_stored_total() {
    let (env, client, owner, token) = setup();

    // Start: counter is 0.
    assert_eq!(locked_for(&client, &token), 0);

    // Create three wills.
    let id1 = create_will(&env, &client, &owner, &token, 100_000);
    assert_eq!(locked_for(&client, &token), 100_000);

    let id2 = create_will(&env, &client, &owner, &token, 250_000);
    assert_eq!(locked_for(&client, &token), 350_000);

    let id3 = create_will(&env, &client, &owner, &token, 50_000);
    assert_eq!(locked_for(&client, &token), 400_000);

    // Cancel the first will.
    client.cancel_will(&id1, &owner);
    assert_eq!(
        locked_for(&client, &token),
        300_000,
        "cancel must decrement the counter"
    );

    // Cancel the third will.
    client.cancel_will(&id3, &owner);
    assert_eq!(
        locked_for(&client, &token),
        250_000,
        "second cancel must decrement the counter again"
    );

    // Cancel the remaining will.
    client.cancel_will(&id2, &owner);
    assert_eq!(
        locked_for(&client, &token),
        0,
        "all wills cancelled: counter must return to zero"
    );
}

/// `top_up` increments the counter; a subsequent cancel decrements it back
/// to zero, confirming that top-ups are tracked symmetrically.
#[test]
fn top_up_then_cancel_returns_counter_to_zero() {
    let (env, client, owner, token) = setup();

    let will_id = create_will(&env, &client, &owner, &token, 100_000);
    assert_eq!(locked_for(&client, &token), 100_000);

    client.top_up(&will_id, &owner, &token, &75_000);
    assert_eq!(locked_for(&client, &token), 175_000);

    client.cancel_will(&will_id, &owner);
    assert_eq!(
        locked_for(&client, &token),
        0,
        "counter must be zero after cancelling a topped-up will"
    );
}

/// Releasing a will (via trigger + release_inheritance) also decrements the
/// counter to zero, just like cancelling does.
#[test]
fn release_decrements_counter_to_zero() {
    let (env, client, owner, token) = setup();

    let will_id = create_will(&env, &client, &owner, &token, 200_000);
    assert_eq!(locked_for(&client, &token), 200_000);

    // Trigger and release.
    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    assert_eq!(
        locked_for(&client, &token),
        0,
        "counter must be zero after releasing a will"
    );
}

/// Two distinct tokens are tracked independently: cancelling one will does
/// not affect the other token's counter.
#[test]
fn two_tokens_tracked_independently() {
    let (env, client, owner, token_a) = setup();

    // Create a second token.
    let owner2 = Address::generate(&env);
    let sac2 = env.register_stellar_asset_contract_v2(owner2.clone());
    let token_b = sac2.address();
    StellarAssetClient::new(&env, &token_b).mint(&owner, &1_000_000_000);

    let id_a = create_will(&env, &client, &owner, &token_a, 300_000);
    let id_b = create_will(&env, &client, &owner, &token_b, 400_000);

    assert_eq!(locked_for(&client, &token_a), 300_000);
    assert_eq!(locked_for(&client, &token_b), 400_000);

    // Cancel the token-a will.
    client.cancel_will(&id_a, &owner);
    assert_eq!(
        locked_for(&client, &token_a),
        0,
        "token_a counter must reach zero"
    );
    assert_eq!(
        locked_for(&client, &token_b),
        400_000,
        "token_b counter must be unaffected"
    );

    // Cancel the token-b will.
    client.cancel_will(&id_b, &owner);
    assert_eq!(locked_for(&client, &token_b), 0);
}
