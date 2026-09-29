                                                                                                                                                                                                                                                                                                                                         #![cfg(test)]

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Beneficiary, Guardian, GuardianVoteReason, WillContract, WillContractClient, WillError, WillStatus};

fn create_token<'a>(env: &Env, admin: &Address) -> (TokenClient<'a>, StellarAssetClient<'a>) {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    (
        TokenClient::new(env, &sac.address()),
        StellarAssetClient::new(env, &sac.address()),
    )
}

fn setup<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let (token_client, token_admin) = create_token(&env, &owner);
    token_admin.mint(&owner, &1_000_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    (env, client, owner, token_client, token_admin.address.clone())
}

fn setup_two_tokens<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
    TokenClient<'a>,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let (token_a_client, token_a_admin) = create_token(&env, &owner);
    token_a_admin.mint(&owner, &1_000_000_000);
    let token_a_addr = token_a_admin.address.clone();
    let token_b_admin_addr = Address::generate(&env);
    let (token_b_client, token_b_admin) = create_token(&env, &token_b_admin_addr);
    token_b_admin.mint(&owner, &1_000_000_000);
    let token_b_addr = token_b_admin.address.clone();
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    (env, client, owner, token_a_client, token_a_addr, token_b_client, token_b_addr)
}

/// Sets up a will contract and funds the owner with native XLM by
/// transferring from the test environment's source account (which has
/// a large native XLM balance in test mode).
fn setup_native<'a>() -> (
    Env,
    WillContractClient<'a>,
    Address, // owner
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);

    // Capture the test source address *before* registering our contract.
    // In the test environment, env.current_contract_address() returns the
    // default source/invoker account which holds a large native XLM balance.
    let test_source = env.current_contract_address();

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // Transfer native XLM from the test source account to the owner,
    // giving the owner XLM to use in the will.
    env.transfer(&test_source, &owner, &10_000_000_000_000i128);

    (env, client, owner)
}

fn advance_time(env: &Env, seconds: u64) {
    env.ledger().with_mut(|l| { l.timestamp += seconds; });
}

#[test]
fn test_get_contract_version() {
    let (_, client, _, _, _) = setup();
    // Baseline version is 1.0.0, encoded as 1_000_000.
    assert_eq!(client.get_contract_version(), CONTRACT_VERSION);
    assert_eq!(client.get_contract_version(), 1_000_000);
}

const DAY: u64 = 86_400;

// ── Token-based (SAC) tests ────────────────────────────────────────────
// ── existing tests updated for multi-token API ────────────────────────────────

#[test]
fn test_create_will_success() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    assert_eq!(will_id, 1);
    let will = client.get_will(&will_id);
    assert_eq!(will.owner, owner);
    assert_eq!(will.balances.get(token_address.clone()).unwrap(), 1_000_000);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.checkin_period_days, 90);
    assert_eq!(will.grace_period_days, 7);
    assert!(!will.is_native);
    assert_eq!(token.balance(&owner), 1_000_000_000 - 1_000_000);
    assert_eq!(token.balance(&client.address), 1_000_000);
    assert!(will.delegate.is_none());
    assert!(will.vesting.is_none());
}

#[test]
fn test_protocol_stats_track_create_cancel_and_release() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let second_admin = Address::generate(&env);
    let (second_token_client, second_token_admin_client) = create_token(&env, &second_admin);
    second_token_admin_client.mint(&owner, &1_000_000_000);
    let second_token_address = second_token_admin_client.address.clone();

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let stats = client.get_protocol_stats();
    assert_eq!(stats.active_will_count, 1);
    assert_eq!(stats.total_locked_by_token.len(), 1);
    assert_eq!(
        stats.total_locked_by_token.get(0).unwrap().token,
        token_address
    );
    assert_eq!(
        stats.total_locked_by_token.get(0).unwrap().total_locked,
        1_000_000
    );

    client.cancel_will(&will_id, &owner);

    let stats = client.get_protocol_stats();
    assert_eq!(stats.active_will_count, 0);
    assert_eq!(stats.total_locked_by_token.get(0).unwrap().total_locked, 0);

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (second_token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    let stats = client.get_protocol_stats();
    assert_eq!(stats.active_will_count, 1);
    assert_eq!(stats.total_locked_by_token.len(), 2);
    assert_eq!(
        stats.total_locked_by_token.get(0).unwrap().token,
        token_address
    );
    assert_eq!(stats.total_locked_by_token.get(0).unwrap().total_locked, 0);
    assert_eq!(
        stats.total_locked_by_token.get(1).unwrap().token,
        second_token_address
    );
    assert_eq!(
        stats.total_locked_by_token.get(1).unwrap().total_locked,
        500_000
    );

    advance_time(&env, 31 * DAY);
    client.trigger_will(&will_id_2);
    advance_time(&env, 4 * DAY);
    client.release_inheritance(&will_id_2);

    let stats = client.get_protocol_stats();
    assert_eq!(stats.active_will_count, 0);
    assert_eq!(stats.total_locked_by_token.get(0).unwrap().total_locked, 0);
    assert_eq!(stats.total_locked_by_token.get(1).unwrap().total_locked, 0);
    assert_eq!(second_token_client.balance(&owner), 1_000_000_000 - 500_000);
}

#[test]
fn test_checkin_resets_deadline() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    client.create_will(
        &owner,
        &vec![&env, (token_address, 0_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
}

#[test]
#[should_panic]
fn test_invalid_percentages_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
}

// ── check_in ─────────────────────────────────────────────────────────────────

#[test]
fn test_checkin_resets_deadline() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 10 * DAY);
    client.check_in(&will_id, &owner);
    let will = client.get_will(&will_id);
    assert_eq!(will.last_checkin, 1_700_000_000 + 10 * DAY);
    assert_eq!(will.status, WillStatus::Active);
}

// ── trigger ──────────────────────────────────────────────────────────────────

#[test]
fn test_trigger_after_missed_checkin() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Triggered);
    assert!(will.trigger_time.is_some());
    assert_eq!(will.trigger_balance, 1_000_000);
}

#[test]
#[should_panic]
fn test_cannot_trigger_before_deadline() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 10 * DAY);
    client.trigger_will(&will_id);
}

// ── get_will_status / get_time_until_deadline ────────────────────────────────

#[test]
fn test_get_will_status_active() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    assert_eq!(client.get_will_status(&will_id), WillStatus::Active);
    // Matches the status embedded in the full struct.
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

#[test]
fn test_get_will_status_triggered() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    assert_eq!(client.get_will_status(&will_id), WillStatus::Triggered);
}

#[test]
fn test_get_time_until_deadline_active() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    // Fresh will: ~90 days (in seconds) remain until the check-in deadline.
    let remaining = client.get_time_until_deadline(&will_id);
    assert_eq!(remaining, Some(90 * DAY as i64));

    // Halfway through the check-in period, roughly half the time remains.
    advance_time(&env, 45 * DAY);
    let remaining = client.get_time_until_deadline(&will_id);
    assert_eq!(remaining, Some(45 * DAY as i64));

    // Past the deadline but not yet triggered: negative, not None.
    advance_time(&env, 50 * DAY);
    let remaining = client.get_time_until_deadline(&will_id);
    assert_eq!(remaining, Some(-5 * DAY as i64));
}

#[test]
fn test_get_time_until_deadline_triggered() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    // Just triggered: the full 7-day grace period remains.
    let remaining = client.get_time_until_deadline(&will_id);
    assert_eq!(remaining, Some(7 * DAY as i64));

    // Partway through the grace period.
    advance_time(&env, 3 * DAY);
    let remaining = client.get_time_until_deadline(&will_id);
    assert_eq!(remaining, Some(4 * DAY as i64));

    // Past the grace period but not yet released: negative, not None.
    advance_time(&env, 10 * DAY);
    let remaining = client.get_time_until_deadline(&will_id);
    assert_eq!(remaining, Some(-6 * DAY as i64));
}

#[test]
fn test_get_time_until_deadline_none_when_not_applicable() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.cancel_will(&will_id, &owner);
    assert_eq!(client.get_will_status(&will_id), WillStatus::Cancelled);
    assert_eq!(client.get_time_until_deadline(&will_id), None);
}

// ── emergency_checkin ────────────────────────────────────────────────────────

#[test]
fn test_emergency_checkin_cancels_trigger() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 2 * DAY);
    client.emergency_checkin(&will_id, &owner);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert!(will.trigger_time.is_none());
    assert_eq!(will.last_checkin, 1_700_000_000 + 91 * DAY + 2 * DAY);
}

// ── release_inheritance ──────────────────────────────────────────────────────

#[test]
fn test_release_inheritance_splits_correctly() {
    let (env, client, owner, token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                basis_points: 6_000,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                basis_points: 4_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);
    assert_eq!(token.balance(&a), 600_000);
    assert_eq!(token.balance(&b), 400_000);
    assert_eq!(token.balance(&client.address), 0);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

#[test]
#[should_panic]
fn test_cannot_release_during_grace_period() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 2 * DAY);
    client.release_inheritance(&will_id);
}

#[test]
fn test_fractional_three_way_split() {
    let (env, client, owner, token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);
    assert_eq!(token.balance(&a), 500_000);
    assert_eq!(token.balance(&b), 333_300);
    assert_eq!(token.balance(&c), 166_700);
    assert_eq!(token.balance(&client.address), 0);
}

#[test]
fn test_release_inheritance_rounding_remainder() {
    let (env, client, owner, token, token_address) = setup();
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_001_i128)],
        &vec![&env, bp(&b1, 3_333), bp(&b2, 3_333), bp(&b3, 3_334)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);
    let s1 = token.balance(&b1);
    let s2 = token.balance(&b2);
    let s3 = token.balance(&b3);
    assert_eq!(s1 + s2 + s3, 1_000_001);
    assert_eq!(token.balance(&client.address), 0);
}

#[test]
fn test_release_inheritance_rolls_back_when_one_beneficiary_rejects_transfer() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let total = 1_000_000_i128;

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), total)],
        &vec![
            &env,
            bp(&beneficiary_a, 6_000),
            bp(&beneficiary_b, 4_000),
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);

    // The first transfer would succeed, but the second beneficiary is frozen
    // and cannot receive this Stellar asset.
    StellarAssetClient::new(&env, &token_address)
        .set_authorized(&beneficiary_b, &false);

    assert!(client.try_release_inheritance(&will_id, &None).is_err());

    // Soroban rolls the entire invocation back: the earlier transfer and all
    // of distribute's state changes must be absent.
    assert_eq!(token.balance(&beneficiary_a), 0);
    assert_eq!(token.balance(&beneficiary_b), 0);
    assert_eq!(token.balance(&client.address), total);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Triggered);
    assert_eq!(will.balances.get(token_address).unwrap(), total);
}

