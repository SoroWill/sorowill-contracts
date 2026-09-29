#![cfg(test)]

//! Regression coverage for issues #350-#353:
//!
//! - **#350**: `create_will` rejects duplicate token addresses with a dedicated
//!   `DuplicateToken` error, and derives the legacy `balance` mirror from the
//!   accumulated `balances` map so the two can never disagree.
//! - **#351**: `create_will` / `cancel_will` record the will's real status in
//!   the audit trail instead of a hardcoded `Active`.
//! - **#352**: `confirm_will` and `close_will` append their status transitions
//!   to `get_will_history`.
//! - **#353**: `cancel_will` decrements `total_locked_by_token` for every token
//!   in the will's `balances` map, not just the primary token.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

struct Harness<'a> {
    env: Env,
    client: WillContractClient<'a>,
    owner: Address,
    token_a: Address,
    token_b: Address,
}

fn setup<'a>() -> Harness<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);

    let token_a = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_a).mint(&owner, &1_000_000_000);

    let token_b = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_b).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    Harness {
        env,
        client,
        owner,
        token_a,
        token_b,
    }
}

fn single_percentage_beneficiary(env: &Env, beneficiary: &Address) -> Vec<Beneficiary> {
    vec![
        env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(10_000),
        },
    ]
}

/// The protocol's currently tracked locked total for `token`, or `0` when the
/// token has no entry at all (which is what `adjust_locked_value` produces once
/// a total returns to zero).
fn locked_for(client: &WillContractClient, token: &Address) -> i128 {
    client
        .get_protocol_stats()
        .total_locked_by_token
        .iter()
        .find(|e| &e.token == token)
        .map(|e| e.total_locked)
        .unwrap_or(0)
}

// ── Issue #350: duplicate token addresses ────────────────────────────────────

#[test]
fn create_will_rejects_duplicate_token_addresses() {
    let h = setup();
    let beneficiary = Address::generate(&h.env);

    let result = h.client.try_create_will(
        &h.owner,
        &vec![
            &h.env,
            (h.token_a.clone(), 100_000_i128),
            (h.token_a.clone(), 50_000_i128),
        ],
        &single_percentage_beneficiary(&h.env, &beneficiary),
        &90,
        &7,
        &vec![&h.env],
        &2,
        &None,
        &0,
    );

    assert_eq!(result, Err(Ok(WillError::DuplicateToken.into())));

    // The rejection happens before any transfer, so nothing was pulled from
    // the owner and no will entered the active set.
    assert_eq!(
        TokenClient::new(&h.env, &h.token_a).balance(&h.owner),
        1_000_000_000
    );
    assert_eq!(h.client.get_protocol_stats().active_will_count, 0);
}

#[test]
fn create_will_derives_balance_mirror_from_balances_map() {
    let h = setup();
    let beneficiary = Address::generate(&h.env);

    let will_id = h.client.create_will(
        &h.owner,
        &vec![
            &h.env,
            (h.token_a.clone(), 100_000_i128),
            (h.token_b.clone(), 7_i128),
        ],
        &single_percentage_beneficiary(&h.env, &beneficiary),
        &90,
        &7,
        &vec![&h.env],
        &2,
        &None,
        &0,
    );

    let will = h.client.get_will(&will_id);
    assert_eq!(will.token, h.token_a);
    assert_eq!(will.balance, will.balances.get(h.token_a.clone()).unwrap());
    assert_eq!(will.balance, 100_000);
}

// ── Issue #353: multi-token cancellation and locked-value totals ─────────────

#[test]
fn cancel_will_zeroes_locked_value_for_every_token() {
    let h = setup();
    let beneficiary = Address::generate(&h.env);

    let will_id = h.client.create_will(
        &h.owner,
        &vec![
            &h.env,
            (h.token_a.clone(), 100_000_i128),
            (h.token_b.clone(), 250_000_i128),
        ],
        &single_percentage_beneficiary(&h.env, &beneficiary),
        &90,
        &7,
        &vec![&h.env],
        &2,
        &None,
        &0,
    );

    assert_eq!(locked_for(&h.client, &h.token_a), 100_000);
    assert_eq!(locked_for(&h.client, &h.token_b), 250_000);

    h.client.cancel_will(&will_id, &h.owner);

    assert_eq!(locked_for(&h.client, &h.token_a), 0);
    assert_eq!(locked_for(&h.client, &h.token_b), 0);

    // Both tokens really were refunded, not just the primary one.
    assert_eq!(
        TokenClient::new(&h.env, &h.token_a).balance(&h.owner),
        1_000_000_000
    );
    assert_eq!(
        TokenClient::new(&h.env, &h.token_b).balance(&h.owner),
        1_000_000_000
    );
}

