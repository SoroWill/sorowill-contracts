#![cfg(test)]

//! Regression coverage for issue #220 and issue #392: `get_will_history`
//! records the full lifecycle, including the transition status, actor, and
//! action labels — and, since #392, the retained trail is bounded at
//! `MAX_HISTORY_ENTRIES` (oldest dropped first) and can be read in pages via
//! `get_will_history_page`.
//!
//! Before #392 every recorded status transition was appended to a single
//! persistent vector that was never trimmed, so a will cycling
//! Active → Triggered → Active through repeated emergency check-ins
//! accumulated entries forever. That risked the per-entry ledger size limit on
//! the write path and blew past resource limits on the read path, where
//! `get_will_history` returned the whole trail in one call with no paging.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Symbol, Vec,
};

use crate::storage::MAX_HISTORY_ENTRIES;
use crate::{
    types::WillStatusTransition, Allocation, Beneficiary, WillContract, WillContractClient,
    WillStatus,
};

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

    (env.clone(), client, owner, token_address)
}

fn advance(env: &Env, days: u64) {
    env.ledger().with_mut(|l| l.timestamp += days * DAY);
}

fn create_will(
    env: &Env,
    client: &WillContractClient<'_>,
    owner: &Address,
    token_address: &Address,
) -> u64 {
    client.create_will(
        owner,
        &vec![env, (token_address.clone(), 10_000_000_i128)],
        &vec![
            env,
            Beneficiary {
                address: Address::generate(env),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![env],
        &2,
        &None,
        &0,
    )
}

/// Cycles the will Active → Triggered → Active `rounds` times. Each round
/// records a `trigger` and an `emerg` transition; `check_in` resets the
/// deadline without recording one, so a round adds exactly two entries on top
/// of the single `create` transition.
fn cycle(env: &Env, client: &WillContractClient<'_>, owner: &Address, will_id: u64, rounds: u32) {
    for _ in 0..rounds {
        // check_in resets the deadline so the next trigger is due.
        client.check_in(&will_id, owner);
        advance(env, 91);
        client.trigger_will(&will_id);
        advance(env, 1);
        client.emergency_checkin(&will_id, owner);
    }
}

/// The action symbols of a trail, for cheap comparisons against the expected
/// sequence.
fn actions(history: &Vec<WillStatusTransition>) -> Vec<Symbol> {
    let mut out = Vec::new(history.env());
    for entry in history.iter() {
        out.push_back(entry.action);
    }
    out
}

#[test]
fn get_will_history_records_lifecycle_transition_sequence() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary.clone(),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 1, "creation should append one audit entry");
    let create = history.get(0).unwrap();
    assert_eq!(create.from_status, WillStatus::Active);
    assert_eq!(create.to_status, WillStatus::Active);
    assert_eq!(create.actor, owner);
    assert_eq!(create.action, symbol_short!("create"));

    advance(&env, 91);
    client.trigger_will(&will_id);

    let history = client.get_will_history(&will_id);
    assert_eq!(
        history.len(),
        2,
        "trigger should append one more transition"
    );
    let trigger = history.get(1).unwrap();
    assert_eq!(trigger.from_status, WillStatus::Active);
    assert_eq!(trigger.to_status, WillStatus::Triggered);
    assert_eq!(trigger.actor, client.address.clone());
    assert_eq!(trigger.action, symbol_short!("trigger"));

    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    let history = client.get_will_history(&will_id);
    assert_eq!(
        history.len(),
        3,
        "release should append the final transition"
    );
    let release = history.get(2).unwrap();
    assert_eq!(release.from_status, WillStatus::Triggered);
    assert_eq!(release.to_status, WillStatus::Released);
    assert_eq!(release.actor, client.address.clone());
    assert_eq!(release.action, symbol_short!("release"));
}

// ── Issue #392: the retained trail is capped and pageable ─────────────────

