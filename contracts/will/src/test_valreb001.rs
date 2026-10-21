#![cfg(test)]

//! Regression tests for issues #596, #597, #598 and #599.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{
    migration, migration_rules, storage, Allocation, Beneficiary, WillContract,
    WillContractClient, WillStatus,
};

/// Creates a funded single-beneficiary will with `guardian_count` guardians
/// (threshold 1 when any exist). Returns
/// `(env, contract_id, owner, token, will_id, guardians)`.
fn setup(
    guardian_count: u32,
) -> (
    Env,
    Address,
    Address,
    Address,
    u64,
    soroban_sdk::Vec<Address>,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);
    let mut guardians: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&env);
    for _ in 0..guardian_count {
        guardians.push_back(Address::generate(&env));
    }
    let threshold: u32 = if guardians.is_empty() { 0 } else { 1 };

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &guardians,
        &threshold,
        &None,
        &0,
    );
    (env, contract_id, owner, token, will_id, guardians)
}

// ---------------------------------------------------------------------------
// #596: migration invariants
// ---------------------------------------------------------------------------

/// Loads the stored will, forces it to `version` and returns it.
fn stale_will(
    env: &Env,
    contract_id: &Address,
    will_id: u64,
    version: u32,
) -> crate::Will {
    env.as_contract(contract_id, || {
        let mut will = match storage::load_will(env, will_id) {
            Ok(will) => will,
            Err(_) => panic!("will not found"),
        };
        will.schema_version = version;
        will
    })
}

#[test]
fn migration_accepts_a_consistent_v0_will() {
    let (env, contract_id, _owner, _token, will_id, _gs) = setup(0);
    let will = stale_will(&env, &contract_id, will_id, 0);
    let upgraded = env.as_contract(&contract_id, || migration::upgrade(&env, will));
    assert_eq!(upgraded.schema_version, migration::CURRENT_SCHEMA_VERSION);
}

#[test]
fn migration_rules_reject_triggered_will_without_trigger_time() {
    let (env, contract_id, _owner, _token, will_id, _gs) = setup(0);
    let mut will = stale_will(&env, &contract_id, will_id, 1);
    assert!(migration_rules::is_valid_migration(&will, 0));

    will.status = WillStatus::Triggered;
    will.trigger_time = None;
    assert!(!migration_rules::is_valid_migration(&will, 0));
}

#[test]
fn migration_rules_reject_non_sequential_version_and_oversized_tally() {
    let (env, contract_id, _owner, _token, will_id, _gs) = setup(0);
    let mut will = stale_will(&env, &contract_id, will_id, 1);
    // Version jumped by two.
    assert!(!migration_rules::is_valid_migration(&will, 5));
    // Tally larger than the (empty) guardian list.
    will.guardian_votes = 3;
    assert!(!migration_rules::is_valid_migration(&will, 0));
}

#[test]
#[should_panic]
fn upgrade_rejects_partially_migrated_triggered_state() {
    let (env, contract_id, _owner, _token, will_id, _gs) = setup(0);
    let mut will = stale_will(&env, &contract_id, will_id, 0);
    will.status = WillStatus::Triggered;
    will.trigger_time = None;
    env.as_contract(&contract_id, || migration::upgrade(&env, will));
}

// ---------------------------------------------------------------------------
// #598: guardian list replacement discards stale vote records
// ---------------------------------------------------------------------------

#[test]
fn update_guardians_purges_votes_even_when_tally_is_zero() {
    let (env, contract_id, owner, _token, will_id, gs) = setup(2);
    let client = WillContractClient::new(&env, &contract_id);
    let removed = gs.get(0).unwrap();
    let kept = gs.get(1).unwrap();

    // A leftover record whose tally was never counted on the will
    // (`guardian_votes == 0`), as when an expired record is left behind.
    env.as_contract(&contract_id, || {
        storage::set_guardian_voted(
            &env,
            will_id,
            &removed,
            1_700_000_000,
            crate::GuardianVoteReason::Other,
        );
    });

    // Remove the guardian, then add them back.
    client.update_guardians(&will_id, &owner, &vec![&env, kept.clone()]);
    client.update_guardians(&will_id, &owner, &vec![&env, kept, removed.clone()]);

    env.as_contract(&contract_id, || {
        assert!(storage::get_guardian_vote(&env, will_id, &removed).is_none());
    });
    let will = client.get_will(&will_id);
    assert_eq!(will.guardian_votes, 0);
    assert_eq!(will.guardian_vote_weight, 0);
}

#[test]
fn update_guardians_resets_the_cooldown_clock() {
    let (env, contract_id, owner, _token, will_id, gs) = setup(2);
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(1_700_100_000);
    client.update_guardians(&will_id, &owner, &vec![&env, gs.get(0).unwrap()]);

    assert_eq!(client.get_will(&will_id).guardian_list_updated_at, 1_700_100_000);
}

// ---------------------------------------------------------------------------
// #599: small percentage shares are not rounded to zero
// ---------------------------------------------------------------------------

#[test]
fn min_one_unit_share_bounds() {
    use crate::distribute_limits::min_one_unit_share;
    // Floor of zero is lifted to one unit.
    assert_eq!(min_one_unit_share(0, 1_000, 2), 1);
    // A larger floor is untouched.
    assert_eq!(min_one_unit_share(500, 1_000, 2), 500);
    // Never take the unit reserved for later beneficiaries.
    assert_eq!(min_one_unit_share(0, 2, 2), 0);
    assert_eq!(min_one_unit_share(10, 5, 2), 3);
}

#[test]
fn distribute_pays_every_small_percentage_beneficiary_at_least_one_unit() {
    use soroban_sdk::token::Client as TokenClient;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: a.clone(),
                allocation: Allocation::Percentage(1),
            },
            Beneficiary {
                address: b.clone(),
                allocation: Allocation::Percentage(1),
            },
            Beneficiary {
                address: c.clone(),
                allocation: Allocation::Percentage(9_998),
            },
        ],
        &90,
        &7,
        &soroban_sdk::Vec::new(&env),
        &0,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * 86_400);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * 86_400);
    client.release_inheritance(&will_id, &None);

    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&a), 1);
    assert_eq!(token_client.balance(&b), 1);
    assert_eq!(token_client.balance(&c), 998);
}

// ---------------------------------------------------------------------------
// #597: distribute pre-flight payout budget
// ---------------------------------------------------------------------------

#[test]
fn payout_budget_accepts_the_maximum_combination() {
    let env = Env::default();
    crate::distribute_limits::assert_payout_budget(&env, crate::MAX_BENEFICIARIES, 10);
    crate::distribute_limits::assert_payout_budget(&env, 0, 10);
}

#[test]
#[should_panic]
fn payout_budget_rejects_more_than_the_maximum_combination() {
    let env = Env::default();
    crate::distribute_limits::assert_payout_budget(&env, crate::MAX_BENEFICIARIES, 11);
}