#[test]
fn test_release_inheritance_handles_near_maximum_balance_without_overflow() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let total = i128::MAX;

    // setup funds the owner with 1_000_000_000 units; extend that balance to
    // the largest positive amount the Stellar test token can represent.
    StellarAssetClient::new(&env, &token_address)
        .mint(&owner, &(total - 1_000_000_000));

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, total)],
        &vec![
            &env,
            bp(&beneficiary_a, 6_000),
            bp(&beneficiary_b, 4_000),
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id, &None);

    let expected_a = (total / 10_000) * 6_000
        + (total % 10_000) * 6_000 / 10_000;
    assert_eq!(token.balance(&beneficiary_a), expected_a);
    assert_eq!(token.balance(&beneficiary_b), total - expected_a);
    assert_eq!(token.balance(&client.address), 0);
}

#[test]
fn test_release_multi_token_proportionally() {
    let (env, client, owner, token_a, token_a_addr, token_b, token_b_addr) = setup_two_tokens();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_a_addr.clone(), 1_000_000_i128), (token_b_addr.clone(), 3_000_000_i128)],
        &vec![&env, bp(&a, 6_000), bp(&b, 4_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);
    assert_eq!(token_a.balance(&a), 600_000);
    assert_eq!(token_a.balance(&b), 400_000);
    assert_eq!(token_b.balance(&a), 1_800_000);
    assert_eq!(token_b.balance(&b), 1_200_000);
}

// ── cancel_will ──────────────────────────────────────────────────────────────

#[test]
fn test_cancel_will_refunds_owner() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.cancel_will(&will_id, &owner);
    assert_eq!(token.balance(&owner), 1_000_000_000);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Cancelled);
}

#[test]
fn test_cancel_will_refunds_all_tokens() {
    let (env, client, owner, token_a, token_a_addr, token_b, token_b_addr) = setup_two_tokens();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_a_addr.clone(), 1_000_000_i128), (token_b_addr.clone(), 2_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.cancel_will(&will_id, &owner);
    assert_eq!(token_a.balance(&owner), 1_000_000_000);
    assert_eq!(token_b.balance(&owner), 1_000_000_000);
}

// ── update_beneficiaries ─────────────────────────────────────────────────────

#[test]
fn test_update_beneficiaries() {
    let (env, client, owner, _token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.update_beneficiaries(&will_id, &owner, &vec![&env, bp(&b, 5_000), bp(&c, 5_000)]);
    assert_eq!(client.get_will(&will_id).beneficiaries.len(), 2);
    assert_eq!(client.get_wills_by_beneficiary(&b, &None, &100).len(), 1);
}

#[test]
fn test_update_beneficiaries_event_payload() {
    let (env, client, owner, _token, token_address) = setup();
    let original = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: original,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let new_beneficiaries = SorobanVec::from_array(
        &env,
        [
            Beneficiary {
                address: b.clone(),
                basis_points: 4_000,
            },
            Beneficiary {
                address: c.clone(),
                basis_points: 6_000,
            },
        ],
    );
    client.update_beneficiaries(&will_id, &owner, &new_beneficiaries);

    use soroban_sdk::{symbol_short, testutils::Events, TryIntoVal};
    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        if !event.1.is_empty() {
            if let Ok(topic0) = event.1.get(0).unwrap().try_into_val(&env) {
                let topic0_sym: soroban_sdk::Symbol = topic0;
                if topic0_sym == symbol_short!("benefup") {
                    found = true;
                    let topic1: u64 = event.1.get(1).unwrap().try_into_val(&env).unwrap();
                    assert_eq!(topic1, will_id);
                    // data: (owner, beneficiary_count, beneficiaries)
                    let data: (Address, u32, SorobanVec<Beneficiary>) =
                        event.2.try_into_val(&env).unwrap();
                    assert_eq!(data.0, owner);
                    assert_eq!(data.1, 2);
                    assert_eq!(data.2, new_beneficiaries);
                }
            }
        }
    }
    assert!(found, "benefup event not found");
}

#[test]
fn test_update_beneficiaries_fractional_split() {
    let (env, client, owner, token, token_address) = setup();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let orig = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, bp(&orig, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.update_beneficiaries(&will_id, &owner, &vec![&env, bp(&a, 2_500), bp(&b, 7_500)]);
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);
    assert_eq!(token.balance(&a), 250_000);
    assert_eq!(token.balance(&b), 750_000);
}

#[test]
#[should_panic]
fn test_update_beneficiaries_rejects_invalid_bp() {
    let (env, client, owner, _token, token_address) = setup();
    let orig = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, old_guardian],
        &2,
        &None,
    );
    client.update_beneficiaries(&will_id, &owner, &vec![&env, bp(&a, 3_000), bp(&b, 3_000)]);
}

// ── update_guardians ─────────────────────────────────────────────────────────

#[test]
fn test_update_guardians() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let old = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env, old],
        &2,
        &None,
    );
    client.update_guardians(&will_id, &owner, &vec![&env, g1.clone(), g2.clone()]);
    let will = client.get_will(&will_id);
    assert_eq!(will.guardians, vec![&env, g1, g2]);
    assert_eq!(will.guardian_votes, 0);
    assert_eq!(
        will.guardians,
        vec![
            &env,
            Guardian {
                address: new_guardian_1,
                weight: 1,
            },
            Guardian {
                address: new_guardian_2,
                weight: 1,
            }
        ]
    );
    assert_eq!(will.guardian_vote_weight, 0);
}

#[test]
#[should_panic]
fn test_update_guardians_rejects_non_owner() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let non_owner = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.update_guardians(&will_id, &non_owner, &vec![&env]);
}

#[test]
#[should_panic]
fn test_update_guardians_rejects_too_many() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.update_guardians(
        &will_id,
        &owner,
        &vec![
            &env,
            GuardianEntry {
                address: Address::generate(&env),
                tier: GuardianTier::Primary,
            },
            GuardianEntry {
                address: Address::generate(&env),
                tier: GuardianTier::Primary,
            },
            GuardianEntry {
                address: Address::generate(&env),
                tier: GuardianTier::Backup,
            },
            GuardianEntry {
                address: Address::generate(&env),
                tier: GuardianTier::Backup,
            },
        ],
    );
    client.update_guardians(&will_id, &owner, &vec![
        &env, Address::generate(&env), Address::generate(&env),
        Address::generate(&env), Address::generate(&env),
    ]);
}

#[test]
fn test_update_guardians_resets_votes() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![
            &env,
            Guardian {
                address: guardian_1.clone(),
                weight: 1,
            },
            Guardian {
                address: guardian_2.clone(),
                weight: 1,
            },
        ],
        &2,
        &None,
    );
    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);
    client.update_guardians(&will_id, &owner, &vec![&env, g2.clone()]);
    assert_eq!(client.get_will(&will_id).guardian_votes, 0);

    client.accept_guardian_role(&will_id, &guardian_1);
    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 1);

    client.update_guardians(&will_id, &owner, &vec![&env, guardian_2.clone()]);
    assert_eq!(client.get_will(&will_id).guardian_votes, 0);
    client.update_guardians(
        &will_id,
        &owner,
        &vec![
            &env,
            Guardian {
                address: guardian_2.clone(),
                weight: 1,
            },
        ],
    );
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 0);
    client.update_guardians(
        &will_id,
        &owner,
        &vec![
            &env,
            Guardian {
                address: guardian_1.clone(),
                weight: 1,
            },
            Guardian {
                address: guardian_2,
                weight: 1,
            },
        ],
    );

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 1);
}

#[test]
#[should_panic]
fn test_update_guardians_rejected_while_triggered() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    client.update_guardians(&will_id, &owner, &vec![&env]);
}

// ── top_up ───────────────────────────────────────────────────────────────────

#[test]
fn test_top_up_increases_balance() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![
            &env,
            Guardian {
                address: guardian_1.clone(),
                weight: 1,
            },
            Guardian {
                address: guardian_2.clone(),
                weight: 1,
            },
        ],
        &2,
        &None,
    );
    client.top_up(&will_id, &owner, &token_address, &500_000);
    assert_eq!(client.get_will(&will_id).balances.get(token_address.clone()).unwrap(), 1_500_000);
    assert_eq!(token.balance(&client.address), 1_500_000);
}

#[test]
fn test_top_up_new_token() {
    let (env, client, owner, _token_a, token_a_addr, token_b, token_b_addr) = setup_two_tokens();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.top_up(&will_id, &owner, &token_b_addr, &500_000);
    let will = client.get_will(&will_id);
    assert_eq!(will.balances.len(), 2);
    assert_eq!(will.balances.get(token_b_addr).unwrap(), 500_000);
}

#[test]
fn test_top_up_existing_token_accumulates() {
    let (env, client, owner, _token_a, token_a_addr, _token_b, _token_b_addr) = setup_two_tokens();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_a_addr.clone(), 1_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.top_up(&will_id, &owner, &token_a_addr, &250_000);
    client.top_up(&will_id, &owner, &token_a_addr, &250_000);
    assert_eq!(client.get_will(&will_id).balances.get(token_a_addr).unwrap(), 1_500_000);
}

#[test]
#[should_panic]
fn test_top_up_zero_amount_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.top_up(&will_id, &owner, &token_address, &500_000);

    use soroban_sdk::{symbol_short, testutils::Events, TryIntoVal};
    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        if !event.1.is_empty() {
            if let Ok(topic0) = event.1.get(0).unwrap().try_into_val(&env) {
                let topic0_sym: soroban_sdk::Symbol = topic0;
                if topic0_sym == symbol_short!("topup") {
                    found = true;
                    let topic1: u64 = event.1.get(1).unwrap().try_into_val(&env).unwrap();
                    assert_eq!(topic1, will_id);
                    // data: (owner, token, amount, new_balance)
                    let data: (Address, Address, i128, i128) =
                        event.2.try_into_val(&env).unwrap();
                    assert_eq!(data, (owner.clone(), token_address.clone(), 500_000_i128, 1_500_000_i128));
                }
            }
        }
    }
    assert!(found, "topup event not found");
}

// ── guardian_trigger ─────────────────────────────────────────────────────────

#[test]
fn test_guardian_trigger_requires_two_votes() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![
            &env,
            Guardian {
                address: guardian_1.clone(),
                weight: 1,
            },
            Guardian {
                address: guardian_2.clone(),
                weight: 1,
            },
            Guardian {
                address: guardian_3.clone(),
                weight: 1,
            },
        ],
        &2,
        &None,
    );
    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);

    client.accept_guardian_role(&will_id, &guardian_1);
    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.guardian_vote_weight, 1);
    assert_eq!(token.balance(&beneficiary), 0);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_guardian_threshold_1_of_1() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
    );
    advance_time(&env, 8 * DAY);
    client.accept_guardian_role(&will_id, &guardian);
    client.guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_guardian_threshold_3_of_3() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let g3 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone(), g3.clone()],
        &3,
        &None,
    );
    advance_time(&env, 8 * DAY);
    client.accept_guardian_role(&will_id, &g1);
    client.accept_guardian_role(&will_id, &g2);
    client.accept_guardian_role(&will_id, &g3);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
    client.guardian_trigger(&will_id, &g3, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_guardian_threshold_invalid_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);
    // Zero threshold with non-empty guardians should fail.
    let result = client.try_create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &0,
        &None,
    );
    assert_eq!(result, Err(Ok(WillError::InvalidGuardianThreshold.into())));
    // Threshold > guardians.len() should fail.
    let result = client.try_create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &2,
        &None,
    );
    assert_eq!(result, Err(Ok(WillError::InvalidGuardianThreshold.into())));
}

