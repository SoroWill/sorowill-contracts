#![cfg(test)]

//! Regression tests for issues #378-#381, all in `merge_wills` and its
//! payout path:
//!
//! - #378 `distribute` computed the keeper bounty from the first token whose
//!   share rounded above zero but paid it out of the *first* entry of
//!   `transfer_plan`, whose balance had never been reduced.
//! - #379 the merge deduplicated guardians by comparing whole `Guardian`
//!   structs, so one address recorded with different weight or consent was
//!   appended twice.
//! - #380 the consumed will's hashed beneficiaries and their committed
//!   percentages were silently dropped while its balance moved on.
//! - #381 the consumed will moved `Active` → `Cancelled` without recording a
//!   transition, so `get_will_history` showed only its creation.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Bytes, Env, Vec as SorobanVec,
};

use crate::{
    Allocation, Beneficiary, GuardianConsent, GuardianSpec, WillContract, WillContractClient,
    WillError, WillStatus,
};

const DAY: u64 = 86_400;

/// Highest `keeper_bounty_bps` `create_will` accepts (1%).
const MAX_BOUNTY_BPS: u32 = 100;

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

fn mint(env: &Env, owner: &Address, amount: i128) -> Address {
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let address = sac.address();
    StellarAssetClient::new(env, &address).mint(owner, &amount);
    address
}

fn beneficiary(env: &Env, bps: u32) -> Beneficiary {
    Beneficiary {
        address: Address::generate(env),
        allocation: Allocation::Percentage(bps),
    }
}

fn consent_of(client: &WillContractClient<'_>, will_id: u64, addr: &Address) -> GuardianConsent {
    for g in client.get_will(&will_id).guardians.iter() {
        if g.address == *addr {
            return g.consent;
        }
    }
    panic!("guardian not found on will");
}

fn weight_of(client: &WillContractClient<'_>, will_id: u64, addr: &Address) -> u32 {
    for g in client.get_will(&will_id).guardians.iter() {
        if g.address == *addr {
            return g.weight;
        }
    }
    panic!("guardian not found on will");
}

fn count_matching(client: &WillContractClient<'_>, will_id: u64, addr: &Address) -> u32 {
    let mut n = 0;
    for g in client.get_will(&will_id).guardians.iter() {
        if g.address == *addr {
            n += 1;
        }
    }
    n
}

// ------------------------------------------------------------------ #381

/// Issue #381: the consumed will really does move `Active` -> `Cancelled` in a
/// merge, so its audit trail must record that. Before the fix
/// `get_will_history` showed only the will's `create` transition, making a
/// merge indistinguishable from a will that had been left alone.
#[test]
fn merge_records_cancellation_transition_for_consumed_will() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = mint(&env, &owner, 10_000_000);
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));

    let a = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
        &vec![&env, beneficiary(&env, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    let b = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 2_000_000_i128)],
        &vec![&env, beneficiary(&env, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // Before the merge the consumed will's trail holds only its creation.
    let before = client.get_will_history(&b);
    assert_eq!(before.len(), 1);
    assert_eq!(before.get_unchecked(0).to_status, WillStatus::Active);

    client.merge_wills(&owner, &a, &b);

    let consumed = client.get_will_history(&b);
    assert_eq!(consumed.len(), 2, "merge must add one entry");
    let cancel = consumed.get_unchecked(1);
    assert_eq!(cancel.from_status, WillStatus::Active);
    assert_eq!(cancel.to_status, WillStatus::Cancelled);
    assert_eq!(cancel.action, symbol_short!("merge"));
    assert_eq!(cancel.actor, owner);
    assert_eq!(cancel.will_id, b);

    // The survivor keeps `Active`, but a merge rewrites its balance,
    // beneficiaries, guardians and periods, so it gets a `merge` entry of the
    // same `Active` -> `Active` shape `create_will` and `split_will` record.
    let survivor = client.get_will_history(&a);
    assert_eq!(survivor.len(), 2, "survivor must get an entry too");
    let merge = survivor.get_unchecked(1);
    assert_eq!(merge.from_status, WillStatus::Active);
    assert_eq!(merge.to_status, WillStatus::Active);
    assert_eq!(merge.action, symbol_short!("merge"));
    assert_eq!(merge.will_id, a);
}

// ------------------------------------------------------------------ #380

/// Issue #380: the consumed will's hashed beneficiaries and their committed
/// percentages were dropped while its balance moved to the survivor. The merge
/// is now rejected until every commitment has revealed and claimed.
#[test]
fn merge_rejected_when_consumed_will_has_hashed_beneficiary() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = mint(&env, &owner, 10_000_000);
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));

    let a = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
        &vec![&env, beneficiary(&env, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    let b = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 2_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::FixedAmount(1_800_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    // The consumed will holds an unrevealed commitment for 10%.
    client.add_hashed_beneficiary(&b, &owner, &Bytes::from_slice(&env, &[7u8; 32]), &1_000);

    let err = client.try_merge_wills(&owner, &a, &b).unwrap_err().unwrap();
    assert_eq!(err, WillError::MergeWithHashedBeneficiaries.into());

    // Rejected before any mutation: both wills are untouched and still active.
    assert_eq!(client.get_will(&a).balance, 1_000_000);
    assert_eq!(client.get_will(&b).balance, 2_000_000);
    assert_eq!(client.get_will_status(&b), WillStatus::Active);
}

/// The guard is symmetric: an unrevealed commitment on the *surviving* will
/// would also be silently re-based against the combined balance.
#[test]
fn merge_rejected_when_surviving_will_has_hashed_beneficiary() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = mint(&env, &owner, 10_000_000);
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));

    let a = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::FixedAmount(900_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );
    let b = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 2_000_000_i128)],
        &vec![&env, beneficiary(&env, 10_000)],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    client.add_hashed_beneficiary(&a, &owner, &Bytes::from_slice(&env, &[9u8; 32]), &1_000);

    let err = client.try_merge_wills(&owner, &a, &b).unwrap_err().unwrap();
    assert_eq!(err, WillError::MergeWithHashedBeneficiaries.into());
}

