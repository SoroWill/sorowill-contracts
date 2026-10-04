#![cfg(test)]

//! Regression coverage for issue #500: check-ins update `will.last_checkin`
//! but, before this fix, never touched the on-chain audit trail
//! (`WillHistory`) at all -- only actions that also changed `status` did, via
//! `record_transition`. That left the trail silent about check-ins even
//! though they are the most frequent lifecycle event a will sees, and the
//! trail's only ordering was an entry's *position*, which is not stable: once
//! `storage::MAX_HISTORY_ENTRIES` starts trimming the oldest entry, every
//! later entry's position shifts, so a caller who cached "entry at index 3"
//! silently ends up looking at a different entry later, rather than seeing a
//! gap.
//!
//! The fix: `check_in` and `batch_check_in` now append a same-status
//! (`Active` -> `Active`) entry via the same `record_transition` path
//! `create_will` already uses for its own initial self-transition (#351), and
//! every `WillStatusTransition` -- check-in or status change alike -- carries
//! a monotonically increasing, per-will `seq` assigned by
//! `storage::append_history` that never changes once written, so trimming
//! produces a detectable gap in `seq` instead of silently reusing or
//! shuffling identities.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::storage::MAX_HISTORY_ENTRIES;
use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

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

    (env.clone(), client, owner, token_address)
}

fn create_basic_will(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token_address: &Address,
) -> u64 {
    let beneficiary = Address::generate(env);
    client.create_will(
        owner,
        &vec![env, (token_address.clone(), 100_000_i128)],
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
        &0,
        &None,
        &0,
    )
}

#[test]
fn check_in_appends_a_sequenced_history_entry() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_basic_will(&env, &client, &owner, &token_address);

    let before = client.get_will_history(&will_id);
    assert_eq!(before.len(), 1, "create_will records its own initial self-transition");
    assert_eq!(before.get(0).unwrap().seq, 1);

    client.check_in(&will_id, &owner);

    let after = client.get_will_history(&will_id);
    assert_eq!(
        after.len(),
        2,
        "check_in must append its own entry, not just update last_checkin silently"
    );
    let entry = after.get(1).unwrap();
    assert_eq!(entry.from_status, WillStatus::Active);
    assert_eq!(entry.to_status, WillStatus::Active);
    assert_eq!(
        entry.seq, 2,
        "seq must continue from create_will's entry, not restart"
    );
}

#[test]
fn batch_check_in_appends_one_entry_per_will_with_independent_sequences() {
    let (env, client, owner, token_address) = setup();
    let will_1 = create_basic_will(&env, &client, &owner, &token_address);
    let will_2 = create_basic_will(&env, &client, &owner, &token_address);

    client.batch_check_in(&vec![&env, will_1, will_2], &owner);

    for will_id in [will_1, will_2] {
        let history = client.get_will_history(&will_id);
        assert_eq!(
            history.len(),
            2,
            "each will gets its own create entry plus its own checkin entry"
        );
        assert_eq!(history.get(0).unwrap().seq, 1);
        assert_eq!(
            history.get(1).unwrap().seq,
            2,
            "each will's seq is independent of the other will's, not shared"
        );
    }
}

#[test]
fn seq_continues_monotonically_across_a_status_change() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_basic_will(&env, &client, &owner, &token_address);

    client.check_in(&will_id, &owner);
    client.check_in(&will_id, &owner);

    // Advance past the check-in deadline and trigger the will, then prove
    // alive via emergency_checkin -- both are pre-existing record_transition
    // call sites, which must now share the same continuous sequence as the
    // checkin entries rather than a separate or restarted one.
    env.ledger().with_mut(|l| l.timestamp += 91 * 86_400);
    client.trigger_will(&will_id);
    client.emergency_checkin(&will_id, &owner);

    let history = client.get_will_history(&will_id);
    let seqs: std::vec::Vec<u64> = history.iter().map(|entry| entry.seq).collect();
    assert_eq!(
        seqs,
        std::vec![1u64, 2, 3, 4, 5],
        "create, checkin, checkin, trigger, emerg must form one unbroken sequence"
    );
}

#[test]
fn history_trimming_leaves_a_detectable_gap_in_seq_never_a_duplicate() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_basic_will(&env, &client, &owner, &token_address);

    // check_in has no minimum-interval requirement -- only that the will is
    // Active -- so repeated calls at the same timestamp are enough to push
    // the trail well past MAX_HISTORY_ENTRIES and force trimming.
    let total_checkins: u64 = MAX_HISTORY_ENTRIES as u64 + 20;
    for _ in 0..total_checkins {
        client.check_in(&will_id, &owner);
    }

    let history = client.get_will_history(&will_id);
    assert_eq!(
        history.len(),
        MAX_HISTORY_ENTRIES as u32,
        "the retained trail is capped even though far more entries were appended"
    );

    // The oldest retained entry's seq tells us exactly how many entries were
    // trimmed from the front -- a caller can detect the gap instead of
    // mistaking a shifted position for an unchanged entry.
    let seqs: std::vec::Vec<u64> = history.iter().map(|entry| entry.seq).collect();
    let first = *seqs.first().unwrap();
    let last = *seqs.last().unwrap();
    let total_ever_appended = total_checkins + 1; // + create_will's own entry
    assert_eq!(
        last, total_ever_appended,
        "the newest entry's seq must equal the total number ever appended"
    );
    assert_eq!(
        first,
        total_ever_appended - MAX_HISTORY_ENTRIES as u64 + 1,
        "the oldest retained entry's seq must reflect exactly how many were trimmed"
    );

    // No gaps and no duplicates anywhere in the retained window: every
    // consecutive pair differs by exactly 1.
    for window in seqs.windows(2) {
        assert_eq!(
            window[1] - window[0],
            1,
            "retained entries must be strictly consecutive, with neither a gap nor a duplicate"
        );
    }
}
