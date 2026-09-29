#![cfg(test)]

//! Regression tests for issue #376: `create_will` and `batch_create_wills` both
//! seed the audit trail with a `create` transition through `record_transition`,
//! but `clone_will` and the child will produced by `split_will` skipped it, so
//! `get_will_history` returned an empty list for them.
//!
//! Both now record the create transition, matching the promise the
//! `batch_create_wills` docs make for every creation path.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SoroVec,
};

use crate::{
    Allocation, Beneficiary, WillContract, WillContractClient, WillStatus, WillStatusTransition,
};

/// Creates a funded source will with two beneficiaries. Returns the env, the
/// client, the owner, the token address, the beneficiaries and the will id.
fn setup() -> (
    Env,
    WillContractClient<'static>,
    Address,
    Address,
    Address,
    Address,
    u64,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary_a = Address::generate(&env);
    let beneficiary_b = Address::generate(&env);
    let source_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a.clone(),
                allocation: Allocation::Percentage(6_000),
            },
            Beneficiary {
                address: beneficiary_b.clone(),
                allocation: Allocation::Percentage(4_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &2,
        &None,
        &0,
    );

    (
        env,
        client,
        owner,
        token_address,
        beneficiary_a,
        beneficiary_b,
        source_id,
    )
}

/// Asserts `history` is exactly one `Active -> Active` `create` entry recorded
/// by `actor` — the seed every creation path is expected to write.
fn assert_single_create(history: SoroVec<WillStatusTransition>, will_id: u64, actor: &Address) {
    assert_eq!(history.len(), 1, "history must hold the create transition");
    let entry = history.get(0).unwrap();
    assert_eq!(entry.will_id, will_id);
    assert_eq!(entry.from_status, WillStatus::Active);
    assert_eq!(entry.to_status, WillStatus::Active);
    assert_eq!(entry.action, symbol_short!("create"));
    assert_eq!(entry.actor, *actor);
}

#[test]
fn get_will_history_starts_with_a_create_entry_for_a_cloned_will() {
    let (env, client, owner, token_address, _a, _b, source_id) = setup();
    let now = env.ledger().timestamp();

    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address, 500_000_i128)],
    );

    assert_single_create(client.get_will_history(&clone_id), clone_id, &owner);
    assert_eq!(
        client.get_will_history(&clone_id).get(0).unwrap().timestamp,
        now,
        "the create entry is stamped at the clone time"
    );

    // The source's own history is untouched: cloning is not a status change.
    assert_single_create(client.get_will_history(&source_id), source_id, &owner);
}

#[test]
fn get_will_history_starts_with_a_create_entry_for_a_split_child() {
    let (env, client, owner, token_address, beneficiary_a, _b, source_id) = setup();
    let now = env.ledger().timestamp();

    let child_id = client.split_will(
        &source_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: beneficiary_a,
                allocation: Allocation::Percentage(6_000),
            },
        ],
        &vec![&env, (token_address, 250_000_i128)],
    );

    assert_single_create(client.get_will_history(&child_id), child_id, &owner);
    assert_eq!(
        client.get_will_history(&child_id).get(0).unwrap().timestamp,
        now,
        "the create entry is stamped at the split time"
    );

    // The source's history keeps only its own create entry; the split does not
    // append a transition to it.
    assert_single_create(client.get_will_history(&source_id), source_id, &owner);
}

#[test]
fn every_creation_path_starts_its_audit_trail_the_same_way() {
    let (env, client, owner, token_address, _a, _b, source_id) = setup();

    // A batch-created will, a clone and a split child all start with the same
    // single `create` entry, so a consumer of `get_will_history` does not have
    // to special-case which creation path was used.
    let batched = client.batch_create_wills(
        &owner,
        &vec![
            &env,
            (
                vec![&env, (token_address.clone(), 100_000_i128)],
                vec![
                    &env,
                    Beneficiary {
                        address: Address::generate(&env),
                        allocation: Allocation::Percentage(10_000),
                    },
                ],
                90_u64,
                7_u64,
                SoroVec::new(&env),
                2_u32,
            ),
        ],
    );
    let batched_id = batched.get(0).unwrap();

    let clone_id = client.clone_will(
        &source_id,
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
    );
    let child_id = client.split_will(
        &source_id,
        &owner,
        &vec![
            &env,
            Beneficiary {
                address: client
                    .get_will(&source_id)
                    .beneficiaries
                    .get(0)
                    .unwrap()
                    .address,
                allocation: Allocation::Percentage(6_000),
            },
        ],
        &vec![&env, (token_address, 100_000_i128)],
    );

    assert_single_create(client.get_will_history(&batched_id), batched_id, &owner);
    assert_single_create(client.get_will_history(&clone_id), clone_id, &owner);
    assert_single_create(client.get_will_history(&child_id), child_id, &owner);

    // Cloning an already-created will appends nothing to the source: the
    // source's history is still just its own creation.
    assert_single_create(client.get_will_history(&source_id), source_id, &owner);
}
