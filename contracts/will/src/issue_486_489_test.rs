#![cfg(test)]

//! Regression tests for #486, #487, #488 and #489.
//!
//! - #486: `renounce_beneficiary` redistribution never exceeds 10_000 bps and
//!   the last remaining percentage beneficiary absorbs the rounding remainder,
//!   including when the renounced share is a prime number of basis points.
//! - #487: the `renounce` event carries the will's `trigger_time`, and the
//!   payout at release matches the split that event published.
//! - #488: a guardian-list update cannot wipe guardian-cancel votes that are
//!   still in flight.
//! - #489: `update_guardians_weighted` rejects zero weights, and the stored
//!   threshold stays valid against the weights actually stored.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Symbol, TryIntoVal, Vec,
};

use crate::{
    Allocation, Beneficiary, GuardianSpec, WillContract, WillContractClient, WillError,
    WillStatus,
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
    StellarAssetClient::new(&env, &token).mint(&owner, &BALANCE);

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

fn create(
    ctx: &Ctx,
    beneficiaries: Vec<Beneficiary>,
    guardians: Vec<Address>,
    threshold: u32,
) -> u64 {
    ctx.client.create_will(
        &ctx.owner,
        &vec![&ctx.env, (ctx.token.clone(), BALANCE)],
        &beneficiaries,
        &90,
        &7,
        &guardians,
        &threshold,
        &None,
        &0,
    )
}

fn pct(address: &Address, bp: u32) -> Beneficiary {
    Beneficiary {
        address: address.clone(),
        allocation: Allocation::Percentage(bp),
    }
}

fn spec(address: &Address, weight: u32) -> GuardianSpec {
    GuardianSpec {
        address: address.clone(),
        weight,
    }
}

fn bp_of(list: &Vec<Beneficiary>, address: &Address) -> u32 {
    for b in list.iter() {
        if &b.address == address {
            if let Allocation::Percentage(bp) = b.allocation {
                return bp;
            }
        }
    }
    panic!("no percentage beneficiary for address");
}

fn pct_total(list: &Vec<Beneficiary>) -> u32 {
    let mut total: u32 = 0;
    for b in list.iter() {
        if let Allocation::Percentage(bp) = b.allocation {
            total += bp;
        }
    }
    total
}

/// Returns the data tuple of the `renounce` event for `will_id`. Must be
/// called immediately after `renounce_beneficiary`, before any other
/// contract call.
fn renounce_event(
    ctx: &Ctx,
    will_id: u64,
) -> (Address, Address, Vec<Beneficiary>, Option<u64>) {
    let target: Symbol = symbol_short!("renounce");
    for (emitter, topics, data) in ctx.env.events().all().iter() {
        if emitter != ctx.contract_id || topics.len() != 2 {
            continue;
        }
        let t0: Result<Symbol, _> = topics.get(0).unwrap().try_into_val(&ctx.env);
        match t0 {
            Ok(s) if s == target => {}
            _ => continue,
        }
        let t1: u64 = topics.get(1).unwrap().try_into_val(&ctx.env).unwrap();
        if t1 == will_id {
            return data.try_into_val(&ctx.env).unwrap();
        }
    }
    panic!("renounce event not found");
}

// ---------------------------------------------------------------------------
// #486 — redistribution cap and remainder absorption
// ---------------------------------------------------------------------------

/// Renounced share 3_001 bps (prime) across 3_333 / 3_333 / 333.
/// Floors: 1_429, 1_429 and 142, which together leave 1 bp over. The last
/// entry gets it (476 instead of its floor share of 475), and the total is
/// exactly 10_000.
#[test]
fn prime_renounced_share_last_recipient_absorbs_overage() {
    let ctx = setup();
    let env = &ctx.env;
    let a = Address::generate(env);
    let b = Address::generate(env);
    let c = Address::generate(env);
    let d = Address::generate(env);

    let will_id = create(
        &ctx,
        vec![env, pct(&a, 3_001), pct(&b, 3_333), pct(&c, 3_333), pct(&d, 333)],
        vec![env],
        0,
    );

    ctx.client.renounce_beneficiary(&will_id, &a);

    let list = ctx.client.get_will(&will_id).beneficiaries;
    assert_eq!(list.len(), 3);
    assert_eq!(bp_of(&list, &b), 4_762);
    assert_eq!(bp_of(&list, &c), 4_762);
    assert_eq!(bp_of(&list, &d), 476);
    assert_eq!(pct_total(&list), 10_000);
}

/// Sweep of prime renounced shares: the total always lands on exactly
/// 10_000, no remaining entry ever loses basis points, and the last entry
/// receives at least its floor share plus at most one bp per earlier entry.
#[test]
fn prime_renounced_shares_never_exceed_cap() {
    let primes: [u32; 8] = [2, 3, 7, 13, 101, 997, 3_001, 7_919];

    for p in primes.iter() {
        let ctx = setup();
        let env = &ctx.env;
        let renouncer = Address::generate(env);
        let x = Address::generate(env);
        let y = Address::generate(env);
        let z = Address::generate(env);

        let rest = 10_000 - *p;
        let r1 = rest / 3;
        let r2 = rest / 3;
        let r3 = rest - r1 - r2;

        let will_id = create(
            &ctx,
            vec![env, pct(&renouncer, *p), pct(&x, r1), pct(&y, r2), pct(&z, r3)],
            vec![env],
            0,
        );

        ctx.client.renounce_beneficiary(&will_id, &renouncer);

        let list = ctx.client.get_will(&will_id).beneficiaries;
        assert_eq!(pct_total(&list), 10_000, "total for prime {}", p);
        assert!(bp_of(&list, &x) >= r1);
        assert!(bp_of(&list, &y) >= r2);

        let z_floor = r3 + ((*p as u64 * r3 as u64) / rest as u64) as u32;
        let z_new = bp_of(&list, &z);
        assert!(z_new >= z_floor, "last entry under-paid for prime {}", p);
        assert!(z_new - z_floor <= 2, "last entry over-absorbed for prime {}", p);
    }
}

/// The *last percentage* entry absorbs the remainder even when a
/// `FixedAmount` entry sits after it, and the fixed entry is untouched.
#[test]
fn remainder_goes_to_last_percentage_entry_not_fixed_entry() {
    let ctx = setup();
    let env = &ctx.env;
    let a = Address::generate(env);
    let b = Address::generate(env);
    let c = Address::generate(env);
    let f = Address::generate(env);

    let will_id = create(
        &ctx,
        vec![
            env,
            pct(&a, 13),
            pct(&b, 4_993),
            pct(&c, 4_994),
            Beneficiary {
                address: f.clone(),
                allocation: Allocation::FixedAmount(100_000),
            },
        ],
        vec![env],
        0,
    );

    ctx.client.renounce_beneficiary(&will_id, &a);

    let list = ctx.client.get_will(&will_id).beneficiaries;
    assert_eq!(bp_of(&list, &b), 4_999);
    assert_eq!(bp_of(&list, &c), 5_001);
    assert_eq!(pct_total(&list), 10_000);

    let fixed = list.iter().find(|e| e.address == f).unwrap();
    assert_eq!(fixed.allocation, Allocation::FixedAmount(100_000));
}

// ---------------------------------------------------------------------------
// #487 — grace-cycle context on the renounce event
// ---------------------------------------------------------------------------

#[test]
fn renounce_while_active_emits_no_trigger_time() {
    let ctx = setup();
    let env = &ctx.env;
    let a = Address::generate(env);
    let b = Address::generate(env);

    let will_id = create(&ctx, vec![env, pct(&a, 5_000), pct(&b, 5_000)], vec![env], 0);

    ctx.client.renounce_beneficiary(&will_id, &a);
    let (who, owner, split, trigger_time) = renounce_event(&ctx, will_id);

    assert_eq!(who, a);
    assert_eq!(owner, ctx.owner);
    assert_eq!(trigger_time, None);
    assert_eq!(split, ctx.client.get_will(&will_id).beneficiaries);
}

/// Renounce during the grace period: the event's `trigger_time` matches the
/// will's, and the actual payout at release matches the split in the event.
#[test]
fn renounce_during_grace_period_records_trigger_time_and_payout_matches_event() {
    let ctx = setup();
    let env = &ctx.env;
    let a = Address::generate(env);
    let b = Address::generate(env);
    let c = Address::generate(env);
    let d = Address::generate(env);

    let will_id = create(
        &ctx,
        vec![env, pct(&a, 7), pct(&b, 3_331), pct(&c, 3_331), pct(&d, 3_331)],
        vec![env],
        0,
    );

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&will_id);
    let will = ctx.client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Triggered);
    let stored_trigger_time = will.trigger_time;
    assert!(stored_trigger_time.is_some());

    env.ledger().set_timestamp(START + 92 * DAY);
    ctx.client.renounce_beneficiary(&will_id, &a);
    let (who, _owner, split, event_trigger_time) = renounce_event(&ctx, will_id);

    assert_eq!(who, a);
    assert_eq!(event_trigger_time, stored_trigger_time);
    assert_eq!(split, ctx.client.get_will(&will_id).beneficiaries);
    assert_eq!(bp_of(&split, &b), 3_333);
    assert_eq!(bp_of(&split, &c), 3_333);
    assert_eq!(bp_of(&split, &d), 3_334);

    // Past the grace period: release and compare payouts with the event.
    env.ledger().set_timestamp(START + 100 * DAY);
    ctx.client.release_inheritance(&will_id, &None);
    assert_eq!(ctx.client.get_will(&will_id).status, WillStatus::Released);

    let token = TokenClient::new(env, &ctx.token);
    assert_eq!(token.balance(&a), 0, "renounced beneficiary must receive nothing");

    let mut paid: i128 = 0;
    for entry in split.iter() {
        if let Allocation::Percentage(bp) = entry.allocation {
            let expected = BALANCE * bp as i128 / 10_000;
            assert_eq!(token.balance(&entry.address), expected);
            paid += expected;
        }
    }
    assert_eq!(paid, BALANCE);
}