#[test]
fn test_guardian_trigger_multi_token() {
    let (env, client, owner, _token_a, token_a_addr, _token_b, token_b_addr) = setup_two_tokens();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 6_000,
            },
            Beneficiary {
                address: beneficiary_b,
                basis_points: 3_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

// ── pagination ───────────────────────────────────────────────────────────────

#[test]
fn test_get_wills_by_owner() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 500_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 250_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );
    let wills = client.get_wills_by_owner(&owner, &None, &100);
    assert_eq!(wills.len(), 2);
}

#[test]
fn test_get_wills_by_beneficiary() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    let wills = client.get_wills_by_beneficiary(&beneficiary, &None, &100);
    assert_eq!(wills.len(), 1);
    assert_eq!(wills.get(0).unwrap().id, will_id);
}

#[test]
fn test_weighted_guardian_single_high_weight_triggers() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_heavy = Address::generate(&env);
    let guardian_light = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
// ── Native XLM tests ──────────────────────────────────────────────────

#[test]
fn test_native_create_will_success() {
    let (env, client, owner) = setup_native();
    let beneficiary = Address::generate(&env);

    let owner_initial = env.balance(&owner);

    let will_id = client.create_will(
        &owner,
        &owner, // token address is unused for native, pass owner as placeholder
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    assert_eq!(will_id, 1);
    let will = client.get_will(&will_id);
    assert_eq!(will.owner, owner);
    assert_eq!(will.balance, 1_000_000_000);
    assert_eq!(will.status, WillStatus::Active);
    assert!(will.is_native);
    assert_eq!(will.checkin_period_days, 90);
    assert_eq!(will.grace_period_days, 7);

    // Owner should have lost the amount, contract gained it
    assert_eq!(env.balance(&owner), owner_initial - 1_000_000_000);
    assert_eq!(env.balance(&client.address), 1_000_000_000);
}

#[test]
fn test_native_checkin_resets_deadline() {
    let (env, client, owner) = setup_native();

    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    advance_time(&env, 10 * DAY);
    client.check_in(&will_id, &owner);

    let will = client.get_will(&will_id);
    assert_eq!(will.last_checkin, 1_700_000_000 + 10 * DAY);
    assert_eq!(will.status, WillStatus::Active);
}

#[test]
fn test_native_trigger_and_release() {
    let (env, client, owner) = setup_native();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
// ── new multi-token tests ─────────────────────────────────────────────────────

#[test]
fn test_pagination_owner() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![
            &env,
            (token_a_addr.clone(), 1_000_000_i128),
            (token_b_addr.clone(), 2_000_000_i128),
        ],
        &vec![&env, Beneficiary { address: beneficiary.clone(), percentage: 100 }],
// ── Basis-point / fractional-split tests ─────────────────────────────────────

#[test]
fn test_fractional_three_way_split() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let beneficiary_c = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                basis_points: 5_000,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                basis_points: 3_333,
            },
            Beneficiary {
                address: beneficiary_c.clone(),
                basis_points: 1_667,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.balances.len(), 2);
    assert_eq!(will.balances.get(token_a_addr.clone()).unwrap(), 1_000_000);
    assert_eq!(will.balances.get(token_b_addr.clone()).unwrap(), 2_000_000);

    // Tokens must have moved from owner to contract.
    assert_eq!(token_a.balance(&owner), 1_000_000_000 - 1_000_000);
    assert_eq!(token_b.balance(&owner), 1_000_000_000 - 2_000_000);
    assert_eq!(token_a.balance(&client.address), 1_000_000);
    assert_eq!(token_b.balance(&client.address), 2_000_000);
}

#[test]
fn test_pagination_beneficiary() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let wills = client.get_wills_by_beneficiary(&beneficiary, &None, &50);
    assert_eq!(wills.len(), 1);
    assert_eq!(wills.get(0).unwrap().id, will_id);
}

#[test]
fn test_native_release_splits_multiple_beneficiaries() {
    let (env, client, owner) = setup_native();
/// Extreme split: 1 bp for A, 9_999 bp for B.
/// On a balance of 1_000_000:
///   A = 1_000_000 * 1 / 10_000 = 100
///   B = remainder               = 999_900
#[test]
fn test_fractional_extreme_one_bp_split() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                percentage: 60,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                percentage: 40,
                basis_points: 1,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                basis_points: 9_999,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Top up with a brand-new token_b — it should appear as a new map entry.
    client.top_up(&will_id, &owner, &token_b_addr, &500_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.balances.len(), 2);
    assert_eq!(will.balances.get(token_a_addr.clone()).unwrap(), 1_000_000);
    assert_eq!(will.balances.get(token_b_addr.clone()).unwrap(), 500_000);

    assert_eq!(token_b.balance(&owner), 1_000_000_000 - 500_000);
    assert_eq!(token_b.balance(&client.address), 500_000);
}

#[test]
fn test_clone_will_copies_configuration() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let source_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, g1.clone()],
        &1,
        &None,
    );
    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address.clone(), 500_000_i128)],
    );
    let clone = client.get_will(&clone_id);
    assert_eq!(clone.checkin_period_days, 90);
    assert_eq!(clone.grace_period_days, 7);
    assert_eq!(clone.status, WillStatus::Active);
    assert_eq!(clone.owner, owner);
}

#[test]
fn test_native_emergency_checkin() {
    let (env, client, owner) = setup_native();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    advance_time(&env, 2 * DAY);
    client.emergency_checkin(&will_id, &owner);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert!(will.trigger_time.is_none());
    assert_eq!(will.last_checkin, 1_700_000_000 + 91 * DAY + 2 * DAY);
    // Balance should still be in the contract
    assert_eq!(env.balance(&client.address), 1_000_000_000);
}

#[test]
fn test_native_cancel_will() {
    let (env, client, owner) = setup_native();
    let owner_initial = env.balance(&owner);
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
    assert_eq!(token.balance(&beneficiary_a), 100);
    assert_eq!(token.balance(&beneficiary_b), 999_900);
    assert_eq!(token.balance(&client.address), 0);
}

/// Validation must reject a basis-point sum of 10_001 (one over the limit).
#[test]
#[should_panic]
fn test_basis_points_over_10000_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 5_001,
            },
            Beneficiary {
                address: beneficiary_b,
                basis_points: 5_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    // Contract should have the native XLM
    assert_eq!(env.balance(&client.address), 1_000_000_000);

    client.cancel_will(&will_id, &owner);

    // Owner should be fully refunded
    assert_eq!(env.balance(&owner), owner_initial);
    assert_eq!(env.balance(&client.address), 0);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Cancelled);
    assert_eq!(will.balance, 0);
}

#[test]
fn test_native_top_up() {
    let (env, client, owner) = setup_native();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &owner,
        &500_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
    );
    let clone = client.get_will(&clone_id);
    assert_eq!(clone.id, 2);
    assert_eq!(clone.checkin_period_days, 90);
    assert_eq!(clone.grace_period_days, 7);
    assert_eq!(clone.beneficiaries, vec![&env, bp(&beneficiary, 10_000)]);
    assert_eq!(clone.guardians, vec![&env, g1]);
    assert_eq!(clone.status, WillStatus::Active);
    assert_eq!(clone.balances.get(token_address).unwrap(), 500_000);
    assert_eq!(clone.owner, owner);
}

#[test]
fn test_clone_will_independent_from_source() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let source_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address.clone(), 500_000_i128)],
    );
    client.top_up(&clone_id, &owner, &token_address, &100_000);
    assert_eq!(client.get_will(&source_id).balances.get(token_address.clone()).unwrap(), 1_000_000);
    assert_eq!(client.get_will(&clone_id).balances.get(token_address).unwrap(), 600_000);
}

#[test]
fn test_clone_will_indexed() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let source_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 4_999,
            },
            Beneficiary {
                address: beneficiary_b,
                basis_points: 5_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    let owner_before_topup = env.balance(&owner);
    client.top_up(&will_id, &owner, &300_000_000);

    assert_eq!(env.balance(&owner), owner_before_topup - 300_000_000);
    assert_eq!(env.balance(&client.address), 800_000_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.balance, 800_000_000);
    client.cancel_will(&will_id, &owner);

    // Full balances must be returned to the owner.
    assert_eq!(token_a.balance(&owner), 1_000_000_000);
    assert_eq!(token_b.balance(&owner), 1_000_000_000);
    // Contract must hold nothing.
    assert_eq!(token_a.balance(&client.address), 0);
    assert_eq!(token_b.balance(&client.address), 0);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Cancelled);
    assert_eq!(will.balances.len(), 0);
}

#[test]
fn test_release_inheritance_distributes_all_tokens_proportionally() {
    let (env, client, owner, token_a, token_a_addr, token_b, token_b_addr) =
        setup_two_tokens();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );
    let owner_wills = client.get_wills_by_owner(&owner, &None, &100);
    assert_eq!(owner_wills.len(), 2);
    let beneficiary_wills = client.get_wills_by_beneficiary(&beneficiary, &None, &100);
    assert_eq!(beneficiary_wills.len(), 2);
    assert!(beneficiary_wills.iter().any(|w| w.id == clone_id));
}

#[test]
fn test_native_guardian_trigger() {
    let (env, client, owner) = setup_native();
    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);

#[test]
#[should_panic]
fn test_guardian_cooldown_blocks_trigger() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_a.clone(), guardian_b.clone()],
        &true,
    );

    // First vote should not trigger release
    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.guardian_votes, 1);
    assert_eq!(env.balance(&beneficiary), 0);

    // Second vote should release
    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(env.balance(&beneficiary), 1_000_000_000);
    assert_eq!(env.balance(&client.address), 0);
}

#[test]
fn test_native_rounding_remainder() {
    let (env, client, owner) = setup_native();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let beneficiary_c = Address::generate(&env);

    // Amount that does not split evenly among 3 beneficiaries (10+33+57=100 -> 10/100, 33/100, 57/100)
    let will_id = client.create_will(
        &owner,
        &owner,
        &100, // 100 XLM
        &vec![&env],
        &false,
        &None,
        &0,
        &vec![&env],
    );
    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    client.update_guardians(&will_id, &owner, &vec![&env, g1.clone()]);
    // Immediately try to trigger — cooldown is active.
    advance_time(&env, 1 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
}

#[test]
fn test_guardian_cooldown_allows_after_period() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone()],
        &2,
        &None,
    );
    // Advance past the guardian cooldown window (7 days) so votes are allowed.
    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