// ------------------------------------------------------------------ #379

/// Creates two single-beneficiary wills and installs `a_specs` / `b_specs` as
/// their weighted guardian lists, so the two wills can disagree about the same
/// guardian address. Returns `(will_a, will_b)`.
fn two_wills_with_guardians<'a>(
    env: &Env,
    client: &WillContractClient<'a>,
    owner: &Address,
    token: &Address,
    a_specs: SorobanVec<GuardianSpec>,
    b_specs: SorobanVec<GuardianSpec>,
) -> (u64, u64) {
    let a = client.create_will(
        owner,
        &vec![env, (token.clone(), 1_000_000_i128)],
        &vec![env, beneficiary(env, 10_000)],
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    let b = client.create_will(
        owner,
        &vec![env, (token.clone(), 2_000_000_i128)],
        &vec![env, beneficiary(env, 10_000)],
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    );
    client.update_guardians_weighted(&a, owner, &a_specs, &Some(1));
    client.update_guardians_weighted(&b, owner, &b_specs, &Some(1));
    (a, b)
}

/// Issue #379: the merge used `merged_guardians.contains(&guardian)` on whole
/// `Guardian` structs. The same address recorded with a different weight or
/// consent compared unequal and was appended a second time, breaking the
/// no-duplicate-guardian rule `assert_valid_guardians` enforces everywhere
/// else and double-counting that guardian's weight toward quorum.
#[test]
fn merge_matches_guardians_by_address_not_by_whole_struct() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = mint(&env, &owner, 10_000_000);
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));

    let shared = Address::generate(&env);
    let only_a = Address::generate(&env);
    let only_b = Address::generate(&env);

    // Same address, different weight on each side, and a different consent
    // state: exactly the case a whole-struct comparison splits into two.
    let (a, b) = two_wills_with_guardians(
        &env,
        &client,
        &owner,
        &token,
        vec![
            &env,
            GuardianSpec {
                address: shared.clone(),
                weight: 2,
            },
            GuardianSpec {
                address: only_a.clone(),
                weight: 1,
            },
        ],
        vec![
            &env,
            GuardianSpec {
                address: shared.clone(),
                weight: 5,
            },
            GuardianSpec {
                address: only_b.clone(),
                weight: 1,
            },
        ],
    );

    client.accept_guardian_role(&b, &shared.clone());
    assert_eq!(consent_of(&client, b, &shared), GuardianConsent::Accepted);

    client.merge_wills(&owner, &a, &b);

    let merged = client.get_will(&a);
    assert_eq!(
        count_matching(&client, a, &shared),
        1,
        "a shared guardian must appear exactly once, not twice"
    );
    assert_eq!(merged.guardians.len(), 3, "one shared plus two unique");
    assert_eq!(count_matching(&client, a, &only_a), 1);
    assert_eq!(count_matching(&client, a, &only_b), 1);

    // weight: the greater of the two (documented rule) — still a single entry,
    // so the guardian's weight counts once toward quorum.
    assert_eq!(weight_of(&client, a, &shared), 5);
    // consent: the more advanced of the two, `Accepted` > `Pending`.
    assert_eq!(consent_of(&client, a, &shared), GuardianConsent::Accepted);
}

/// A guardian who declined on both wills stays `Rejected` — terminal for them,
/// so they cannot vote on the merged will until the owner re-appoints them
/// through `update_guardians`.
#[test]
fn merge_keeps_rejected_consent_when_rejected_on_both_wills() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = mint(&env, &owner, 10_000_000);
    let client = WillContractClient::new(&env, &env.register(WillContract, ()));

    let shared = Address::generate(&env);
    let (a, b) = two_wills_with_guardians(
        &env,
        &client,
        &owner,
        &token,
        vec![
            &env,
            GuardianSpec {
                address: shared.clone(),
                weight: 1,
            },
        ],
        vec![
            &env,
            GuardianSpec {
                address: shared.clone(),
                weight: 1,
            },
        ],
    );

    client.reject_guardian_role(&a, &shared.clone());
    client.reject_guardian_role(&b, &shared.clone());
    assert_eq!(consent_of(&client, a, &shared), GuardianConsent::Rejected);
    assert_eq!(consent_of(&client, b, &shared), GuardianConsent::Rejected);

    client.merge_wills(&owner, &a, &b);

    assert_eq!(count_matching(&client, a, &shared), 1);
    assert_eq!(consent_of(&client, a, &shared), GuardianConsent::Rejected);
}

