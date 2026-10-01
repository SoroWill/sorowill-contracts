#![cfg(test)]

//! Regression tests for #490, #491, #492 and #493.
//!
//! - #490: `release_inheritance` only pays out a will that is still
//!   `Triggered`, re-validated right before distribution; no ordering of
//!   trigger / check-in / release can release an `Active` will.
//! - #491: a release at the beneficiary cap pays everyone across several
//!   tokens, over-cap creation is rejected, and an over-cap stored will
//!   (e.g. a legacy layout) is refused at release.
//! - #492: owner-index pagination stays correct across additions and
//!   deletions, and out-of-bounds cursors return an empty page.
//! - #493: `confirm_will` re-checks ownership and beneficiaries at
//!   confirmation time.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus,
    MAX_BENEFICIARIES,
};

const DAY: u64 = 86_400;
const START: u64 = 1_700_000_000;
const BALANCE: i128 = 1_000_000;

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    client: WillContractClient<'a>,
    owner: Address,
    token: Address,
}

fn setup<'a>() -> Ctx<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &(BALANCE * 100));

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    Ctx {
        env,
        contract_id,
        client,
        owner,
        token,
    }
}

fn extra_token(ctx: &Ctx) -> Address {
    let token = ctx
        .env
        .register_stellar_asset_contract_v2(ctx.owner.clone())
        .address();
    StellarAssetClient::new(&ctx.env, &token).mint(&ctx.owner, &BALANCE);
    token
}

fn pct(address: &Address, bp: u32) -> Beneficiary {
    Beneficiary {
        address: address.clone(),
        allocation: Allocation::Percentage(bp),
    }
}

fn create_with(
    ctx: &Ctx,
    tokens: Vec<(Address, i128)>,
    beneficiaries: Vec<Beneficiary>,
    confirmation_window: u64,
) -> u64 {
    ctx.client.create_will(
        &ctx.owner,
        &tokens,
        &beneficiaries,
        &90,
        &7,
        &vec![&ctx.env],
        &0,
        &None,
        &confirmation_window,
    )
}

fn create_simple(ctx: &Ctx, beneficiary: &Address) -> u64 {
    create_with(
        ctx,
        vec![&ctx.env, (ctx.token.clone(), BALANCE)],
        vec![&ctx.env, pct(beneficiary, 10_000)],
        0,
    )
}

fn cap_beneficiaries(ctx: &Ctx) -> (Vec<Beneficiary>, Vec<Address>) {
    let mut beneficiaries: Vec<Beneficiary> = Vec::new(&ctx.env);
    let mut addresses: Vec<Address> = Vec::new(&ctx.env);
    for _ in 0..MAX_BENEFICIARIES {
        let a = Address::generate(&ctx.env);
        beneficiaries.push_back(pct(&a, 10_000 / MAX_BENEFICIARIES));
        addresses.push_back(a);
    }
    (beneficiaries, addresses)
}

fn owner_page(ctx: &Ctx, cursor: Option<u64>, limit: u32) -> Vec<u64> {
    let page = ctx.client.get_wills_by_owner(&ctx.owner, &cursor, &limit);
    let mut ids: Vec<u64> = Vec::new(&ctx.env);
    for will in page.iter() {
        ids.push_back(will.id);
    }
    ids
}

// ---------------------------------------------------------------------------
// #490 — release only from a still-Triggered will
// ---------------------------------------------------------------------------

#[test]
fn release_of_active_will_is_rejected() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_simple(&ctx, &b);

    env.ledger().set_timestamp(START + 200 * DAY);
    assert_eq!(
        ctx.client.try_release_inheritance(&id, &None),
        Err(Ok(WillError::WillNotTriggered.into()))
    );
    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Active);
    assert_eq!(TokenClient::new(env, &ctx.token).balance(&b), 0);
}

#[test]
fn trigger_and_release_in_same_ledger_is_rejected() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_simple(&ctx, &b);

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&id);
    assert_eq!(
        ctx.client.try_release_inheritance(&id, &None),
        Err(Ok(WillError::GracePeriodNotExpired.into()))
    );
    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Triggered);
    assert_eq!(TokenClient::new(env, &ctx.token).balance(&b), 0);
}