#[test]
fn history_is_capped_and_drops_the_oldest_entries() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    // Push well past the cap so trimming has to do real work.
    cycle(&env, &client, &owner, will_id, MAX_HISTORY_ENTRIES);

    let history = client.get_will_history(&will_id);
    assert_eq!(
        history.len(),
        MAX_HISTORY_ENTRIES,
        "the retained trail must never exceed MAX_HISTORY_ENTRIES",
    );

    // The retained window is the *newest* entries: `create` is long gone, and
    // the trail still ends on the final transition.
    let symbols = actions(&history);
    assert!(
        !symbols.contains(&symbol_short!("create")),
        "the oldest entries must be the ones dropped",
    );
    let last = symbols.get(symbols.len() - 1).unwrap();
    assert_eq!(
        last,
        symbol_short!("emerg"),
        "the trail must still end on the most recent transition",
    );
}

#[test]
fn history_shorter_than_the_cap_is_returned_whole() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    cycle(&env, &client, &owner, will_id, 3);

    // 1 create + 2 per round (trigger, emerg) = 7, comfortably under the cap.
    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), 7);
    assert_eq!(actions(&history).get(0).unwrap(), symbol_short!("create"));
}

#[test]
fn history_page_returns_a_bounded_slice() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    cycle(&env, &client, &owner, will_id, 10);

    let page = client.get_will_history_page(&will_id, &None, &3);
    assert_eq!(page.len(), 3, "limit must bound the page size");
}

#[test]
fn history_pages_walk_the_whole_trail_without_gaps_or_duplicates() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    cycle(&env, &client, &owner, will_id, 10);

    // Walk the trail in pages of 4 and reassemble it.
    const PAGE: u32 = 4;
    let mut walked: Vec<WillStatusTransition> = Vec::new(&env);
    let mut cursor: u32 = 0;
    loop {
        let page = client.get_will_history_page(&will_id, &Some(cursor), &PAGE);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= PAGE, "a page must never exceed the limit");
        for entry in page.iter() {
            walked.push_back(entry);
        }
        cursor += PAGE;
    }

    let whole = client.get_will_history(&will_id);
    assert_eq!(
        walked.len(),
        whole.len(),
        "paging the trail must visit every retained entry exactly once",
    );
    for (i, entry) in walked.iter().enumerate() {
        let i = i as u32;
        assert_eq!(
            entry.timestamp,
            whole.get(i).unwrap().timestamp,
            "paged entry {i} must match the unpaged trail at the same offset",
        );
    }
}

#[test]
fn history_page_caps_the_limit_at_max_page_size() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    // 1 create + 2 per round, for far more rounds than the cap allows: the raw
    // trail is ~100 transitions, so an unclamped read would return all of them.
    cycle(&env, &client, &owner, will_id, MAX_HISTORY_ENTRIES);

    // A limit far above MAX_PAGE_SIZE is clamped rather than honoured, exactly
    // like the paginated owner/beneficiary reads — so the page comes back at
    // the cap, not at the size of the whole trail.
    let page = client.get_will_history_page(&will_id, &None, &10_000);
    assert_eq!(page.len(), MAX_HISTORY_ENTRIES);
}

#[test]
fn history_page_past_the_end_returns_nothing() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    let page = client.get_will_history_page(&will_id, &Some(9_999), &10);
    assert!(
        page.is_empty(),
        "a cursor beyond the trail must return no entries"
    );
}

#[test]
fn history_page_of_an_unknown_will_is_empty() {
    let (_env, client, _owner, _token_address) = setup();

    let page = client.get_will_history_page(&42, &None, &10);
    assert!(page.is_empty());
    assert!(client.get_will_history(&42).is_empty());
}

#[test]
fn history_cap_does_not_disturb_the_lifecycle_it_records() {
    let (env, client, owner, token_address) = setup();
    let will_id = create_will(&env, &client, &owner, &token_address);

    // Drive well past the cap, then settle the will: the terminal transition
    // must still be recorded and the will must still be releasable.
    cycle(&env, &client, &owner, will_id, MAX_HISTORY_ENTRIES);
    advance(&env, 91);
    client.trigger_will(&will_id);
    advance(&env, 8);
    client.release_inheritance(&will_id, &None);

    let history = client.get_will_history(&will_id);
    assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
    let last = history.get(history.len() - 1).unwrap();
    assert_eq!(last.to_status, WillStatus::Released);
    assert_eq!(last.action, symbol_short!("release"));
}