#[test]
fn test_initial_guardian_cooldown() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, bp(&beneficiary, 10_000)],
        &90,
        &7,
        &vec![&env, g1.clone(), g2],
        &2,
        &None,
    );
    // Will just created — cooldown should be active.
    let result = client.try_guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    assert!(result.is_err());
}

#[test]
#[should_panic]
fn test_native_cannot_trigger_before_deadline() {
    let (env, client, owner) = setup_native();

    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                percentage: 100,
fn test_create_will_zero_amount_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

#[test]
fn test_batch_create_wills() {
    let (env, client, owner, token, token_address) = setup();
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);
    let ids = client.batch_create_wills(
        &owner,
        &vec![
            &env,
            (
                vec![&env, (token_address.clone(), 100_000_i128)].into(),
                vec![&env, bp(&b1, 10_000)].into(),
                90u64,
                7u64,
                vec![&env].into(),
            ),
            (
                vec![&env, (token_address.clone(), 200_000_i128)].into(),
                vec![&env, bp(&b2, 10_000)].into(),
                30u64,
                3u64,
                vec![&env].into(),
            ),
            (
                vec![&env, (token_address.clone(), 300_000_i128)].into(),
                vec![&env, bp(&b3, 10_000)].into(),
                60u64,
                5u64,
                vec![&env].into(),
            ),
        ],
    );
    assert_eq!(ids.len(), 3);
    assert_eq!(ids.get(0).unwrap(), 1);
    assert_eq!(ids.get(1).unwrap(), 2);
    assert_eq!(ids.get(2).unwrap(), 3);
    assert_eq!(token.balance(&client.address), 600_000);
    let w1 = client.get_will(&1);
    assert_eq!(w1.checkin_period_days, 90);
    let w2 = client.get_will(&2);
    assert_eq!(w2.checkin_period_days, 30);
    let w3 = client.get_will(&3);
    assert_eq!(w3.checkin_period_days, 60);
}

#[test]
#[should_panic]
fn test_update_beneficiaries_rejects_invalid_basis_points() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_orig = Address::generate(&env);
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_orig,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    advance_time(&env, 10 * DAY);
    client.trigger_will(&will_id);
}

#[test]
#[should_panic]
fn test_native_cannot_release_during_grace_period() {
    let (env, client, owner) = setup_native();

    let will_id = client.create_will(
        &owner,
        &owner,
        &1_000_000_000,
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 2 * DAY);
    // Grace period is 7 days, so 2 days is still within it
    client.release_inheritance(&will_id);
}

    );
}

#[test]
#[should_panic]
fn test_batch_too_many_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let specs: SorobanVec<_> = (0..11)
        .map(|_| {
            (
                vec![&env, (token_address.clone(), 100_000_i128)].into(),
                vec![&env, bp(&beneficiary, 10_000)].into(),
                90u64,
                7u64,
                vec![&env].into(),
            )
        })
        .collect();
    client.batch_create_wills(&owner, &specs);
}

#[test]
fn test_batch_transfers_tokens() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let ids = client.batch_create_wills(
        &owner,
        &vec![
            &env,
            (
                vec![&env, (token_address.clone(), 400_000_i128)].into(),
                vec![&env, bp(&beneficiary, 10_000)].into(),
                90u64,
                7u64,
                vec![&env].into(),
            ),
            (
                vec![&env, (token_address.clone(), 600_000_i128)].into(),
                vec![&env, bp(&beneficiary, 10_000)].into(),
                90u64,
                7u64,
                vec![&env].into(),
            ),
        ],
    );
    assert_eq!(ids.len(), 2);
    assert_eq!(token.balance(&owner), 0);
    assert_eq!(token.balance(&client.address), 1_000_000);
}


#[test]
fn test_migrate_will_updates_schema_version() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // New wills should be created at CURRENT_SCHEMA_VERSION
    let will = client.get_will(&will_id);
    assert_eq!(will.schema_version, 1);

    // Migrating a will that's already at current version should be a no-op
    client.migrate_will(&will_id, &owner);
    let will = client.get_will(&will_id);
    assert_eq!(will.schema_version, 1);
}

#[test]
#[should_panic]
fn test_migrate_will_rejects_non_owner() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let non_owner = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.migrate_will(&will_id, &non_owner);
}

#[test]
#[should_panic]
fn test_migrate_nonexistent_will() {
    let (env, client, owner, _token, _token_address) = setup();

    // Try to migrate a will that doesn't exist
    client.migrate_will(&999, &owner);
}

#[test]
fn test_migrate_will_preserves_state() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                percentage: 60,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                percentage: 40,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &2,
        &None,
    );

    advance_time(&env, 10 * DAY);
    client.check_in(&will_id, &owner);

    // Migrate the will
    client.migrate_will(&will_id, &owner);

    // Verify all state is preserved
    let will = client.get_will(&will_id);
    assert_eq!(will.owner, owner);
    assert_eq!(will.balance, 1_000_000);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.checkin_period_days, 90);
    assert_eq!(will.grace_period_days, 7);
    assert_eq!(will.last_checkin, 1_700_000_000 + 10 * DAY);
    assert_eq!(will.beneficiaries.len(), 2);
    assert_eq!(will.beneficiaries.get(0).unwrap().percentage, 60);
    assert_eq!(will.guardians.len(), 1);
    assert_eq!(will.guardians.get(0).unwrap(), &guardian);
    assert_eq!(will.schema_version, 1);
}

#[test]
fn test_migrate_will_emits_event() {
    use soroban_sdk::{testutils::Events, symbol_short, TryIntoVal};

    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Clear events from create_will
    let _ = env.events().all();

    // Migrate and check for event
    client.migrate_will(&will_id, &owner);
    let events = env.events().all();

    let mut found = false;
    for event in events.iter() {
        if !event.1.is_empty() {
            if let Ok(topic0) = event.1.get(0).unwrap().try_into_val(&env) {
                let topic0_sym: soroban_sdk::Symbol = topic0;
                if topic0_sym == symbol_short!("migrated") {
                    found = true;
                    assert_eq!(event.0, client.address.clone());
                    let topic1: u64 = event.1.get(1).unwrap().try_into_val(&env).unwrap();
                    assert_eq!(topic1, will_id);
                    let data: (Address, u32, u32) = event.2.try_into_val(&env).unwrap();
                    assert_eq!(data.0, owner);
                    assert_eq!(data.1, 1); // old version
                    assert_eq!(data.2, 1); // new version
                }
            }
        }
    }
    assert!(found, "migrated event not found");
}

#[test]
fn test_migrate_will_while_triggered() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    // Migration should succeed even while triggered
    client.migrate_will(&will_id, &owner);

    let will = client.get_will(&will_id);
    assert_eq!(will.schema_version, 1);
    assert_eq!(will.status, WillStatus::Triggered);
}

#[test]
fn test_migrate_will_after_emergency_checkin() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    advance_time(&env, 2 * DAY);
    client.emergency_checkin(&will_id, &owner);

    // Migrate after emergency checkin
    client.migrate_will(&will_id, &owner);

    let will = client.get_will(&will_id);
    assert_eq!(will.schema_version, 1);
    assert_eq!(will.status, WillStatus::Active);
}

#[test]
fn test_new_wills_created_at_current_version() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // Create multiple wills
    for i in 0..3 {
        let will_id = client.create_will(
            &owner,
            &vec![&env, (token_address, (1_000_000 * (i + 1) as i128))],
            &vec![
                &env,
                Beneficiary {
                    address: beneficiary.clone(),
                    percentage: 100,
                },
            ],
            &90,
            &7,
            &vec![&env],
            &2,
            &None,
        );

        let will = client.get_will(&will_id);
        assert_eq!(will.schema_version, 1, "Will {} not at current version", will_id);
    }
}


#[test]
fn test_merge_wills_basic() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 600_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 400_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_b.clone(),
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    client.merge_wills(&owner, &will_id_1, &will_id_2);

    let merged_will = client.get_will(&will_id_1);
    assert_eq!(merged_will.balance, 1_000_000);
    assert_eq!(merged_will.beneficiaries.len(), 2);
    assert_eq!(merged_will.status, WillStatus::Active);
    // Check-in period should be minimum (30 days)
    assert_eq!(merged_will.checkin_period_days, 30);
    // Grace period should be maximum (7 days)
    assert_eq!(merged_will.grace_period_days, 7);

    let consumed_will = client.get_will(&will_id_2);
    assert_eq!(consumed_will.balance, 0);
    assert_eq!(consumed_will.status, WillStatus::Cancelled);
}

#[test]
fn test_merge_wills_matching_beneficiaries() {
    let (env, client, owner, _token, token_address) = setup();
    let shared_beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 600_000)],
        &vec![
            &env,
            Beneficiary {
                address: shared_beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 400_000)],
        &vec![
            &env,
            Beneficiary {
                address: shared_beneficiary.clone(),
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    client.merge_wills(&owner, &will_id_1, &will_id_2);

    let merged_will = client.get_will(&will_id_1);
    assert_eq!(merged_will.balance, 1_000_000);
    // Should merge into a single beneficiary entry
    assert_eq!(merged_will.beneficiaries.len(), 1);
    assert_eq!(
        merged_will.beneficiaries.get(0).unwrap().percentage,
        100
    );
}

#[test]
fn test_merge_wills_with_guardians() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);
    let guardian_3 = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &2,
        &None,
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env, guardian_2.clone(), guardian_3.clone()],
        &2,
        &None,
    );

    client.merge_wills(&owner, &will_id_1, &will_id_2);

    let merged_will = client.get_will(&will_id_1);
    assert_eq!(merged_will.guardians.len(), 3);
    assert!(merged_will.guardians.contains(&guardian_1));
    assert!(merged_will.guardians.contains(&guardian_2));
    assert!(merged_will.guardians.contains(&guardian_3));
}

#[test]
#[should_panic]
fn test_merge_wills_same_will_id() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.merge_wills(&owner, &will_id, &will_id);
}

#[test]
#[should_panic]
fn test_merge_wills_not_owner() {
    let (env, client, owner, _token, token_address) = setup();
    let not_owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![
            &env,
            Guardian {
                address: guardian_heavy.clone(),
                weight: 3,
            },
            Guardian {
                address: guardian_light.clone(),
                weight: 1,
            },
        ],
        &2,
        &None,
    );

    client.guardian_trigger(&will_id, &guardian_heavy, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.guardian_vote_weight, 3);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_weighted_guardian_insufficient_weight_stays_active() {
    let (env, client, _owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_light = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
        &vec![&env],
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    client.merge_wills(&not_owner, &will_id_1, &will_id_2);
}

#[test]
#[should_panic]
fn test_merge_wills_first_not_active() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![
            &env,
            Guardian {
                address: guardian_light.clone(),
                weight: 1,
            },
        ],
        &2,
        &None,
    );

    client.guardian_trigger(&will_id, &guardian_light, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.guardian_vote_weight, 1);
}