#[test]
fn emergency_checkin_before_release_blocks_release() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_simple(&ctx, &b);

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&id);
    env.ledger().set_timestamp(START + 92 * DAY);
    ctx.client.emergency_checkin(&id, &ctx.owner);
    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Active);

    env.ledger().set_timestamp(START + 100 * DAY);
    assert_eq!(
        ctx.client.try_release_inheritance(&id, &None),
        Err(Ok(WillError::WillNotTriggered.into()))
    );
    assert_eq!(TokenClient::new(env, &ctx.token).balance(&b), 0);
}

#[test]
fn release_after_grace_still_pays_out() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_simple(&ctx, &b);

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&id);
    env.ledger().set_timestamp(START + 100 * DAY);
    ctx.client.release_inheritance(&id, &None);

    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Released);
    assert_eq!(TokenClient::new(env, &ctx.token).balance(&b), BALANCE);
}

// ---------------------------------------------------------------------------
// #491 — bounded distribution
// ---------------------------------------------------------------------------

#[test]
fn release_at_beneficiary_cap_across_tokens_pays_everyone() {
    let ctx = setup();
    let env = &ctx.env;
    assert_eq!(crate::MAX_RELEASE_PAYOUTS, MAX_BENEFICIARIES * 10);

    let t2 = extra_token(&ctx);
    let t3 = extra_token(&ctx);
    let (beneficiaries, addresses) = cap_beneficiaries(&ctx);
    let id = create_with(
        &ctx,
        vec![
            env,
            (ctx.token.clone(), BALANCE),
            (t2.clone(), BALANCE),
            (t3.clone(), BALANCE),
        ],
        beneficiaries,
        0,
    );

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&id);
    env.ledger().set_timestamp(START + 100 * DAY);
    ctx.client.release_inheritance(&id, &None);
    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Released);

    let per_beneficiary = BALANCE / MAX_BENEFICIARIES as i128;
    for token in [ctx.token.clone(), t2, t3].iter() {
        let client = TokenClient::new(env, token);
        for a in addresses.iter() {
            assert_eq!(client.balance(&a), per_beneficiary);
        }
    }
}

#[test]
fn create_above_beneficiary_cap_is_rejected() {
    let ctx = setup();
    let env = &ctx.env;

    // 10 x 909 + 910 = 10_000 bps across MAX_BENEFICIARIES + 1 entries.
    let mut beneficiaries: Vec<Beneficiary> = Vec::new(env);
    for i in 0..(MAX_BENEFICIARIES + 1) {
        let bp = if i == MAX_BENEFICIARIES { 910 } else { 909 };
        beneficiaries.push_back(pct(&Address::generate(env), bp));
    }

    assert_eq!(
        ctx.client.try_create_will(
            &ctx.owner,
            &vec![env, (ctx.token.clone(), BALANCE)],
            &beneficiaries,
            &90,
            &7,
            &vec![env],
            &0,
            &None,
            &0,
        ),
        Err(Ok(WillError::TooManyBeneficiaries.into()))
    );
}

#[test]
fn release_refuses_over_cap_stored_will() {
    let ctx = setup();
    let env = &ctx.env;
    let (beneficiaries, _addresses) = cap_beneficiaries(&ctx);
    let id = create_with(
        &ctx,
        vec![env, (ctx.token.clone(), BALANCE)],
        beneficiaries,
        0,
    );

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&id);

    // Simulate a will stored under an older layout with too many entries.
    let mut will = ctx.client.get_will(&id);
    will.beneficiaries.push_back(pct(&Address::generate(env), 1));
    env.as_contract(&ctx.contract_id, || {
        crate::storage::save_will(env, &will);
    });

    env.ledger().set_timestamp(START + 100 * DAY);
    assert_eq!(
        ctx.client.try_release_inheritance(&id, &None),
        Err(Ok(WillError::TooManyBeneficiaries.into()))
    );
    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Triggered);
}

// ---------------------------------------------------------------------------
// #492 — owner pagination cursor validity
// ---------------------------------------------------------------------------

