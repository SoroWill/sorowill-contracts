#![cfg(test)]

//! Regression tests for issue #354: the grace-deadline second must belong to
//! exactly one of `emergency_checkin` and `release_inheritance`.
//!
//! Previously `emergency_checkin` accepted `now <= deadline` while
//! `release_inheritance` accepted `now >= deadline`, so at
//! `now == deadline` both succeeded and the outcome depended purely on which
//! transaction the ledger included first.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError, WillStatus};

const DAY: u64 = 86_400;

/// Creates a `Triggered` will: 90-day check-in, 7-day grace, triggered at
/// `1_700_000_000`. Returns `(env, contract, owner, beneficiary, token, will_id,
/// grace_deadline)`.
fn triggered_will() -> (Env, Address, Address, Address, Address, u64, u64) {
    let env = Env::default();
    env.mock_all_auths();
    let t0 = 1_700_000_000;
    env.ledger().set_timestamp(t0);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let beneficiary = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token.clone(), 1_000_000_i128)],
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

    // Miss the check-in deadline and trigger.
    env.ledger().set_timestamp(t0 + 91 * DAY);
    client.trigger_will(&will_id);
    let grace_deadline = (t0 + 91 * DAY) + 7 * DAY;

    (
        env,
        contract_id,
        owner,
        beneficiary,
        token,
        will_id,
        grace_deadline,
    )
}

/// At exactly the grace deadline the owner wins and the beneficiaries lose.
#[test]
fn grace_deadline_second_belongs_to_the_owner() {
    let (env, contract_id, owner, _b, _t, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(grace_deadline);

    // The owner may still cancel at the boundary second.
    client.emergency_checkin(&will_id, &owner);
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}

/// …and the beneficiaries can never release at that same second.
#[test]
fn release_is_rejected_at_exactly_the_grace_deadline() {
    let (env, contract_id, _owner, beneficiary, _token, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(grace_deadline);

    let err = client
        .try_release_inheritance(&will_id, &Some(beneficiary.clone()))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::GracePeriodNotExpired.into());
    assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);
}

/// One second later the position is reversed: the owner is out of time and the
/// estate is releasable. Together with the two tests above this pins the
/// boundary to a single owner — there is no timestamp at which both or neither
/// call succeeds.
#[test]
fn one_second_past_the_deadline_only_the_release_succeeds() {
    let (env, contract_id, owner, beneficiary, token, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(grace_deadline + 1);

    let err = client
        .try_emergency_checkin(&will_id, &owner)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, WillError::GracePeriodExpired.into());

    client.release_inheritance(&will_id, &Some(beneficiary.clone()));
    assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
    assert_eq!(
        TokenClient::new(&env, &token).balance(&beneficiary),
        1_000_000
    );
}

/// A `Triggered` will sitting on its boundary second reports `0` from
/// `get_time_until_deadline`, which callers must read as "the owner may still
/// act", not "the estate is releasable".
#[test]
fn time_until_deadline_reports_zero_on_the_boundary_second() {
    let (env, contract_id, _owner, _b, _t, will_id, grace_deadline) = triggered_will();
    let client = WillContractClient::new(&env, &contract_id);

    env.ledger().set_timestamp(grace_deadline);
    assert_eq!(client.get_time_until_deadline(&will_id), Some(0));
}

/// `guardian_cancel_trigger` enforces the same deadline as `emergency_checkin`
/// from the owner's side, so a guardian quorum can still cancel on the boundary
/// second and is rejected from the following second on.
#[test]
fn guardian_cancel_follows_the_same_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let t0 = 1_700_000_000;
    env.ledger().set_timestamp(t0);

    let owner = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token).mint(&owner, &1_000_000);
    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let g1 = Address::generate(&env);
    let g2 = Address::generate(&env);
    let will_id = client.create_will(
        &owner,
        &vec![&env, (token, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: Address::generate(&env),
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, g1.clone(), g2.clone()],
        &2,
        &None,
        &0,
    );
    client.accept_guardian_role(&will_id, &g1);
    client.accept_guardian_role(&will_id, &g2);

    env.ledger().set_timestamp(t0 + 91 * DAY);
    client.trigger_will(&will_id);
    let grace_deadline = (t0 + 91 * DAY) + 7 * DAY;
    // The 7-day guardian-list cooldown is long past by the deadline second, so
    // the grace boundary is the only thing that decides whether the cancel is
    // accepted.
    env.ledger().set_timestamp(grace_deadline);

    client.guardian_cancel_trigger(&will_id, &g1.clone());
    client.guardian_cancel_trigger(&will_id, &g2.clone());
    assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
}
