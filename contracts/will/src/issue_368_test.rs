#![cfg(test)]

//! Regression tests for issue #368: the global `TriggeredWills` index must be
//! readable a page at a time.
//!
//! `get_triggered_wills` used to take no arguments and return the entire
//! vector in one call. Every triggered will id in the protocol lived in that
//! single unbounded `Vec<u64>`, so a keeper bot polling it had to deserialize
//! the whole index on every poll, and the entry grew without any read-side
//! bound. It now takes a `(cursor, limit)` pair and reuses
//! `storage::paginate_ids`, like `get_wills_by_owner` and
//! `get_wills_by_beneficiary`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

const DAY: u64 = 86_400;

/// Page size used in these tests: smaller than the number of triggered wills
/// created below, so paging is genuinely exercised.
const PAGE: u32 = 3;

/// Registers the contract with a funded owner and returns a client.
fn setup() -> (Env, WillContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

/// Creates a funded will owned by `owner` that locks `token`, and returns its
/// id.
fn create(env: &Env, client: &WillContractClient, owner: &Address, token: &Address) -> u64 {
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        env,
        Beneficiary {
            address: Address::generate(env),
            allocation: Allocation::Percentage(10_000),
        },
    ];
    client.create_will(
        owner,
        &vec![env, (token.clone(), 1_000_i128)],
        &beneficiaries,
        &1,
        &7,
        &vec![env],
        &1,
        &None,
        &0,
    )
}

/// Advances past every check-in deadline and triggers each of `count` freshly
/// created wills, returning their ids in allocation order.
fn make_triggered(
    env: &Env,
    client: &WillContractClient,
    owner: &Address,
    token: &Address,
    count: u64,
) -> SorobanVec<u64> {
    let mut ids = vec![env];
    for _ in 0..count {
        ids.push_back(create(env, client, owner, token));
    }
    env.ledger().with_mut(|l| l.timestamp += 2 * DAY);
    for will_id in ids.iter() {
        client.trigger_will(&will_id);
    }
    ids
}

/// Issue #368, acceptance criterion: paging through **more** triggered wills
/// than fit in one page yields every id exactly once, in order.
#[test]
fn paging_visits_every_triggered_will_across_multiple_pages() {
    let (env, client, owner, token) = setup();
    let expected = make_triggered(&env, &client, &owner, &token, 7);
    assert_eq!(expected.len(), 7, "seven wills, three-id pages");

    // Walk the index with the cursor, exactly as a keeper bot would.
    let mut seen: SorobanVec<u64> = vec![&env];
    let mut cursor: Option<u64> = None;
    loop {
        let page = client.get_triggered_wills(&cursor, &PAGE);
        if page.is_empty() {
            break;
        }
        assert!(
            page.len() <= PAGE,
            "a page may never exceed the requested limit"
        );
        for id in page.iter() {
            seen.push_back(id);
        }
        cursor = page.last();
    }

    assert_eq!(seen, expected, "paging must visit every id once, in order");
}

/// A single call returns at most `limit` ids, and the cursor is exclusive, so
/// consecutive pages tile the index without gaps or repeats.
#[test]
fn a_single_call_returns_at_most_the_requested_limit() {
    let (env, client, owner, token) = setup();
    let ids = make_triggered(&env, &client, &owner, &token, 5);
    let (a, b, c, d) = (
        ids.get(0).unwrap(),
        ids.get(1).unwrap(),
        ids.get(2).unwrap(),
        ids.get(3).unwrap(),
    );

    assert_eq!(client.get_triggered_wills(&None, &1).len(), 1);
    assert_eq!(client.get_triggered_wills(&None, &2).len(), 2);

    let first = client.get_triggered_wills(&None, &2);
    let second = client.get_triggered_wills(&first.last(), &2);
    assert_eq!(first, vec![&env, a, b]);
    assert_eq!(second, vec![&env, c, d]);
}

/// `limit` is capped at `storage::MAX_PAGE_SIZE`, exactly as for the other
/// paginated indexes, so a caller cannot ask for an unbounded read.
#[test]
fn the_limit_is_capped_at_max_page_size() {
    let (env, client, owner, token) = setup();
    make_triggered(&env, &client, &owner, &token, 3);

    // Ask for far more than the cap: the answer is still bounded.
    let page = client.get_triggered_wills(&None, &10_000);
    assert!(page.len() <= crate::storage::MAX_PAGE_SIZE);
    assert_eq!(page.len(), 3, "only three wills are triggered");
}

/// Order-preserving removal is what makes a cursor safe here: a will leaving
/// `Triggered` from the middle of the index must not make the next page skip
/// or repeat the ids after it.
#[test]
fn removing_a_middle_entry_does_not_break_paging() {
    let (env, client, owner, token) = setup();
    let ids = make_triggered(&env, &client, &owner, &token, 5);
    let (a, b, c, d) = (
        ids.get(0).unwrap(),
        ids.get(1).unwrap(),
        ids.get(3).unwrap(),
        ids.get(4).unwrap(),
    );

    // Release the middle will, which unindexes it.
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&ids.get(2).unwrap(), &None);

    let first = client.get_triggered_wills(&None, &2);
    let second = client.get_triggered_wills(&first.last(), &10);

    let mut seen: SorobanVec<u64> = first;
    for id in second.iter() {
        seen.push_back(id);
    }

    let expected = vec![&env, a, b, c, d];
    assert_eq!(
        seen, expected,
        "the released id must be gone and the rest must stay in order"
    );
}