// ---------------------------------------------------------------------------
// #488 — guardian update vs in-flight cancel votes
// ---------------------------------------------------------------------------

/// Concurrent scenario: a cancel vote is accumulating during the grace
/// period. Guardian updates are refused, the cancel vote survives, and the
/// cancel still reaches quorum. Only after it resolves is an update accepted.
#[test]
fn guardian_update_cannot_wipe_accumulating_cancel_vote() {
    let ctx = setup();
    let env = &ctx.env;
    let beneficiary = Address::generate(env);
    let ga = Address::generate(env);
    let gb = Address::generate(env);
    let newcomer = Address::generate(env);

    let will_id = create(
        &ctx,
        vec![env, pct(&beneficiary, 10_000)],
        vec![env, ga.clone(), gb.clone()],
        2,
    );
    ctx.client.accept_guardian_role(&will_id, &ga);
    ctx.client.accept_guardian_role(&will_id, &gb);

    env.ledger().set_timestamp(START + 91 * DAY);
    ctx.client.trigger_will(&will_id);

    ctx.client.guardian_cancel_trigger(&will_id, &ga);
    assert_eq!(ctx.client.get_will(&will_id).guardian_cancel_votes, 1);

    assert_eq!(
        ctx.client.try_update_guardians(
            &will_id,
            &ctx.owner,
            &vec![env, ga.clone(), newcomer.clone()],
        ),
        Err(Ok(WillError::WillNotActive.into()))
    );
    assert_eq!(
        ctx.client.try_update_guardians_weighted(
            &will_id,
            &ctx.owner,
            &vec![env, spec(&newcomer, 1)],
            &Some(1),
        ),
        Err(Ok(WillError::WillNotActive.into()))
    );

    let will = ctx.client.get_will(&will_id);
    assert_eq!(will.guardian_cancel_votes, 1, "cancel vote must survive");
    assert_eq!(will.guardians.len(), 2);

    ctx.client.guardian_cancel_trigger(&will_id, &gb);
    let will = ctx.client.get_will(&will_id);
    assert_eq!(will.status, WillStatus::Active);
    assert_eq!(will.guardian_cancel_votes, 0);

    ctx.client
        .update_guardians(&will_id, &ctx.owner, &vec![env, ga.clone(), newcomer.clone()]);
    assert_eq!(ctx.client.get_will(&will_id).guardians.len(), 2);
}

