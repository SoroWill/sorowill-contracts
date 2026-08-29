#![cfg(test)]

//! Tests for `archive_will` (#284), `reject_guardian_role`, and `update_periods`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, GuardianConsent, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &10_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner, token_address)
}

#[test]
fn archive_will_rejects_active_will() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
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
        &2,
        &None,
        &0,
    );

    let res = client.try_archive_will(&will_id);
    assert_eq!(res, Err(Ok(WillError::WillNotSettled.into())));
}

#[test]
fn archive_will_rejects_triggered_will() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
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
        &2,
        &None,
        &0,
    );

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    let res = client.try_archive_will(&will_id);
    assert_eq!(res, Err(Ok(WillError::WillNotSettled.into())));
}

#[test]
fn archive_will_succeeds_on_cancelled_will() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
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
        &2,
        &None,
        &0,
    );

    client.cancel_will(&will_id, &owner);
    client.archive_will(&will_id);

    let res = client.try_get_will(&will_id);
    assert_eq!(res, Err(Ok(WillError::WillNotFound.into())));
}

#[test]
fn reject_guardian_role_sets_consent_to_rejected() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);
    let guardian = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
        &vec![
            &env,
            Beneficiary {
                address: beneficiary,
                allocation: Allocation::Percentage(10_000),
            },
        ],
        &90,
        &7,
        &vec![&env, guardian.clone()],
        &1,
        &None,
        &0,
    );

    client.reject_guardian_role(&will_id, &guardian);

    let will = client.get_will(&will_id);
    assert_eq!(will.guardians.get(0).unwrap().consent, GuardianConsent::Rejected);
}

#[test]
fn update_periods_updates_checkin_and_grace_periods() {
    let (env, client, owner, token_address) = setup();
    let beneficiary = Address::generate(&env);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address, 1_000_000_i128)],
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
        &2,
        &None,
        &0,
    );

    client.update_periods(&will_id, &owner, &Some(120), &Some(14));

    let will = client.get_will(&will_id);
    assert_eq!(will.checkin_period_days, 120);
    assert_eq!(will.grace_period_days, 14);
}
