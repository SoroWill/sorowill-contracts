#![cfg(test)]

//! Regression tests for issues #592-#595.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{
    balance_consistency, Allocation, Beneficiary, WillContract, WillContractClient, WillStatus,
};

const DAY: u64 = 86_400;

/// Registers a fresh Stellar asset token and mints `amount` of it to `to`.
fn new_token(env: &Env, admin: &Address, to: &Address, amount: i128) -> Address {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(to, &amount);
    token_address
}

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let primary = new_token(&env, &owner, &owner, 1_000_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    (env, client, owner, primary)
}

fn one_beneficiary(env: &Env) -> SorobanVec<Beneficiary> {
    vec![
        env,
        Beneficiary {
            address: Address::generate(env),
            allocation: Allocation::Percentage(10_000),
        },
    ]
}

// ---------------------------------------------------------------------------
// #592: balance == balances[primary_token]
// ---------------------------------------------------------------------------

#[test]
fn issue_592_top_ups_keep_mirror_and_map_in_sync() {
    let (env, client, owner, primary) = setup();
    let secondary = new_token(&env, &owner, &owner, 1_000_000_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (primary.clone(), 1_000_i128)];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &one_beneficiary(&env),
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    client.top_up(&will_id, &owner, &secondary, &500);
    let will = client.get_will(&will_id);
    assert!(balance_consistency::is_consistent(&will));
    assert_eq!(will.balance, 1_000);

    client.top_up(&will_id, &owner, &primary, &250);
    let will = client.get_will(&will_id);
    assert!(balance_consistency::is_consistent(&will));
    assert_eq!(will.balance, 1_250);
    assert_eq!(will.balances.get(primary.clone()).unwrap(), 1_250);
    assert_eq!(will.balances.get(secondary.clone()).unwrap(), 500);
}

#[test]
fn issue_592_state_reads_between_top_up_and_checkin_stay_consistent() {
    let (env, client, owner, primary) = setup();
    let secondary = new_token(&env, &owner, &owner, 1_000_000_000);
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (primary.clone(), 2_000_i128)];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &one_beneficiary(&env),
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    client.emergency_checkin(&will_id, &owner);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert!(balance_consistency::is_consistent(&will));

    client.top_up(&will_id, &owner, &secondary, &1);
    assert!(balance_consistency::is_consistent(&client.get_will(&will_id)));
}

#[test]
fn issue_592_sync_heals_a_stale_mirror() {
    let (env, client, owner, primary) = setup();
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (primary, 700_i128)];
    let will_id = client.create_will(
        &owner,
        &tokens,
        &one_beneficiary(&env),
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );
    let mut will = client.get_will(&will_id);
    will.balance = 1;
    assert!(!balance_consistency::is_consistent(&will));
    balance_consistency::sync_primary_balance(&mut will);
    assert!(balance_consistency::is_consistent(&will));
    assert_eq!(will.balance, 700);
}

// ---------------------------------------------------------------------------
// #593: confirmation deadline buffer
// ---------------------------------------------------------------------------

#[test]
fn issue_593_expiry_helper_boundaries() {
    use crate::deadline_precision::{confirmation_expired, CONFIRMATION_DEADLINE_BUFFER_SECONDS};
    let deadline = 1_000_u64;
    assert!(!confirmation_expired(deadline, deadline));
    assert!(!confirmation_expired(deadline + CONFIRMATION_DEADLINE_BUFFER_SECONDS, deadline));
    assert!(confirmation_expired(deadline + CONFIRMATION_DEADLINE_BUFFER_SECONDS + 1, deadline));
    assert!(!confirmation_expired(u64::MAX, u64::MAX));
}

fn pending_will<'a>(
    env: &Env,
    client: &WillContractClient<'a>,
    owner: &Address,
    token: &Address,
    window: u64,
) -> u64 {
    let tokens: SorobanVec<(Address, i128)> = vec![env, (token.clone(), 1_000_i128)];
    client.create_will(
        owner,
        &tokens,
        &one_beneficiary(env),
        &90,
        &7,
        &vec![env],
        &0,
        &None,
        &window,
    )
}

#[test]
fn issue_593_confirm_succeeds_within_buffer_after_short_window() {
    let (env, client, owner, token) = setup();
    // A window near the ledger interval: the owner lands 3 seconds late.
    let will_id = pending_will(&env, &client, &owner, &token, 5);
    env.ledger().with_mut(|l| l.timestamp += 5 + 3);
    client.confirm_will(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

#[test]
fn issue_593_confirm_fails_just_past_buffer() {
    use crate::deadline_precision::CONFIRMATION_DEADLINE_BUFFER_SECONDS;
    let (env, client, owner, token) = setup();
    let will_id = pending_will(&env, &client, &owner, &token, 5);
    env.ledger()
        .with_mut(|l| l.timestamp += 5 + CONFIRMATION_DEADLINE_BUFFER_SECONDS + 1);
    assert!(client.try_confirm_will(&will_id, &owner).is_err());
}

// ---------------------------------------------------------------------------
// #594: a rejected guardian_trigger leaves no partial release behind
// ---------------------------------------------------------------------------

#[test]
fn issue_594_rejected_guardian_trigger_leaves_will_unchanged() {
    use crate::GuardianVoteReason;
    let (env, client, owner, token) = setup();
    let will_id = pending_will(&env, &client, &owner, &token, 0);
    env.ledger()
        .with_mut(|l| l.timestamp += (crate::GUARDIAN_COOLDOWN_DAYS + 1) * DAY);
    let outsider = Address::generate(&env);
    assert!(client
        .try_guardian_trigger(&will_id, &outsider, &GuardianVoteReason::Other)
        .is_err());
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.guardian_votes, 0);
    assert_eq!(will.balance, 1_000);
}

// ---------------------------------------------------------------------------
// #595: keeper bounty rounds down, dust stays with beneficiaries
// ---------------------------------------------------------------------------

#[test]
fn issue_595_bounty_rounds_down_at_boundaries() {
    use crate::bounty_rounding::keeper_bounty;
    // 199 * 50 / 10_000 = 0.995 -> 0
    assert_eq!(keeper_bounty(199, 50), 0);
    // 200 * 50 / 10_000 = 1 exactly
    assert_eq!(keeper_bounty(200, 50), 1);
    // 201 * 50 / 10_000 = 1.005 -> 1
    assert_eq!(keeper_bounty(201, 50), 1);
    // 399 * 50 / 10_000 = 1.995 -> 1 (never rounds up)
    assert_eq!(keeper_bounty(399, 50), 1);
    assert_eq!(keeper_bounty(0, 50), 0);
    assert_eq!(keeper_bounty(10_000, 10_000), 10_000);
}

#[test]
fn issue_595_bounty_never_exceeds_exact_fraction() {
    use crate::bounty_rounding::keeper_bounty;
    let mut balance: i128 = 1;
    while balance < 5_000 {
        let bounty = keeper_bounty(balance, 333);
        // bounty <= balance * bps / 10_000 < bounty + 1
        assert!(bounty * 10_000 <= balance * 333);
        assert!((bounty + 1) * 10_000 > balance * 333);
        balance += 7;
    }
}