// ------------------------------------------------------------------ #378

/// Issues #378: the bounty was computed from the first token whose share
/// rounded above zero but paid out of the *first* entry of `transfer_plan`,
/// whose balance had never been reduced — so the keeper was paid out of
/// beneficiaries' funds, or the release aborted outright.
///
/// This will's first token holds only 50 units, whose 1% bounty rounds to 0;
/// the second token's rounds to a non-zero amount, so the two tokens
/// desynchronise under the old code.
#[test]
fn keeper_bounty_is_paid_from_the_token_it_was_computed_from() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let primary = mint(&env, &owner, 100);
    let secondary = mint(&env, &owner, 1_000_000);
    let keeper = Address::generate(&env);
    let heir = Address::generate(&env);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let will_id = client.create_will(
        &owner,
        &vec![
            &env,
            (primary.clone(), 50_i128),
            (secondary.clone(), 100_000_i128),
        ],
        &vec![
            &env,
            Beneficiary {
                address: heir.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &Some(MAX_BOUNTY_BPS),
        &0,
    );

    // Sanity: the first token's bounty share does round to zero, the second's
    // does not. If this stops holding the test no longer exercises #378.
    assert_eq!(50 * MAX_BOUNTY_BPS as i128 / 10_000, 0);
    assert_eq!(100_000 * MAX_BOUNTY_BPS as i128 / 10_000, 1_000);

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &Some(keeper.clone()));

    let expected_bounty = 100_000 * MAX_BOUNTY_BPS as i128 / 10_000;
    let primary_client = TokenClient::new(&env, &primary);
    let secondary_client = TokenClient::new(&env, &secondary);

    // The owner minted 100 of the primary and 1_000_000 of the secondary, and
    // locked 50 and 100_000 respectively, so anything beyond the lock that has
    // come back is a refund the release did not owe.
    let primary_before = 100 - 50;
    let secondary_before = 1_000_000 - 100_000;
    assert_eq!(primary_client.balance(&owner), primary_before);
    assert_eq!(secondary_client.balance(&owner), secondary_before);

    // Paid out of the very token it was computed from.
    assert_eq!(secondary_client.balance(&keeper), expected_bounty);

    // The tiny first token is untouched: no bounty was taken from it, and its
    // full 50 units reach the heir.
    assert_eq!(primary_client.balance(&keeper), 0);
    assert_eq!(primary_client.balance(&heir), 50);

    // The second token's remainder reaches the heir in full, so the payout did
    // not come out of the beneficiaries' share.
    assert_eq!(secondary_client.balance(&heir), 100_000 - expected_bounty);

    // Contract holds nothing: the bounty and both shares are all accounted for.
    assert_eq!(primary_client.balance(&contract_id), 0);
    assert_eq!(secondary_client.balance(&contract_id), 0);
}

/// Documented rounding: when *every* token's bounty share rounds to zero, no
/// bounty is paid at all and no beneficiary share is reduced to make room.
#[test]
fn no_bounty_is_paid_when_every_token_rounds_to_zero() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let primary = mint(&env, &owner, 100);
    let secondary = mint(&env, &owner, 100);
    let keeper = Address::generate(&env);
    let heir = Address::generate(&env);

    let client = WillContractClient::new(&env, &env.register(WillContract, ()));
    let will_id = client.create_will(
        &owner,
        &vec![
            &env,
            (primary.clone(), 50_i128),
            (secondary.clone(), 50_i128),
        ],
        &vec![
            &env,
            Beneficiary {
                address: heir.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &Some(MAX_BOUNTY_BPS),
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &Some(keeper.clone()));

    assert_eq!(TokenClient::new(&env, &primary).balance(&keeper), 0);
    assert_eq!(TokenClient::new(&env, &secondary).balance(&keeper), 0);
    // Both tokens go to the heir in full.
    assert_eq!(TokenClient::new(&env, &primary).balance(&heir), 50);
    assert_eq!(TokenClient::new(&env, &secondary).balance(&heir), 50);
}

/// A single token whose bounty rounds above zero still pays exactly as before:
/// the fix must not change the ordinary one-token case.
#[test]
fn single_token_bounty_is_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = mint(&env, &owner, 1_000_000);
    let keeper = Address::generate(&env);
    let heir = Address::generate(&env);

    let client = WillContractClient::new(&env, &env.register(WillContract, ()));
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: heir.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &Some(MAX_BOUNTY_BPS),
        &0,
    );

    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &Some(keeper.clone()));

    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&keeper), 10_000);
    assert_eq!(token_client.balance(&heir), 990_000);
}