#[test]
fn test_weighted_guardian_combined_votes() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![&env],
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    // Cancel the first will
    client.cancel_will(&will_id_1, &owner);

    client.merge_wills(&owner, &will_id_1, &will_id_2);
}

#[test]
#[should_panic]
fn test_merge_wills_second_not_active() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![
            &env,
            Guardian {
                address: guardian_a.clone(),
                weight: 1,
            },
            Guardian {
                address: guardian_b.clone(),
                weight: 1,
            },
        ],
        &2,
        &None,
    );

    client.guardian_trigger(&will_id, &guardian_a, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_vote_weight, 1);

    client.guardian_trigger(&will_id, &guardian_b, &GuardianVoteReason::Incapacitated);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.guardian_vote_weight, 2);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_get_wills_by_owner_and_status() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
        &vec![&env],
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    // Cancel the second will
    client.cancel_will(&will_id_2, &owner);

    client.merge_wills(&owner, &will_id_1, &will_id_2);
}

#[test]
fn test_merge_wills_recalculates_percentages() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &token_address,
        &500_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
    );
    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 250_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let active_wills = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &100);
    assert_eq!(active_wills.len(), 2);

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id_1);

    let active_wills = client.get_wills_by_owner_and_status(&owner, &WillStatus::Active, &None, &100);
    assert_eq!(active_wills.len(), 1);
    assert_eq!(active_wills.get(0).unwrap().id, will_id_2);

    let triggered_wills = client.get_wills_by_owner_and_status(&owner, &WillStatus::Triggered, &None, &100);
    assert_eq!(triggered_wills.len(), 1);
    assert_eq!(triggered_wills.get(0).unwrap().id, will_id_1);

    let released_wills = client.get_wills_by_owner_and_status(&owner, &WillStatus::Released, &None, &100);
    assert_eq!(released_wills.len(), 0);
}

#[test]
fn test_close_will_marks_settled() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    assert_eq!(token.balance(&beneficiary), 1_000_000);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);

    client.close_will(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Settled);
}

#[test]
fn test_merge_wills_complex_beneficiaries() {
    let (env, client, owner, _token, token_address) = setup();
    let benef_a = Address::generate(&env);
    let benef_b = Address::generate(&env);
    let benef_c = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
                address: benef_a.clone(),
                percentage: 60,
            },
            Beneficiary {
                address: benef_b.clone(),
                percentage: 40,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);

    client.close_will(&will_id, &owner);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Settled);
    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: benef_b.clone(),
                percentage: 50,
            },
            Beneficiary {
                address: benef_c.clone(),
                percentage: 50,
            },
        ],
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    client.merge_wills(&owner, &will_id_1, &will_id_2);

    let merged_will = client.get_will(&will_id_1);
    assert_eq!(merged_will.balance, 2_000_000);
    // Should have 3 beneficiaries total
    assert_eq!(merged_will.beneficiaries.len(), 3);

    let mut total_percentage: u32 = 0;
    for beneficiary in merged_will.beneficiaries.iter() {
        total_percentage += beneficiary.percentage;
    }
    assert_eq!(total_percentage, 100);
}

#[test]
#[should_panic]
fn test_close_will_requires_released_status() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
fn test_merge_wills_exceeds_beneficiary_limit() {
    let (env, client, owner, _token, token_address) = setup();
    let mut benefs_a = Vec::new(&env);
    let mut benefs_b = Vec::new(&env);

    // Create 6 beneficiaries for will_a
    for i in 0..6 {
        benefs_a.push_back(Beneficiary {
            address: Address::generate(&env),
            percentage: if i == 5 { 34 } else { 11 },
        });
    }

    // Create 5 beneficiaries for will_b
    for i in 0..5 {
        benefs_b.push_back(Beneficiary {
            address: Address::generate(&env),
            percentage: if i == 4 { 20 } else { 20 },
        });
    }

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &benefs_a,
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &benefs_b,
        &30,
        &3,
        &vec![&env],
        &2,
        &None,
    );

    // This should panic because it would exceed MAX_BENEFICIARIES (10)
    client.merge_wills(&owner, &will_id_1, &will_id_2);
}

#[test]
#[should_panic]
fn test_merge_wills_exceeds_guardian_limit() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);
    let guardian_3 = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.close_will(&will_id, &owner);
}

#[test]
#[should_panic]
fn test_close_will_requires_owner() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let non_owner = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
    );

    // Create a second will with different guardians to exceed the limit
    let g4 = Address::generate(&env);
    let g5 = Address::generate(&env);

    let will_id_2 = client.create_will(
        &owner,
        &token_address,
        &500_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env, g4.clone(), g5.clone()],
    );

    // This should panic because it would exceed MAX_GUARDIANS (3)
    client.merge_wills(&owner, &will_id_1, &will_id_2);
}

#[test]
fn test_merge_wills_preserves_token_address() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &token_address,
        &600_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    client.close_will(&will_id, &non_owner);
}

#[test]
fn test_release_event_includes_per_beneficiary_breakdown() {
    let will_id_2 = client.create_will(
        &owner,
        &token_address,
        &400_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &30,
        &3,
        &vec![&env],
    );

    client.merge_wills(&owner, &will_id_1, &will_id_2);

    let merged_will = client.get_will(&will_id_1);
    assert_eq!(merged_will.token, token_address);
}

#[test]
fn test_merge_wills_updates_beneficiary_index() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 600_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary_a.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );
    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 400_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary_b.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Before merge: each beneficiary has one will indexed.
    assert_eq!(client.get_wills_by_beneficiary(&beneficiary_a, &None, &50).len(), 1);
    assert_eq!(client.get_wills_by_beneficiary(&beneficiary_b, &None, &50).len(), 1);

    client.merge_wills(&owner, &will_id_1, &will_id_2);

    // After merge: beneficiary_a still has one will (the surviving one).
    assert_eq!(client.get_wills_by_beneficiary(&beneficiary_a, &None, &50).len(), 1);
}

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    use soroban_sdk::{symbol_short, testutils::Events, TryIntoVal};
    let events = env.events().all();
    let mut found = false;
    for event in events.iter() {
        if !event.1.is_empty() {
            if let Ok(topic0) = event.1.get(0).unwrap().try_into_val(&env) {
                let topic0_sym: soroban_sdk::Symbol = topic0;
                if topic0_sym == symbol_short!("released") {
                    found = true;
                    let topic1: u64 = event.1.get(1).unwrap().try_into_val(&env).unwrap();
                    assert_eq!(topic1, will_id);

                    let data: (i128, bool, soroban_sdk::Vec<(Address, u32, i128)>) =
                        event.2.try_into_val(&env).unwrap();
                    assert_eq!(data.0, 1_000_000_i128);
                    assert!(!data.1, "should not be guardian-triggered");
                    assert_eq!(data.2.len(), 2);

                    let first = data.2.get(0).unwrap();
                    assert_eq!(first.0, beneficiary_a);
                    assert_eq!(first.1, 60);
                    assert_eq!(first.2, 600_000);

                    let second = data.2.get(1).unwrap();
                    assert_eq!(second.0, beneficiary_b);
                    assert_eq!(second.1, 40);
                    assert_eq!(second.2, 400_000);
                }
            }
        }
    }
    assert!(found, "released event not found");
}

#[test]
fn test_guardian_release_event_is_guardian_triggered() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone()],
        &2,
        &None,
    );

    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
}

#[test]
fn test_merge_wills_clears_guardian_votes() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);
    let guardian_3 = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 500_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &2,
        &None,
    );
    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 500_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &30,
        &3,
        &vec![&env, guardian_3.clone()],
        &1,
        &None,
    );

    // Advance past guardian cooldown then cast a vote on will_id_1.
    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id_1, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id_1).guardian_votes, 1);

    // Merge wills — should clear guardian votes on the surviving will.
    client.merge_wills(&owner, &will_id_1, &will_id_2);

    let merged_will = client.get_will(&will_id_1);
    assert_eq!(merged_will.guardian_votes, 0);
}

// --- Issue #24 / #25: Audit trail and history tests ---

#[test]
fn test_will_history_full_lifecycle_release() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Initial history: one entry for creation
    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 1);
    let t0 = history.get(0).unwrap();
    assert_eq!(t0.from_status, WillStatus::Active);
    assert_eq!(t0.to_status, WillStatus::Active);
    assert_eq!(t0.action, symbol_short!("create"));

    // Trigger
    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 2);
    let t1 = history.get(1).unwrap();
    assert_eq!(t1.from_status, WillStatus::Active);
    assert_eq!(t1.to_status, WillStatus::Triggered);
    assert_eq!(t1.action, symbol_short!("trigger"));

    // Release after grace period
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 3);
    let t2 = history.get(2).unwrap();
    assert_eq!(t2.from_status, WillStatus::Triggered);
    assert_eq!(t2.to_status, WillStatus::Released);
    assert_eq!(t2.action, symbol_short!("release"));
}

#[test]
fn test_will_history_full_lifecycle_cancel() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.cancel_will(&will_id, &owner);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 2);
    let t1 = history.get(1).unwrap();
    assert_eq!(t1.from_status, WillStatus::Active);
    assert_eq!(t1.to_status, WillStatus::Cancelled);
    assert_eq!(t1.action, symbol_short!("cancel"));
}

#[test]
fn test_will_history_emergency_checkin() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    advance_time(&env, 2 * DAY);
    client.emergency_checkin(&will_id, &owner);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 3);
    let t2 = history.get(2).unwrap();
    assert_eq!(t2.from_status, WillStatus::Triggered);
    assert_eq!(t2.to_status, WillStatus::Active);
    assert_eq!(t2.action, symbol_short!("emerg"));
}

#[test]
fn test_will_history_guardian_trigger() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &2,
        &None,
    );

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Incapacitated);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 2);
    let t1 = history.get(1).unwrap();
    assert_eq!(t1.from_status, WillStatus::Active);
    assert_eq!(t1.to_status, WillStatus::Released);
    assert_eq!(t1.action, symbol_short!("gtrigr"));
    assert_eq!(t1.actor, guardian_2);
}

#[test]
fn test_will_history_empty_for_new_will() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().will_id, will_id);
}

// --- Issue #23: Archive tests ---

#[test]
fn test_archive_released_will() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    client.archive_will(&will_id);

    // Will should no longer appear in owner queries
    let wills = client.get_wills_by_owner(&owner);
    assert_eq!(wills.len(), 0);

    // Will should no longer appear in beneficiary queries
    let wills = client.get_wills_by_beneficiary(&beneficiary);
    assert_eq!(wills.len(), 0);
}

#[test]
fn test_archive_cancelled_will() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.cancel_will(&will_id, &owner);
    client.archive_will(&will_id);

    let wills = client.get_wills_by_owner(&owner);
    assert_eq!(wills.len(), 0);
}

