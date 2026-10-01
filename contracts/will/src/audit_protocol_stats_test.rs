#![cfg(test)]

//! Regression coverage for issue #498: `get_protocol_stats` sums locked
//! balances incrementally but had no verified way to catch bookkeeping
//! drift. `audit_protocol_stats`/`repair_protocol_stats` (added for #441)
//! already provide that mechanism -- compare the incrementally-maintained
//! `ProtocolStats` against a fresh recomputation from every will's stored
//! balance -- but had zero test coverage, and the `ProtocolStatsAudit` type
//! `audit_protocol_stats` returns was referenced in `lib.rs` without ever
//! being defined in `types.rs`, a compile error fixed alongside these tests.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec,
};

use crate::storage;
use crate::types::{ProtocolStats, TokenLockedBalance};
use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env.clone(), client, owner, token_address, contract_id)
}

#[test]
fn audit_reports_consistent_after_normal_operations() {
    let (env, client, owner, token_address, _contract_id) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    let audit = client.audit_protocol_stats();
    assert!(
        audit.consistent,
        "stored and recomputed stats must agree after a single create_will"
    );
    assert_eq!(audit.stored, audit.computed);
    assert_eq!(audit.stored.active_will_count, 1);
}

#[test]
fn audit_reports_consistent_across_multiple_wills_and_a_cancellation() {
    let (env, client, owner, token_address, _contract_id) = setup();
    let beneficiary = Address::generate(&env);

    let will_1 = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
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
        &0,
        &None,
        &0,
    );
    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 250_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Cancelling one will must be reflected in both the stored and
    // recomputed totals identically.
    client.cancel_will(&will_1, &owner);

    let audit = client.audit_protocol_stats();
    assert!(
        audit.consistent,
        "stored and recomputed stats must agree after create_will x2 + cancel_will"
    );
    assert_eq!(audit.stored.active_will_count, 1);
}

#[test]
fn audit_detects_drift_and_repair_fixes_it() {
    let (env, client, owner, token_address, contract_id) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Sanity check: consistent before we deliberately corrupt anything.
    assert!(client.audit_protocol_stats().consistent);

    // Directly corrupt the stored stats to simulate bookkeeping drift --
    // an incrementing helper skipped on some future code path, for example.
    let real_computed = env.as_contract(&contract_id, || storage::recompute_protocol_stats(&env));
    let corrupted = ProtocolStats {
        active_will_count: real_computed.active_will_count + 41,
        total_locked_by_token: vec![
            &env,
            TokenLockedBalance {
                token: token_address.clone(),
                total_locked: 999_999_999,
            },
        ],
    };
    env.as_contract(&contract_id, || {
        storage::save_protocol_stats(&env, &corrupted);
    });

    let audit = client.audit_protocol_stats();
    assert!(
        !audit.consistent,
        "audit must detect the deliberately-introduced drift"
    );
    assert_eq!(audit.stored.active_will_count, real_computed.active_will_count + 41);
    assert_eq!(
        audit.computed, real_computed,
        "the recomputed side must be unaffected by the corrupted stored value"
    );

    // repair_protocol_stats() must overwrite the drifted stored value with
    // the recomputed truth, and return it.
    let repaired = client.repair_protocol_stats();
    assert_eq!(repaired, real_computed);

    let audit_after_repair = client.audit_protocol_stats();
    assert!(
        audit_after_repair.consistent,
        "audit must report consistent again immediately after repair"
    );
    assert_eq!(audit_after_repair.stored, real_computed);
}

#[test]
fn repair_drops_tokens_whose_recomputed_total_is_zero() {
    let (env, client, owner, token_address, contract_id) = setup();
    let beneficiary = Address::generate(&env);

    client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 100_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env],
        &0,
        &None,
        &0,
    );

    // Inject a stale entry for a token no active will actually holds.
    let stale_token = Address::generate(&env);
    let mut stored = env.as_contract(&contract_id, || storage::get_protocol_stats(&env));
    stored.total_locked_by_token.push_back(TokenLockedBalance {
        token: stale_token.clone(),
        total_locked: 5_000,
    });
    env.as_contract(&contract_id, || {
        storage::save_protocol_stats(&env, &stored);
    });

    assert!(!client.audit_protocol_stats().consistent);

    let repaired = client.repair_protocol_stats();
    let stale_entry_present = repaired
        .total_locked_by_token
        .iter()
        .any(|entry| entry.token == stale_token);
    assert!(
        !stale_entry_present,
        "repair must drop a token entry whose recomputed total is zero, not keep it at zero"
    );
}