/// Defensive guard: if an `Active` will ever carries cancel votes, every
/// guardian-replacing entry point refuses rather than wiping them.
#[test]
fn guardian_update_rejected_when_cancel_votes_recorded_on_active_will() {
    let ctx = setup();
    let env = &ctx.env;
    let beneficiary = Address::generate(env);
    let ga = Address::generate(env);
    let gb = Address::generate(env);
    let newcomer = Address::generate(env);

    let will_id = create(
        &ctx,
        vec![env, pct(&beneficiary, 10_000)],
        vec![env, ga.clone(), gb.clone()],
        2,
    );

    let mut will = ctx.client.get_will(&will_id);
    will.guardian_cancel_votes = 1;
    will.guardian_cancel_vote_weight = 1;
    env.as_contract(&ctx.contract_id, || {
        crate::storage::save_will(env, &will);
    });

    assert_eq!(
        ctx.client.try_update_guardians(
            &will_id,
            &ctx.owner,
            &vec![env, ga.clone(), newcomer.clone()],
        ),
        Err(Ok(WillError::GuardianCancelInProgress.into()))
    );
    assert_eq!(
        ctx.client.try_update_guardians_weighted(
            &will_id,
            &ctx.owner,
            &vec![env, spec(&ga, 1), spec(&newcomer, 1)],
            &Some(2),
        ),
        Err(Ok(WillError::GuardianCancelInProgress.into()))
    );
    assert_eq!(
        ctx.client.try_update_will_settings(
            &will_id,
            &ctx.owner,
            &None,
            &Some(vec![env, ga.clone(), newcomer.clone()]),
            &None,
            &None,
        ),
        Err(Ok(WillError::GuardianCancelInProgress.into()))
    );

    let will = ctx.client.get_will(&will_id);
    assert_eq!(will.guardian_cancel_votes, 1);
    assert_eq!(will.guardians.get(1).unwrap().address, gb);
}