// ── Issue #351: real from/to status in the audit trail ───────────────────────

#[test]
fn history_records_pending_confirmation_creation_and_cancellation() {
    let h = setup();
    let beneficiary = Address::generate(&h.env);

    let will_id = h.client.create_will(
        &h.owner,
        &vec![&h.env, (h.token_a.clone(), 100_000_i128)],
        &single_percentage_beneficiary(&h.env, &beneficiary),
        &90,
        &7,
        &vec![&h.env],
        &2,
        &None,
        &(30 * DAY),
    );
    assert_eq!(
        h.client.get_will(&will_id).status,
        WillStatus::PendingConfirmation
    );

    // Creation must record the real initial status, not a hardcoded `Active`.
    let history = h.client.get_will_history(&will_id);
    assert_eq!(history.len(), 1);
    let create = history.get(0).unwrap();
    assert_eq!(create.from_status, WillStatus::PendingConfirmation);
    assert_eq!(create.to_status, WillStatus::PendingConfirmation);
    assert_eq!(create.action, symbol_short!("create"));

    // Cancelling while still pending must record `PendingConfirmation` as the
    // from-status, matching the state the will was actually in.
    h.client.cancel_will(&will_id, &h.owner);

    let history = h.client.get_will_history(&will_id);
    assert_eq!(history.len(), 2);
    let cancel = history.get(1).unwrap();
    assert_eq!(cancel.from_status, WillStatus::PendingConfirmation);
    assert_eq!(cancel.to_status, WillStatus::Cancelled);
    assert_eq!(cancel.action, symbol_short!("cancel"));
}

// ── Issue #352: confirm_will / close_will audit entries ──────────────────────

#[test]
fn confirm_will_records_pending_to_active_transition() {
    let h = setup();
    let beneficiary = Address::generate(&h.env);

    let will_id = h.client.create_will(
        &h.owner,
        &vec![&h.env, (h.token_a.clone(), 100_000_i128)],
        &single_percentage_beneficiary(&h.env, &beneficiary),
        &90,
        &7,
        &vec![&h.env],
        &2,
        &None,
        &(30 * DAY),
    );

    h.client.confirm_will(&will_id, &h.owner);

    let history = h.client.get_will_history(&will_id);
    assert_eq!(history.len(), 2);
    let confirm = history.get(1).unwrap();
    assert_eq!(confirm.from_status, WillStatus::PendingConfirmation);
    assert_eq!(confirm.to_status, WillStatus::Active);
    assert_eq!(confirm.actor, h.owner);
    assert_eq!(confirm.action, symbol_short!("confirm"));
}

#[test]
fn close_will_records_released_to_settled_transition() {
    let h = setup();
    let beneficiary = Address::generate(&h.env);

    let will_id = h.client.create_will(
        &h.owner,
        &vec![&h.env, (h.token_a.clone(), 1_000_000_i128)],
        &single_percentage_beneficiary(&h.env, &beneficiary),
        &90,
        &7,
        &vec![&h.env],
        &2,
        &None,
        &0,
    );

    h.env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    h.client.trigger_will(&will_id);
    h.env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    h.client.release_inheritance(&will_id, &None);
    h.client.close_will(&will_id, &h.owner);

    let history = h.client.get_will_history(&will_id);
    let close = history.get(history.len() - 1).unwrap();
    assert_eq!(close.from_status, WillStatus::Released);
    assert_eq!(close.to_status, WillStatus::Settled);
    assert_eq!(close.actor, h.owner);
    assert_eq!(close.action, symbol_short!("close"));
}