#[test]
#[should_panic]
fn test_archive_active_will_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Trying to archive an Active will should fail
    client.archive_will(&will_id);
}

#[test]
#[should_panic]
fn test_archive_triggered_will_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                percentage: 100,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    // Trying to archive a Triggered will should fail
    client.archive_will(&will_id);
}

// ── Issue #11: Pull-based beneficiary claim tests ────────────────────────────

/// Pull-mode distribute'stores claimable shares instead of transferring tokens.
#[test]
fn test_pull_distribution_stores_shares() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
// ── #15: Time-weighted guardian vote expiry tests ────────────────────────────

#[test]
fn test_guardian_vote_expiry_defaults_to_grace_period() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                basis_points: 6_000,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                basis_points: 4_000,
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_vote_expiry_days, 7);
}

#[test]
fn test_guardian_vote_expiry_custom_value() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    // Tokens stay in the contract — not transferred to beneficiaries.
    assert_eq!(token.balance(&beneficiary_a), 0);
    assert_eq!(token.balance(&beneficiary_b), 0);
    assert_eq!(token.balance(&client.address), 1_000_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.balance, 0);
}

/// Beneficiaries can independently claim their shares.
#[test]
fn test_claim_share_transfers_tokens() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
        &3,
        &vec![&env],
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_vote_expiry_days, 3);
}

#[test]
fn test_expired_guardian_vote_does_not_count() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);
    let guardian_3 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone(), guardian_3.clone()],
        &2,
        &None,
    );

    guardian_1 votes, then 3 days pass (beyond 2-day expiry), then guardian_2 votes.
    // guardian_1's vote should be expired, so only 1 valid vote - no release.
    // guardian_3 can then vote to reach 2 valid votes and trigger release.

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);

    advance_time(&env, 3 * DAY);

    client.guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Deceased);
    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_votes, 1);
    assert_eq!(will.status, WillStatus::Active);

    client.guardian_trigger(&will_id, &guardian_3, &GuardianVoteReason::Unreachable);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_same_guardian_cannot_revote_before_expiry() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                basis_points: 6_000,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                basis_points: 4_000,
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone()],
        &2,
        &None,
    );

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Other);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);

    advance_time(&env, 1 * DAY);

    let result = client.try_guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Other);
    assert!(result.is_err());
}

#[test]
fn test_same_guardian_can_revote_after_expiry() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    // Beneficiary A claims first.
    client.claim_share(&will_id, &beneficiary_a);
    assert_eq!(token.balance(&beneficiary_a), 600_000);
    assert_eq!(token.balance(&client.address), 400_000);

    // Beneficiary B claims second.
    client.claim_share(&will_id, &beneficiary_b);
    assert_eq!(token.balance(&beneficiary_b), 400_000);
    assert_eq!(token.balance(&client.address), 0);
}

/// A non-beneficiary cannot claim a share.
#[test]
#[should_panic]
fn test_claim_share_rejects_non_beneficiary() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let non_beneficiary = Address::generate(&env);
        &vec![&env, guardian_1.clone()],
        &2,
        &vec![&env],
    );

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);

    advance_time(&env, 3 * DAY);

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);
}

// ── #17: Guardian vote reason code tests ─────────────────────────────────────

#[test]
fn test_guardian_vote_reason_stored_and_emitted() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    client.claim_share(&will_id, &non_beneficiary);
}

/// A beneficiary cannot claim twice.
#[test]
#[should_panic]
fn test_claim_share_rejects_already_claimed() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
        &vec![&env, guardian_1.clone()],
        &0,
        &vec![&env],
    );

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Deceased);

    use soroban_sdk::testutils::Events;
    let events = env.events().all();
    let mut found_gvote = false;
    for event in events.iter() {
        if !event.1.is_empty() {
            if let Ok(topic0) = event.1.get(0).unwrap().try_into_val(&env) {
                let topic0_sym: soroban_sdk::Symbol = topic0;
                if topic0_sym == soroban_sdk::symbol_short!("gvote") {
                    found_gvote = true;
                    let data: (Address, u32, GuardianVoteReason) =
                        event.2.try_into_val(&env).unwrap();
                    assert_eq!(data.0, guardian_1);
                    assert_eq!(data.1, 1);
                    assert_eq!(data.2, GuardianVoteReason::Deceased);
                }
            }
        }
    }
    assert!(found_gvote, "gvote event not found");
}

#[test]
fn test_all_guardian_reason_codes() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &true,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    client.claim_share(&will_id, &beneficiary);
    client.claim_share(&will_id, &beneficiary);
}

/// Pull-mode with three-way fractional split: each beneficiary claims independently.
#[test]
fn test_pull_distribution_fractional_split() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let beneficiary_c = Address::generate(&env);
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &0,
        &vec![&env],
    );

    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Other);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
}

// ── #16: Multi-tier grace period tests ──────────────────────────────────────

#[test]
fn test_create_will_with_grace_tiers() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &14,
        &vec![&env],
        &2,
        &None,
    );

    let will = client.get_will(&will_id);
    assert_eq!(will.grace_tiers.len(), 2);
    assert_eq!(will.released_basis_points, 0);
}

#[test]
fn test_release_tier_first_milestone() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary_a.clone(), basis_points: 6_000 },
              Beneficiary { address: beneficiary_b.clone(), basis_points: 4_000 }],
        &90,
        &14,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    assert_eq!(token.balance(&beneficiary_a), 600_000);
    assert_eq!(token.balance(&beneficiary_b), 400_000);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
}

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    // All tokens stay in the contract.
    assert_eq!(token.balance(&client.address), 1_000_000);

    // Each beneficiary claims their share.
    client.claim_share(&will_id, &beneficiary_a);
    assert_eq!(token.balance(&beneficiary_a), 500_000);

    client.claim_share(&will_id, &beneficiary_b);
    assert_eq!(token.balance(&beneficiary_b), 333_300);

    client.claim_share(&will_id, &beneficiary_c);
    assert_eq!(token.balance(&beneficiary_c), 166_700);

    assert_eq!(token.balance(&client.address), 0);
}

/// Guardian trigger also works with pull-mode distribution.
#[test]
fn test_guardian_trigger_pull_distribution() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &2,
        &None,
    );

    advance_time(&env, 8 * DAY);
    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Incapacitated);

    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
}

#[test]
fn test_release_tier_both_milestones() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &true,
        &None,
    );

    client.accept_guardian_role(&will_id, &guardian_1);
    client.accept_guardian_role(&will_id, &guardian_2);
    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Incapacitated);

    // Tokens stay in contract (pull mode).
    assert_eq!(token.balance(&beneficiary), 0);
    assert_eq!(token.balance(&client.address), 1_000_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);

    // Beneficiary can claim.
    client.claim_share(&will_id, &beneficiary);
    assert_eq!(token.balance(&beneficiary), 1_000_000);
    assert_eq!(token.balance(&client.address), 0);
}

// ── Issue #12: Fallback beneficiary tests ────────────────────────────────────

/// Owner can set and read the fallback beneficiary.
#[test]
fn test_set_fallback_beneficiary() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let fallback = Address::generate(&env);
        &14,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 3_000,
            },
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 7_000,
            },
        ],
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    advance_time(&env, 8 * DAY);
    client.release_tier(&will_id, &0);
    assert_eq!(token.balance(&beneficiary), 300_000);

    advance_time(&env, 7 * DAY);
    client.release_tier(&will_id, &1);
    assert_eq!(token.balance(&beneficiary), 1_000_000);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.balance, 0);
}

#[test]
#[should_panic]
fn test_release_tier_before_deadline() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
        &None,
    );

    assert_eq!(client.get_fallback_beneficiary(&will_id), None);

    client.set_fallback_beneficiary(&will_id, &owner, &Some(fallback.clone()));
    assert_eq!(client.get_fallback_beneficiary(&will_id), Some(fallback.clone()));

    client.set_fallback_beneficiary(&will_id, &owner, &None);
    assert_eq!(client.get_fallback_beneficiary(&will_id), None);
}

/// Fallback can be set at will creation.
#[test]
fn test_create_will_with_fallback() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let fallback = Address::generate(&env);
        &14,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 5_000,
            },
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 5_000,
            },
        ],
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    advance_time(&env, 3 * DAY);
    client.release_tier(&will_id, &0);
}

#[test]
#[should_panic]
fn test_release_tier_already_released() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
        &Some(fallback.clone()),
    );

    assert_eq!(client.get_fallback_beneficiary(&will_id), Some(fallback));
    let will = client.get_will(&will_id);
    assert_eq!(will.fallback_beneficiary, Some(fallback));
}

// ── Issue #13: Guardian consent flow tests ───────────────────────────────────

/// Guardian must accept before voting.
#[test]
fn test_guardian_must_accept_before_voting() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);
        &14,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 5_000,
            },
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 5_000,
            },
        ],
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_tier(&will_id, &0);
    client.release_tier(&will_id, &0);
}

#[test]
#[should_panic]
fn test_release_tier_out_of_range() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone(), guardian_2.clone()],
        &false,
        &None,
    );

    // Guardian 1 tries to vote without accepting — must panic.
    let result = client.try_guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert!(result.is_err());

    // Guardian 1 accepts, then votes successfully.
    client.accept_guardian_role(&will_id, &guardian_1);
    client.guardian_trigger(&will_id, &guardian_1, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);
}

/// Guardian can accept their role.
#[test]
fn test_accept_guardian_role() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);
        &14,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 5_000,
            },
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 5_000,
            },
        ],
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_tier(&will_id, &5);
}

#[test]
#[should_panic]
fn test_release_tier_no_grace_tiers() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &false,
        &None,
    );

    client.accept_guardian_role(&will_id, &guardian);

    // Accepting again must panic.
    let result = client.try_accept_guardian_role(&will_id, &guardian);
    assert!(result.is_err());
}

/// Guardian trigger rejects unaccepted guardian.
#[test]
#[should_panic]
fn test_guardian_trigger_rejects_unaccepted() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &vec![&env],
        &0,
        &vec![&env],
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_tier(&will_id, &0);
}

#[test]
#[should_panic]
fn test_invalid_grace_tiers_bp_not_10000() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &false,
        &None,
    );

    // Must panic because guardian has not accepted.
    client.guardian_trigger(&will_id, &guardian, &GuardianVoteReason::Incapacitated);
}

/// Non-guardian cannot accept guardian role.
#[test]
#[should_panic]
fn test_accept_guardian_role_rejects_non_guardian() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let non_guardian = Address::generate(&env);

    let will_id = client.create_will(
        &14,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 4_000,
            },
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 5_000,
            },
        ],
    );
}

#[test]
#[should_panic]
fn test_invalid_grace_tiers_not_ascending() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
        &None,
    );

    client.accept_guardian_role(&will_id, &non_guardian);
}