// ---------------------------------------------------------------------------
// #489 — zero weights and threshold integrity
// ---------------------------------------------------------------------------

#[test]
fn zero_weight_guardian_is_rejected() {
    let ctx = setup();
    let env = &ctx.env;
    let beneficiary = Address::generate(env);
    let g1 = Address::generate(env);
    let g2 = Address::generate(env);

    let will_id = create(&ctx, vec![env, pct(&beneficiary, 10_000)], vec![env], 0);

    assert_eq!(
        ctx.client
            .try_update_guardians_weighted(&will_id, &ctx.owner, &vec![env, spec(&g1, 0)], &Some(1)),
        Err(Ok(WillError::InvalidGuardianThreshold.into()))
    );
    assert_eq!(
        ctx.client.try_update_guardians_weighted(
            &will_id,
            &ctx.owner,
            &vec![env, spec(&g1, 2), spec(&g2, 0)],
            &Some(2),
        ),
        Err(Ok(WillError::InvalidGuardianThreshold.into()))
    );
    assert_eq!(ctx.client.get_will(&will_id).guardians.len(), 0);
}

#[test]
fn threshold_holds_after_weighted_guardian_update() {
    let ctx = setup();
    let env = &ctx.env;
    let beneficiary = Address::generate(env);
    let g1 = Address::generate(env);
    let g2 = Address::generate(env);

    let will_id = create(&ctx, vec![env, pct(&beneficiary, 10_000)], vec![env], 0);

    ctx.client.update_guardians_weighted(
        &will_id,
        &ctx.owner,
        &vec![env, spec(&g1, 2), spec(&g2, 3)],
        &Some(5),
    );
    let will = ctx.client.get_will(&will_id);
    assert_eq!(will.guardian_threshold, 5);
    assert_eq!(will.guardians.get(0).unwrap().weight, 2);
    assert_eq!(will.guardians.get(1).unwrap().weight, 3);

    // Above the stored total weight: rejected.
    assert_eq!(
        ctx.client.try_update_guardians_weighted(
            &will_id,
            &ctx.owner,
            &vec![env, spec(&g1, 2), spec(&g2, 3)],
            &Some(6),
        ),
        Err(Ok(WillError::InvalidGuardianThreshold.into()))
    );

    // Omitting the threshold keeps 5, still reachable with 2 + 3.
    ctx.client.update_guardians_weighted(
        &will_id,
        &ctx.owner,
        &vec![env, spec(&g1, 2), spec(&g2, 3)],
        &None,
    );
    assert_eq!(ctx.client.get_will(&will_id).guardian_threshold, 5);

    // Shrinking total weight below the stored threshold: rejected.
    assert_eq!(
        ctx.client.try_update_guardians_weighted(
            &will_id,
            &ctx.owner,
            &vec![env, spec(&g1, 2), spec(&g2, 2)],
            &None,
        ),
        Err(Ok(WillError::InvalidGuardianThreshold.into()))
    );
}
