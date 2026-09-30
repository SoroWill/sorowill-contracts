#![cfg(test)]

//! Tests for issues #502, #503, #504 and #505.
//!
//! All four were reported against code that has since changed, so these tests
//! exist to pin the behaviour each issue asks for and to show where it is
//! enforced. If any of these fail, the corresponding invariant has regressed.
//!
//! - **#502** — `merge_wills` must require both wills to be owned by the caller.
//!   Enforced by `load_owned` for `will_id_a` and an explicit `will_b.owner`
//!   check in `merge_wills`.
//! - **#503** — merging must consolidate a beneficiary present in both wills and
//!   sum their shares rather than emitting two entries. Enforced by
//!   `merge_beneficiaries`, which accumulates per address.
//! - **#504** — `split_will` must reject duplicate addresses and addresses that
//!   are not on the source will. Duplicates are caught by
//!   `split_uniqueness_check::assert_split_addresses_unique` (which raises
//!   `InvalidSplit`), and unknown addresses by the `BeneficiaryNotFound` check
//!   that follows it.
//! - **#505** — `release_inheritance` must reject a double release. Enforced by
//!   the `WillStatus::Triggered` assertion, because the first release moves the
//!   will to `Released`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

fn setup() -> (Env, WillContractClient<'static>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token = sac.address();
    StellarAssetClient::new(&env, &token).mint(&owner, &10_000_000_000);
    // A second owner, so the cross-owner merge tests have someone else's will to
    // try to merge in. It needs its own balance: `create_will` pulls the tokens
    // from the owner.
    let other = Address::generate(&env);
    StellarAssetClient::new(&env, &token).mint(&other, &10_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token, other)
}

/// Creates a will with an explicit beneficiary list, so tests can control which
/// addresses appear on it.
fn create_will_with(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
    amount: i128,
    beneficiaries: &[(Address, u32)],
) -> u64 {
    // Built by pushing rather than `vec![env, ..]`, which only takes a single
    // element or a slice and cannot take a mapped iterator.
    let mut list: soroban_sdk::Vec<Beneficiary> = soroban_sdk::Vec::new(env);
    for (address, bp) in beneficiaries {
        list.push_back(Beneficiary {
            address: address.clone(),
            allocation: Allocation::Percentage(*bp),
        });
    }

    client.create_will(
        owner,
        &vec![env, (token.clone(), amount)],
        &list,
        &90,
        &7,
        &vec![env],
        &1,
        &None,
        &0,
    )
}

/// `shared` plus a fresh address, each at 5_000bp so the total is 10_000.
fn pair(env: &Env, shared: &Address) -> [(Address, u32); 2] {
    [
        (shared.clone(), 5_000u32),
        (Address::generate(env), 5_000u32),
    ]
}

fn beneficiaries_of(client: &WillContractClient, will_id: u64) -> Vec<(Address, i128)> {
    client
        .get_will(&will_id)
        .beneficiaries
        .iter()
        .map(|b| match b.allocation {
            Allocation::Percentage(bp) => (b.address, bp as i128),
            Allocation::FixedAmount(amount) => (b.address, amount),
        })
        .collect()
}

// ── #502: merge_wills requires both wills to belong to the caller ───────────

#[test]
fn merge_rejects_a_will_owned_by_someone_else() {
    let (env, client, owner, token, other) = setup();
    let mine = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );
    let theirs = create_will_with(
        &env,
        &client,
        &other,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );

    // Merging my will with someone else's must be rejected even though I sign
    // the call: the attack in #502 is guessing another owner's will id.
    let err = client
        .try_merge_wills(&owner, &mine, &theirs)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::NotSameOwner.into());
}

#[test]
fn merge_rejects_when_only_one_side_is_mine() {
    let (env, client, owner, token, other) = setup();
    let theirs = create_will_with(
        &env,
        &client,
        &other,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );
    let mine = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );

    // Order must not matter: the argument order is the same either way round.
    let err = client
        .try_merge_wills(&owner, &theirs, &mine)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::NotOwner.into());
}

#[test]
fn merge_succeeds_when_both_wills_are_mine() {
    let (env, client, owner, token, _other) = setup();
    let a = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );
    let b = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        50_000,
        &[(Address::generate(&env), 10_000)],
    );

    client.merge_wills(&owner, &a, &b);
}

// ── #503: duplicate beneficiaries are consolidated, not doubled ─────────────

#[test]
fn merge_consolidates_a_beneficiary_present_in_both_wills() {
    let (env, client, owner, token, _other) = setup();
    let shared = Address::generate(&env);
    let only_a = Address::generate(&env);
    let only_b = Address::generate(&env);

    let a = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(shared.clone(), 4_000), (only_a.clone(), 6_000)],
    );
    let b = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(shared.clone(), 3_000), (only_b.clone(), 7_000)],
    );

    client.merge_wills(&owner, &a, &b);

    let merged = beneficiaries_of(&client, a);
    let shared_entries: Vec<_> = merged.iter().filter(|(addr, _)| addr == &shared).collect();

    // Exactly one entry: two entries would pay this address twice.
    assert_eq!(
        shared_entries.len(),
        1,
        "a beneficiary in both wills must be consolidated into a single entry, got {:?}",
        shared_entries
    );

    // 4_000bp of 100_000 plus 3_000bp of 100_000 = 7_000 shared, and the rest
    // renormalised across the two remaining addresses.
    let total: i128 = merged.iter().map(|(_, bp)| *bp).sum();
    assert_eq!(
        total, 10_000,
        "merged percentages must still total 100%: {merged:?}"
    );
    assert!(merged.iter().any(|(addr, _)| addr == &only_a));
    assert!(merged.iter().any(|(addr, _)| addr == &only_b));
}

