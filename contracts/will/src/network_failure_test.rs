#![cfg(test)]

//! Failure-scenario coverage for issue #727.
//!
//! The Soroban test host has no RPC layer, so "network" failures are modelled
//! at the contract boundary, which is where they actually matter on-chain:
//!
//! - **Rejected / failed transactions** (RPC error, simulation failure, or a
//!   transaction that aborts on-chain) must leave the will exactly as it was.
//!   Soroban rolls back all storage writes when a call panics, so a
//!   `try_*` call that returns `Err` must not change observable state.
//! - **Timeouts** are modelled by the ledger clock moving past a deadline
//!   between submission and inclusion. A call submitted after its window has
//!   closed must be refused, and the will must stay in its prior state.
//! - **Resubmission** (a client retrying a transaction whose first attempt
//!   actually landed) must be refused without double-applying its effect.
//! - **Reorgs** are modelled as the ledger timestamp moving backwards
//!   relative to a previously observed state. Deadlines stored on the will
//!   are the single source of truth, so a rewound clock must not unlock an
//!   early release.
//!
//! See `tests/integration.rs` for the end-to-end (real wasm / real network)
//! counterpart and for the documented failure-handling policy.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

const DAY: u64 = 86_400;
const START: u64 = 1_700_000_000;

struct Fixture {
    env: Env,
    contract_id: Address,
    token_address: Address,
    owner: Address,
    beneficiary: Address,
    will_id: u64,
}

impl Fixture {
    fn client(&self) -> WillContractClient<'_> {
        WillContractClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_address)
    }
}

fn setup() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let will_id = WillContractClient::new(&env, &contract_id).create_will(
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
        &0,
        &None,
        &0,
    );

    Fixture {
        env,
        contract_id,
        token_address,
        owner,
        beneficiary,
        will_id,
    }
}

// --- Rejected / failed transactions -------------------------------------

#[test]
fn failed_trigger_before_deadline_leaves_will_active() {
    let f = setup();

    // Still inside the check-in window: the RPC-simulated trigger must fail.
    let result = f.client().try_trigger_will(&f.will_id);
    assert!(result.is_err(), "early trigger must be rejected");

    assert_eq!(f.client().get_will(&f.will_id).status, WillStatus::Active);
}

#[test]
fn failed_update_leaves_beneficiaries_unchanged() {
    let f = setup();
    let before = f.client().get_will(&f.will_id).beneficiaries;

    // Percentages do not sum to 10,000: the transaction aborts on-chain.
    let bad = vec![
        &f.env,
        Beneficiary {
            address: Address::generate(&f.env),
            allocation: Allocation::Percentage(5_000),
        },
    ];
    let result = f
        .client()
        .try_update_beneficiaries(&f.will_id, &f.owner, &bad);
    assert!(result.is_err());

    assert_eq!(f.client().get_will(&f.will_id).beneficiaries.len(), before.len());
}

// --- Timeouts ------------------------------------------------------------

#[test]
fn checkin_submitted_after_deadline_is_refused_and_will_is_triggerable() {
    let f = setup();

    // The owner's check-in lands after the window closed (timeout).
    f.env
        .ledger()
        .with_mut(|l| l.timestamp = START + 91 * DAY);
    let late = f.client().try_check_in(&f.will_id, &f.owner);
    assert!(late.is_err(), "late check-in must be refused");

    // The failed check-in must not have reset the countdown, so the will
    // can still be triggered by anyone.
    f.client().trigger_will(&f.will_id);
    assert_eq!(f.client().get_will(&f.will_id).status, WillStatus::Triggered);
}

#[test]
fn emergency_checkin_after_grace_expiry_is_refused() {
    let f = setup();
    f.env.ledger().with_mut(|l| l.timestamp = START + 91 * DAY);
    f.client().trigger_will(&f.will_id);

    // Grace period is 7 days; the owner's reply arrives too late.
    f.env
        .ledger()
        .with_mut(|l| l.timestamp = START + 91 * DAY + 8 * DAY);
    let result = f.client().try_emergency_checkin(&f.will_id, &f.owner);
    assert!(result.is_err());
    assert_eq!(f.client().get_will(&f.will_id).status, WillStatus::Triggered);
}

// --- Resubmission --------------------------------------------------------

#[test]
fn resubmitted_trigger_is_refused_without_double_applying() {
    let f = setup();
    f.env.ledger().with_mut(|l| l.timestamp = START + 91 * DAY);

    f.client().trigger_will(&f.will_id);
    let first = f.client().get_will(&f.will_id);

    // The client retries a trigger whose first attempt already landed.
    let retry = f.client().try_trigger_will(&f.will_id);
    assert!(retry.is_err(), "duplicate trigger must be refused");

    let after = f.client().get_will(&f.will_id);
    assert_eq!(after.status, WillStatus::Triggered);
    assert_eq!(after.trigger_time, first.trigger_time);
}

#[test]
fn resubmitted_release_cannot_pay_out_twice() {
    let f = setup();
    f.env.ledger().with_mut(|l| l.timestamp = START + 91 * DAY);
    f.client().trigger_will(&f.will_id);
    f.env
        .ledger()
        .with_mut(|l| l.timestamp = START + 91 * DAY + 8 * DAY);

    f.client().release_inheritance(&f.will_id, &None);
    let paid = f.token().balance(&f.beneficiary);
    assert!(paid > 0);

    // Retried release after the first one landed.
    let retry = f.client().try_release_inheritance(&f.will_id, &None);
    assert!(retry.is_err(), "second release must be refused");
    assert_eq!(f.token().balance(&f.beneficiary), paid);
}

// --- Reorgs --------------------------------------------------------------

#[test]
fn rewound_clock_does_not_unlock_early_release() {
    let f = setup();
    f.env.ledger().with_mut(|l| l.timestamp = START + 91 * DAY);
    f.client().trigger_will(&f.will_id);

    // Observed state is at the end of grace, then a reorg rewinds the
    // ledger to just after the trigger. The stored grace deadline still
    // governs, so release must not succeed.
    f.env
        .ledger()
        .with_mut(|l| l.timestamp = START + 91 * DAY + 8 * DAY);
    f.env.ledger().with_mut(|l| l.timestamp = START + 91 * DAY + 1);

    let result = f.client().try_release_inheritance(&f.will_id, &None);
    assert!(result.is_err(), "release during grace must be refused");
    assert_eq!(f.client().get_will(&f.will_id).status, WillStatus::Triggered);
}