#[test]
fn owner_pagination_cursor_survives_additions_and_deletions() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);

    let w1 = create_simple(&ctx, &b);
    let w2 = create_simple(&ctx, &b);
    let w3 = create_simple(&ctx, &b);
    let w4 = create_simple(&ctx, &b);

    let page1 = owner_page(&ctx, None, 2);
    assert_eq!(page1, vec![env, w1, w2]);
    let cursor = w2;

    // Between pages: the cursor's own will leaves the index and a new will
    // is added.
    env.as_contract(&ctx.contract_id, || {
        crate::storage::remove_owner_index(env, &ctx.owner, cursor);
    });
    let w5 = create_simple(&ctx, &b);

    let page2 = owner_page(&ctx, Some(cursor), 2);
    assert_eq!(page2, vec![env, w3, w4], "stale cursor must not skip or repeat");

    let page3 = owner_page(&ctx, Some(w4), 2);
    assert_eq!(page3, vec![env, w5], "newly added will appears on a later page");

    let page4 = owner_page(&ctx, Some(w5), 2);
    assert_eq!(page4.len(), 0, "cursor at the last id yields an empty page");
}

#[test]
fn owner_pagination_out_of_bounds_cursor_returns_empty() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);

    let w1 = create_simple(&ctx, &b);
    let w2 = create_simple(&ctx, &b);

    assert_eq!(owner_page(&ctx, None, 10), vec![env, w1, w2]);
    assert_eq!(owner_page(&ctx, Some(w1), 10), vec![env, w2]);
    assert_eq!(owner_page(&ctx, Some(w2 + 1_000), 10).len(), 0);
    assert_eq!(owner_page(&ctx, Some(u64::MAX), 10).len(), 0);
    assert_eq!(owner_page(&ctx, None, 0).len(), 0);

    // An owner with no wills at all: any cursor yields an empty page.
    let stranger = Address::generate(env);
    let page = ctx.client.get_wills_by_owner(&stranger, &Some(w1), &10);
    assert_eq!(page.len(), 0);
}

// ---------------------------------------------------------------------------
// #493 — confirm_will re-validation
// ---------------------------------------------------------------------------

#[test]
fn confirm_rejected_for_non_owner() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_with(
        &ctx,
        vec![env, (ctx.token.clone(), BALANCE)],
        vec![env, pct(&b, 10_000)],
        30 * DAY,
    );
    assert_eq!(
        ctx.client.get_will(&id).status,
        WillStatus::PendingConfirmation
    );

    let stranger = Address::generate(env);
    assert_eq!(
        ctx.client.try_confirm_will(&id, &stranger),
        Err(Ok(WillError::NotOwner.into()))
    );
    assert_eq!(
        ctx.client.get_will(&id).status,
        WillStatus::PendingConfirmation
    );
}

#[test]
fn confirm_rechecks_ownership_after_owner_change() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_with(
        &ctx,
        vec![env, (ctx.token.clone(), BALANCE)],
        vec![env, pct(&b, 10_000)],
        30 * DAY,
    );

    // Ownership changes during the confirmation delay.
    let new_owner = Address::generate(env);
    let mut will = ctx.client.get_will(&id);
    will.owner = new_owner.clone();
    env.as_contract(&ctx.contract_id, || {
        crate::storage::save_will(env, &will);
    });

    env.ledger().set_timestamp(START + DAY);
    assert_eq!(
        ctx.client.try_confirm_will(&id, &ctx.owner),
        Err(Ok(WillError::NotOwner.into())),
        "the previous owner's confirmation must be rejected"
    );
    assert_eq!(
        ctx.client.get_will(&id).status,
        WillStatus::PendingConfirmation
    );

    ctx.client.confirm_will(&id, &new_owner);
    assert_eq!(ctx.client.get_will(&id).status, WillStatus::Active);
}

#[test]
fn confirm_revalidates_beneficiaries_changed_during_delay() {
    let ctx = setup();
    let env = &ctx.env;
    let b = Address::generate(env);
    let id = create_with(
        &ctx,
        vec![env, (ctx.token.clone(), BALANCE)],
        vec![env, pct(&b, 10_000)],
        30 * DAY,
    );

    // The stored list becomes invalid during the delay (duplicate address).
    let mut will = ctx.client.get_will(&id);
    will.beneficiaries = vec![env, pct(&b, 5_000), pct(&b, 5_000)];
    env.as_contract(&ctx.contract_id, || {
        crate::storage::save_will(env, &will);
    });

    assert_eq!(
        ctx.client.try_confirm_will(&id, &ctx.owner),
        Err(Ok(WillError::DuplicateBeneficiary.into()))
    );
    assert_eq!(
        ctx.client.get_will(&id).status,
        WillStatus::PendingConfirmation
    );
}
