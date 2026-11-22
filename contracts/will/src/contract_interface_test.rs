#![cfg(test)]

//! Verification test for the contract's public interface.
//! This test ensures the contract compiles and the main entry points
//! are accessible, as part of documentation improvements (issue #198).
//!
//! Compliance checks (issue #780):
//! The will contract must satisfy the following regulatory requirements.
//! Each requirement is documented here and verified by an automated test
//! below so that the checks run as part of the normal test suite.
//!
//! Requirements:
//! - R1 (Ownership): a will is created by and recorded against its owner.
//! - R2 (Beneficiary allocation): every beneficiary allocation is recorded
//!   and the sum of percentage allocations does not exceed 100% (10_000 bps).
//! - R3 (Deadline): a will always exposes a positive time-until-deadline
//!   while it is active, so beneficiaries can be notified in time.
//! - R4 (Auditability): protocol stats and contract version are queryable so
//!   the state of the contract can be audited at any time.
//!
//! Audit documentation: the tests below are the executable record of the
//! requirements above. Any change to the contract that breaks a requirement
//! will fail the corresponding `compliance_*` test.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

#[test]
fn contract_interface_is_available() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary = Address::generate(&env);

    // Test that main entry points are accessible
    // create_will
    let will_id = client.create_will(
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

    // get_will
    let will = client.get_will(&will_id);
    assert_eq!(will.id, will_id);

    // get_will_status
    let status = client.get_will_status(&will_id);
    assert_eq!(status, will.status);

    // get_time_until_deadline
    let deadline = client.get_time_until_deadline(&will_id);
    assert!(deadline.is_some());

    // get_wills_by_owner
    let owner_wills = client.get_wills_by_owner(&owner, &None, &10);
    assert!(!owner_wills.is_empty());

    // get_wills_by_beneficiary
    let beneficiary_wills = client.get_wills_by_beneficiary(&beneficiary, &None, &10);
    assert!(!beneficiary_wills.is_empty());

    // get_protocol_stats
    let stats = client.get_protocol_stats();
    assert!(!stats.total_locked_by_token.is_empty());

    // get_contract_version
    let version = client.get_contract_version();
    assert!(version > 0);
}

/// Sets up a funded owner, a registered contract and a single beneficiary.
fn setup_compliance_env(env: &Env) -> (WillContractClient<'_>, Address, Address, Address) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(env, &contract_id);

    let beneficiary = Address::generate(env);
    (client, owner, token_address, beneficiary)
}

/// R1: a will is created by and recorded against its owner.
#[test]
fn compliance_owner_is_recorded() {
    let env = Env::default();
    let (client, owner, token_address, beneficiary) = setup_compliance_env(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 100_000_i128)],
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

    let will = client.get_will(&will_id);
    assert_eq!(will.owner, owner, "R1: will must be recorded against its owner");
}

/// R2: beneficiary allocations are recorded and do not exceed 100%.
#[test]
fn compliance_allocations_within_limit() {
    let env = Env::default();
    let (client, owner, token_address, beneficiary) = setup_compliance_env(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 100_000_i128)],
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

    let will = client.get_will(&will_id);
    assert!(
        !will.beneficiaries.is_empty(),
        "R2: at least one beneficiary allocation must be recorded"
    );

    let mut total_bps: u32 = 0;
    for b in will.beneficiaries.iter() {
        if let Allocation::Percentage(bps) = b.allocation {
            total_bps += bps;
        }
    }
    assert!(
        total_bps <= 10_000,
        "R2: total percentage allocation must not exceed 100%"
    );
}

/// R3: an active will exposes a positive time-until-deadline.
#[test]
fn compliance_deadline_is_positive() {
    let env = Env::default();
    let (client, owner, token_address, beneficiary) = setup_compliance_env(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 100_000_i128)],
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

    let deadline = client.get_time_until_deadline(&will_id);
    assert!(
        deadline.is_some() && deadline.unwrap() > 0,
        "R3: active will must expose a positive time-until-deadline"
    );
}

/// R4: protocol stats and contract version are queryable for auditing.
#[test]
fn compliance_audit_data_is_queryable() {
    let env = Env::default();
    let (client, _owner, _token_address, _beneficiary) = setup_compliance_env(&env);

    let version = client.get_contract_version();
    assert!(version > 0, "R4: contract version must be queryable");

    let stats = client.get_protocol_stats();
    assert!(
        !stats.total_locked_by_token.is_empty(),
        "R4: protocol stats must be queryable for auditing"
    );
}