#[test]
fn merge_does_not_duplicate_a_beneficiary_across_three_wills() {
    // Sequentially merging three wills that all name the same beneficiary must
    // still leave a single entry.
    let (env, client, owner, token, _other) = setup();
    let shared = Address::generate(&env);

    // Each will splits its beneficiaries across the shared address and a
    // second one so the percentages total 10_000 as the contract requires.
    let a = pair(&env, &shared);
    let b = pair(&env, &shared);
    let c = pair(&env, &shared);
    let a = create_will_with(&env, &client, &owner, &token, 10_000, &a);
    let b = create_will_with(&env, &client, &owner, &token, 10_000, &b);
    let c = create_will_with(&env, &client, &owner, &token, 10_000, &c);

    client.merge_wills(&owner, &a, &b);
    client.merge_wills(&owner, &a, &c);

    let merged = beneficiaries_of(&client, a);
    assert_eq!(
        merged.iter().filter(|(addr, _)| addr == &shared).count(),
        1,
        "still one entry after three merges: {merged:?}"
    );
    let total: i128 = merged.iter().map(|(_, bp)| *bp).sum();
    assert_eq!(total, 10_000, "percentages must total 100%: {merged:?}");
}

// ── #504: split_will rejects duplicate and unknown addresses ────────────────

#[test]
fn split_rejects_the_same_address_twice() {
    let (env, client, owner, token, _other) = setup();
    let beneficiary = Address::generate(&env);
    let will_id = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(beneficiary.clone(), 10_000)],
    );

    // #504's "new owner addresses must be unique": the same address listed twice
    // would otherwise collapse silently and leave source and child disagreeing
    // about how much moved.
    let dupes = vec![
        &env,
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(5_000),
        },
        Beneficiary {
            address: beneficiary.clone(),
            allocation: Allocation::Percentage(5_000),
        },
    ];

    let err = client
        .try_split_will(
            &will_id,
            &owner,
            &dupes,
            &vec![&env, (token.clone(), 10_000)],
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::InvalidSplit.into());
}

#[test]
fn split_rejects_an_address_that_is_not_a_beneficiary_of_the_source() {
    let (env, client, owner, token, _other) = setup();
    let on_will = Address::generate(&env);
    let stranger = Address::generate(&env);
    let will_id = create_will_with(&env, &client, &owner, &token, 100_000, &[(on_will, 10_000)]);

    // Splitting to an address the source never named would invent a beneficiary
    // on the child (#377).
    let strangers = vec![
        &env,
        Beneficiary {
            address: stranger,
            allocation: Allocation::Percentage(5_000),
        },
    ];

    let err = client
        .try_split_will(
            &will_id,
            &owner,
            &strangers,
            &vec![&env, (token.clone(), 10_000)],
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::BeneficiaryNotFound.into());
}

#[test]
fn split_succeeds_for_a_real_beneficiary() {
    let (env, client, owner, token, _other) = setup();
    let moved = Address::generate(&env);
    let staying = Address::generate(&env);
    // Two beneficiaries so the source still has one after the split; moving the
    // only beneficiary is rejected with InvalidSplit.
    let will_id = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(moved.clone(), 4_000), (staying, 6_000)],
    );

    let request = vec![
        &env,
        Beneficiary {
            address: moved,
            allocation: Allocation::Percentage(4_000),
        },
    ];

    let child = client.split_will(
        &will_id,
        &owner,
        &request,
        &vec![&env, (token.clone(), 10_000)],
    );
    assert!(child > 0);
}

// ── #505: release is not repeatable ────────────────────────────────────────

#[test]
fn release_rejects_a_second_call_on_the_same_will() {
    let (env, client, owner, token, _other) = setup();
    let will_id = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);

    client.release_inheritance(&will_id, &None);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);

    // The second call must fail: the will is no longer Triggered, so the
    // status assertion rejects it rather than distributing a second time.
    let err = client
        .try_release_inheritance(&will_id, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::WillNotTriggered.into());

    // And the will is gone from the triggered view, so a keeper iterating
    // get_triggered_wills cannot pick it up again.
    assert!(client.get_triggered_wills(&None, &50).is_empty());
}

#[test]
fn release_before_the_grace_deadline_is_rejected() {
    let (env, client, owner, token, _other) = setup();
    let will_id = create_will_with(
        &env,
        &client,
        &owner,
        &token,
        100_000,
        &[(Address::generate(&env), 10_000)],
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    // Still inside the 7-day grace period.
    env.ledger().with_mut(|l| l.timestamp += DAY);
    let err = client
        .try_release_inheritance(&will_id, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::GracePeriodNotExpired.into());
}