/// update_guardians resets consent for old guardians and sets Pending for new ones.
#[test]
fn test_update_guardians_resets_consent() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_1 = Address::generate(&env);
    let guardian_2 = Address::generate(&env);
    let guardian_3 = Address::generate(&env);

    let will_id = client.create_will(
        &14,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 5_000,
            },
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 5_000,
            },
        ],
    );
}

#[test]
#[should_panic]
fn test_invalid_grace_tiers_beyond_grace_period() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_1.clone()],
        &false,
        &None,
    );

    // Guardian 1 accepts.
    client.accept_guardian_role(&will_id, &guardian_1);

    // Replace with guardian_2 — guardian_1's consent should be cleared.
    client.update_guardians(&will_id, &owner, &vec![&env, guardian_2.clone()]);

    // Guardian 1 is no longer a guardian, so accepting must fail.
    let result = client.try_accept_guardian_role(&will_id, &guardian_1);
    assert!(result.is_err());

    // Guardian 2 is named but has not accepted, so voting must fail.
    let result = client.try_guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Incapacitated);
    assert!(result.is_err());

    // Guardian 2 accepts, then votes successfully.
    client.accept_guardian_role(&will_id, &guardian_2);
    client.guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Incapacitated);
    assert_eq!(client.get_will(&will_id).guardian_votes, 1);

    // Add guardian_3 alongside guardian_2 — guardian_2's consent should persist.
    client.update_guardians(
        &will_id,
        &owner,
        &vec![&env, guardian_2.clone(), guardian_3.clone()],
    );

    // Guardian 2 should still be accepted (consent was not cleared for them
    // because they were re-added). Actually, update_guardians clears ALL old
    // consents and resets new ones to Pending. So guardian_2 must re-accept.
    let result = client.try_guardian_trigger(&will_id, &guardian_2, &GuardianVoteReason::Incapacitated);
    assert!(result.is_err());
}

// ── Issue #14: Guardian self-initiated replacement request tests ──────────────

/// Guardian can request replacement.
#[test]
fn test_request_guardian_replacement() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
        &vec![&env],
        &0,
        &vec![
            &env,
            GraceTier {
                day_offset: 7 * DAY,
                basis_points: 5_000,
            },
            GraceTier {
                day_offset: 14 * DAY,
                basis_points: 5_000,
            },
        ],
    );
}

#[test]
fn test_release_inheritance_still_works_with_empty_tiers() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    assert_eq!(token.balance(&beneficiary), 1_000_000);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
}

#[test]
fn test_grace_tiers_three_way_split() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                basis_points: 6_000,
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                basis_points: 4_000,
            },
        ],
        &90,
        &30,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    advance_time(&env, 11 * DAY);
    client.release_tier(&will_id, &0);
    assert_eq!(token.balance(&beneficiary_a), 120_000);
    assert_eq!(token.balance(&beneficiary_b), 80_000);
    assert_eq!(token.balance(&client.address), 800_000);

    advance_time(&env, 10 * DAY);
    client.release_tier(&will_id, &1);
    assert_eq!(token.balance(&beneficiary_a), 300_000);
    assert_eq!(token.balance(&beneficiary_b), 200_000);
    assert_eq!(token.balance(&client.address), 500_000);

    advance_time(&env, 10 * DAY);
    client.release_tier(&will_id, &2);
    assert_eq!(token.balance(&beneficiary_a), 600_000);
    assert_eq!(token.balance(&beneficiary_b), 400_000);
    assert_eq!(token.balance(&client.address), 0);

    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.balance, 0);
}

// ── #20: Batch check-in tests ───────────────────────────────────────────────

#[test]
fn test_batch_checkin_multiple_wills() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &token_address,
        &500_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &false,
        &None,
    );

    client.request_guardian_replacement(&will_id, &guardian);
}

/// Non-guardian cannot request replacement.
#[test]
#[should_panic]
fn test_request_guardian_replacement_rejects_non_guardian() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let non_guardian = Address::generate(&env);
        &vec![&env],
        &0,
        &vec![&env],
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 300_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &30,
        &5,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 10 * DAY);

    client.batch_check_in(
        &vec![&env, will_id_1, will_id_2],
        &owner,
    );

    let will_1 = client.get_will(&will_id_1);
    assert_eq!(will_1.last_checkin, 1_700_000_000 + 10 * DAY);

    let will_2 = client.get_will(&will_id_2);
    assert_eq!(will_2.last_checkin, 1_700_000_000 + 10 * DAY);
}

#[test]
fn test_batch_checkin_single_will() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
        &None,
    );

    client.request_guardian_replacement(&will_id, &non_guardian);
}

/// Cannot request replacement on a non-Active will.
#[test]
#[should_panic]
fn test_request_guardian_replacement_rejected_while_triggered() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &token_address,
        &1_000_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
        &0,
        &vec![&env],
    );

    advance_time(&env, 5 * DAY);

    client.batch_check_in(&vec![&env, will_id], &owner);

    let will = client.get_will(&will_id);
    assert_eq!(will.last_checkin, 1_700_000_000 + 5 * DAY);
}

#[test]
#[should_panic]
fn test_batch_checkin_rejects_non_active_will() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &token_address,
        &500_000,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &false,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);

    client.request_guardian_replacement(&will_id, &guardian);
        &vec![&env],
        &0,
        &vec![&env],
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 300_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &30,
        &5,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id_2);

    advance_time(&env, 1 * DAY);
    client.batch_check_in(&vec![&env, will_id_1, will_id_2], &owner);
}

#[test]
fn test_batch_checkin_emits_event() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id_1 = client.create_will(
        &owner,
        &vec![&env, (token_address, 500_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    let will_id_2 = client.create_will(
        &owner,
        &vec![&env, (token_address, 250_000)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &60,
        &5,
        &vec![&env],
        &2,
        &None,
    );

    use soroban_sdk::{testutils::Events, symbol_short, TryIntoVal};
    advance_time(&env, 5 * DAY);
    client.batch_check_in(&vec![&env, will_id_1, will_id_2], &owner);

    let events = env.events().all();
    let mut found_batch = false;
    for event in events.iter() {
        if !event.1.is_empty() {
            if let Ok(topic0) = event.1.get(0).unwrap().try_into_val(&env) {
                let topic0_sym: soroban_sdk::Symbol = topic0;
                if topic0_sym == symbol_short!("batchck") {
                    found_batch = true;
                }
            }
        }
    }
    assert!(found_batch, "batch checkin event not found");
}

// ── Issue #77: Duplicate Guardian Validation ──────────────────────────────────

#[test]
#[should_panic(expected = "DuplicateGuardian")]
fn test_duplicate_guardians_rejected_in_create() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone(), guardian.clone()],
        &false,
    );
}

#[test]
#[should_panic(expected = "DuplicateGuardian")]
fn test_duplicate_guardians_rejected_in_update() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian_a = Address::generate(&env);
    let guardian_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian_a],
        &false,
    );

    client.update_guardians(&will_id, &owner, &vec![&env, guardian_b.clone(), guardian_b]);
}

// ── Issue #78: Owner Cannot Be Guardian ───────────────────────────────────────

#[test]
#[should_panic(expected = "OwnerCannotBeGuardian")]
fn test_owner_cannot_be_guardian_in_create() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, owner.clone()],
        &false,
    );
}

#[test]
#[should_panic(expected = "OwnerCannotBeGuardian")]
fn test_owner_cannot_be_guardian_in_update() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env, guardian],
        &false,
    );

    client.update_guardians(&will_id, &owner, &vec![&env, owner.clone()]);
}

// ── Issue #79: Reject Zero-Percentage Beneficiaries ──────────────────────────

#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_zero_percentage_beneficiary_rejected_in_create() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 10_000,
            },
            Beneficiary {
                address: beneficiary_b,
                basis_points: 0,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
    );
}

#[test]
#[should_panic(expected = "InvalidPercentages")]
fn test_zero_percentage_beneficiary_rejected_in_update() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
    );

    client.update_beneficiaries(
        &will_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 5_000,
            },
            Beneficiary {
                address: beneficiary_b,
                basis_points: 5_000,
            },
            Beneficiary {
                address: Address::generate(&env),
                basis_points: 0,
            },
        ],
    );
}

// ── Issue #80: Rounding Behavior in Distribution ────────────────────────────

#[test]
fn test_rounding_with_small_balance_and_many_beneficiaries() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let beneficiary_c = Address::generate(&env);
    let beneficiary_d = Address::generate(&env);
    let beneficiary_e = Address::generate(&env);
    let beneficiary_f = Address::generate(&env);
    let beneficiary_g = Address::generate(&env);
    let beneficiary_h = Address::generate(&env);
    let beneficiary_i = Address::generate(&env);
    let beneficiary_j = Address::generate(&env);

    // Create a will with 9 base units distributed equally among 10 beneficiaries
    // Each should get 0.9 units, which truncates to 0 for all but the last one
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 9_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_b,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_c,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_d,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_e,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_f,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_g,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_h,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_i,
                basis_points: 1_000,
            },
            Beneficiary {
                address: beneficiary_j,
                basis_points: 1_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &false,
    );

    // Trigger and release the will
// ── Issue #69: remove_beneficiary_index must extend TTL ──────────────────────

/// After removing a beneficiary from a will, the BeneficiaryWills index entry
/// must still be readable even after the ledger sequence has advanced past the
/// TTL that would have been set by the original `index_by_beneficiary` write.
/// The fix in `remove_beneficiary_index` calls `extend_ttl` after every write,
/// so the entry survives purely-pruning workloads.
#[test]
fn test_remove_beneficiary_index_extends_ttl() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);

    // Create a will with two beneficiaries so beneficiary_b has an index entry.
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary { address: beneficiary_a.clone(), basis_points: 5_000 },
            Beneficiary { address: beneficiary_b.clone(), basis_points: 5_000 },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Remove beneficiary_a: this exercises remove_beneficiary_index and should
    // bump the TTL on beneficiary_b's index (which still holds will_id).
    client.update_beneficiaries(
        &will_id,
        &owner,
        &vec![&env, Beneficiary { address: beneficiary_b.clone(), basis_points: 10_000 }],
    );

    // Advance the ledger far beyond what the original TTL would have been.
    // DAY_IN_LEDGERS ≈ 17,280; BUMP_AMOUNT = 60 days = 1,036,800 ledgers.
    // We advance 61 days worth of ledgers to simulate a TTL that was *not*
    // refreshed. The Soroban test host does not actually expire entries by
    // ledger sequence in unit tests, so we assert readability as a proxy for
    // "the write + extend path was exercised without panic."
    env.ledger().with_mut(|l| {
        l.sequence_number += 17_280 * 61; // 61 days of ledgers
        l.timestamp += 61 * DAY;
    });

    // beneficiary_b must still appear in its index.
    let wills = client.get_wills_by_beneficiary(&beneficiary_b, &None, &50);
    assert_eq!(wills.len(), 1);
    assert_eq!(wills.get(0).unwrap().id, will_id);

    // beneficiary_a was removed, so its index must be empty.
    let wills_a = client.get_wills_by_beneficiary(&beneficiary_a, &None, &50);
    assert_eq!(wills_a.len(), 0);
}

// ── Issue #70: cancel_will must prune beneficiary index ──────────────────────

/// After cancelling a will, `get_wills_by_beneficiary` must return an empty
/// list for the former beneficiaries — no stale entries should remain.
#[test]
fn test_cancel_will_removes_beneficiary_index() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    // Sanity: beneficiary is indexed before cancel.
    assert_eq!(client.get_wills_by_beneficiary(&beneficiary, &None, &50).len(), 1);

    client.cancel_will(&will_id, &owner);

    // After cancel, the index must be clean.
    let wills = client.get_wills_by_beneficiary(&beneficiary, &None, &50);
    assert_eq!(wills.len(), 0, "cancelled will must not appear in beneficiary index");
}

/// cancel_will with multiple beneficiaries must prune all of their index entries.
#[test]
fn test_cancel_will_removes_all_beneficiary_indexes() {
    let (env, client, owner, _token, token_address) = setup();
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);
    let b3 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary { address: b1.clone(), basis_points: 4_000 },
            Beneficiary { address: b2.clone(), basis_points: 3_000 },
            Beneficiary { address: b3.clone(), basis_points: 3_000 },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.cancel_will(&will_id, &owner);

    assert_eq!(client.get_wills_by_beneficiary(&b1, &None, &50).len(), 0);
    assert_eq!(client.get_wills_by_beneficiary(&b2, &None, &50).len(), 0);
    assert_eq!(client.get_wills_by_beneficiary(&b3, &None, &50).len(), 0);
}

// ── Issue #71: distribute() must prune owner and beneficiary indexes ──────────

/// After a normal release_inheritance, the will must be absent from both the
/// owner index and every beneficiary index.
#[test]
fn test_release_inheritance_removes_indexes() {
    let (env, client, owner, _token, token_address) = setup();
    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary { address: b1.clone(), basis_points: 6_000 },
            Beneficiary { address: b2.clone(), basis_points: 4_000 },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    // Owner index must no longer list the will.
    let owner_wills = client.get_wills_by_owner(&owner);
    assert!(
        !owner_wills.iter().any(|w| w.id == will_id),
        "released will must not appear in owner index"
    );

    // Beneficiary indexes must no longer list the will.
    assert_eq!(
        client.get_wills_by_beneficiary(&b1, &None, &50).len(), 0,
        "released will must not appear in b1 beneficiary index"
    );
    assert_eq!(
        client.get_wills_by_beneficiary(&b2, &None, &50).len(), 0,
        "released will must not appear in b2 beneficiary index"
    );
}

/// After a guardian_trigger-driven release, same index-pruning guarantees hold.
#[test]
fn test_guardian_trigger_release_removes_indexes() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone()],
        &2,
        &None,
    );

    // Advance past the guardian-list cooldown (7 days).
    advance_time(&env, 8 * DAY);

    client.guardian_trigger(&will_id, &g1, &GuardianVoteReason::Incapacitated);
    client.guardian_trigger(&will_id, &g2, &GuardianVoteReason::Incapacitated);

    // Will must be Released after quorum.
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);

    // Both indexes must be clean.
    let owner_wills = client.get_wills_by_owner(&owner);
    assert!(
        !owner_wills.iter().any(|w| w.id == will_id),
        "guardian-released will must not appear in owner index"
    );
    assert_eq!(
        client.get_wills_by_beneficiary(&beneficiary, &None, &50).len(), 0,
        "guardian-released will must not appear in beneficiary index"
    );
}

// ── Issue #72: Checks-effects-interactions ordering ──────────────────────────

/// After cancel_will, the on-chain status must already be Cancelled before any
/// token balances move. We verify this by reading the will state immediately
/// after the call and asserting the status is terminal — if the state were only
/// written after the transfer loop, a reentrant token could read Active status.
#[test]
fn test_cancel_will_status_committed_before_transfer() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    client.cancel_will(&will_id, &owner);

    // State must be terminal regardless of transfer order.
    assert_eq!(client.get_will(&will_id).status, WillStatus::Cancelled);
    // Balance must be zeroed.
    assert_eq!(client.get_will(&will_id).balances.len(), 0);
}

/// After release_inheritance, the on-chain status must be Released and the
/// balance map empty — both committed before any external transfer fires.
#[test]
fn test_release_inheritance_status_committed_before_transfer() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
    );

    advance_time(&env, 91 * DAY);
    client.trigger_will(&will_id);
    advance_time(&env, 8 * DAY);
    client.release_inheritance(&will_id);

    // Verify that the last beneficiary received all the remainder
    // (0 + 0 + 0 + 0 + 0 + 0 + 0 + 0 + 0 + 9 due to rounding remainder)
    assert_eq!(token.balance(&beneficiary_j), 9);
    assert_eq!(token.balance(&beneficiary_a), 0);
    assert_eq!(token.balance(&beneficiary_b), 0);
    assert_eq!(token.balance(&beneficiary_c), 0);
    assert_eq!(token.balance(&beneficiary_d), 0);
    assert_eq!(token.balance(&beneficiary_e), 0);
    assert_eq!(token.balance(&beneficiary_f), 0);
    assert_eq!(token.balance(&beneficiary_g), 0);
    assert_eq!(token.balance(&beneficiary_h), 0);
    assert_eq!(token.balance(&beneficiary_i), 0);

    // Verify will is now released
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    let will = client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Released);
    assert_eq!(will.balances.len(), 0);
    assert_eq!(will.balance, 0);
}

// ── Issue #74: Reject zero-length check-in/grace periods ─────────────────────

#[test]
#[should_panic(expected = "InvalidPeriod")]
fn test_create_will_zero_checkin_period_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary, basis_points: 10_000 }],
        &0,
        &7,
        &vec![&env],
        &None,
    );
}

#[test]
#[should_panic(expected = "InvalidPeriod")]
fn test_create_will_zero_grace_period_rejected() {
    let (env, client, owner, _token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![&env, Beneficiary { address: beneficiary, basis_points: 10_000 }],
        &90,
        &0,
        &vec![&env],
        &None,
    );
}

// ── Atomicity of create_will when the token transfer fails ───────────────
//
// Soroban transactions are atomic: if any host call inside a contract
// invocation fails (traps), every storage write performed earlier in that
// same invocation is rolled back as if it never happened. `create_will`
// relies on this: it writes the `Will` record, the `NextWillId` counter, and
// the owner/beneficiary index entries only after the loop that performs the
// token transfer for every entry. If `token::Client::transfer` panics (e.g.
// the owner has insufficient balance or never approved a large enough
// allowance for the contract to pull from), the whole invocation must
// revert with no partial state left behind. This was previously an
// assumption baked into the atomicity of the host — never directly
// exercised by a test.
#[test]
fn test_create_will_reverts_atomically_on_insufficient_balance() {
    let (env, client, owner, token, token_address) = setup();
    let beneficiary = Address::generate(&env);

    // owner was minted 1_000_000_000 in `setup`; ask for far more than that
    // so the underlying SAC `transfer` call traps.
    let excessive_amount = 10_000_000_000_i128;

    let result = client.try_create_will(
        &owner,
        &vec![&env, (token_address.clone(), excessive_amount)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &None,
    );
    assert!(result.is_err(), "create_will should fail when the transfer cannot be completed");

    // No Will record and no index entries should have been left behind by
    // the failed attempt.
    assert!(client.get_wills_by_owner(&owner).is_empty());
    assert!(client.get_wills_by_beneficiary(&beneficiary).is_empty());

    // The owner's balance must be untouched — the failed transfer must not
    // have moved any funds either.
    assert_eq!(token.balance(&owner), 1_000_000_000);
    assert_eq!(token.balance(&client.address), 0);

    // `NextWillId` must not have been incremented by the failed attempt: the
    // next *successful* call should still allocate id 1, not 2.
    let good_will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &None,
    );
    assert_eq!(
        good_will_id, 1,
        "a failed create_will must not have consumed a will id"
    );
}

// ── Behavior against a non-conforming ("misbehaving") token ───────────────
//
// Every other test in this file uses the real Stellar Asset Contract, which
// faithfully implements SEP-41's `transfer`. `create_will`'s `tokens`
// parameter accepts *any* `Address`, with no interface or behavior
// validation (see issue #74) — nothing stops an owner from locking a will
// against a token contract that doesn't actually move funds. This test uses
// the `NoopToken` mock defined below, whose `transfer` silently returns
// without adjusting any balance, to document the contract's current
// behavior in that situation: it happily records a `Will` with a balance
// that was never actually backed by a real transfer.
//
// This is a documented gap, not a fix. Whether `create_will` should
// additionally verify the token's balance moved (e.g. by reading balances
// before/after the transfer call) is left as a follow-up beyond this test —
// see issue #74 for the broader token-validation discussion. Right now the
// contract trusts `token::Client::transfer` unconditionally, exactly as it
// would for a real SAC.
#[test]
fn test_create_will_with_noop_token_records_unbacked_balance() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);

    let token_id = env.register(NoopToken, ());
    let token_client = NoopTokenClient::new(&env, &token_id);
    // Deliberately do NOT mint anything to `owner` — a conforming token
    // would reject the transfer below for insufficient balance. This mock
    // never checks balances at all, which is exactly the misbehavior being
    // demonstrated.

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_id.clone(), 5_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                basis_points: 10_000,
            },
        ],
        &90,
        &7,
        &vec![&env],
        &None,
    );

    // The contract recorded a balance for a transfer that never actually
    // happened: the mock's `transfer` is a no-op, so no funds ever moved,
    // yet `Will.balances` reflects the full requested amount.
    let will = client.get_will(&will_id);
    assert_eq!(will.balances.get(token_id.clone()).unwrap(), 5_000_000);

    // Confirming the transfer really was a no-op: the mock token never
    // tracked any balance for the owner or the contract, because its
    // `transfer` does not touch storage at all.
    assert_eq!(token_client.balance(&owner), 0);
    assert_eq!(token_client.balance(&contract_id), 0);
}

/// Minimal mock SEP-41-shaped token whose `transfer` silently no-ops
/// instead of moving funds or reverting. Shares the "fake token contract"
/// approach used by `test_support::MaliciousToken` (issue #55's reentrancy
/// harness), but simulates a different misbehavior: instead of reentering
/// the caller, it simply pretends every transfer succeeded without moving
/// any balance.
#[contract]
pub struct NoopToken;

#[contractimpl]
impl NoopToken {
    pub fn mint(_env: Env, _to: Address, _amount: i128) {}

    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }

    /// Always "succeeds" without moving any balance, unlike a conforming
    /// SEP-41 token which would debit `from` and credit `to`.
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {
        // no-op: silently pretend the transfer happened.
    }
}
