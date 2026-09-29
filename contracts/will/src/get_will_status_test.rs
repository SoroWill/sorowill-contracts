#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env, Vec as SorobanVec,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};

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

    (env, client, owner, token_address)
}

#[test]
fn test_get_will_status_pending_confirmation() {
    let (env, client, owner, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &1,
        &None,
        &3600,
    );

    let status = client.get_will_status(&will_id);
    assert_eq!(status, WillStatus::PendingConfirmation);
}

#[test]
fn test_get_will_status_active() {
    let (env, client, owner, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // confirmation_delay_seconds is 0 above, so the will is already
    // Active -- confirm_will would now be rejected as WillNotConfirmed.

    let status = client.get_will_status(&will_id);
    assert_eq!(status, WillStatus::Active);
}

#[test]
fn test_get_will_status_triggered() {
    let (env, client, owner, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // confirmation_delay_seconds is 0 above, so the will is already Active.

    // Advance past check-in deadline
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);

    // Trigger the will
    client.trigger_will(&will_id);

    let status = client.get_will_status(&will_id);
    assert_eq!(status, WillStatus::Triggered);
}

#[test]
fn test_get_will_status_released() {
    let (env, client, owner, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // confirmation_delay_seconds is 0 above, so the will is already Active.
    env.ledger().with_mut(|l| l.timestamp += 31 * DAY);
    client.trigger_will(&will_id);

    // Release
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    let status = client.get_will_status(&will_id);
    assert_eq!(status, WillStatus::Released);
}

#[test]
fn test_get_will_status_cancelled() {
    let (env, client, owner, token_address) = setup();

    let beneficiary = Address::generate(&env);
    let beneficiaries: SorobanVec<Beneficiary> = vec![
        &env,
        Beneficiary {
            address: beneficiary,
            allocation: Allocation::Percentage(10_000),
        },
    ];
    let tokens: SorobanVec<(Address, i128)> = vec![&env, (token_address, 1_000_000_i128)];

    let will_id = client.create_will(
        &owner,
        &tokens,
        &beneficiaries,
        &30,
        &7,
        &vec![&env],
        &1,
        &None,
        &0,
    );

    // confirmation_delay_seconds is 0 above, so the will is already Active.
    client.cancel_will(&will_id, &owner);

    let status = client.get_will_status(&will_id);
    assert_eq!(status, WillStatus::Cancelled);
}

#[test]
fn test_get_will_status_nonexistent_will_panics() {
    let (_env, client, _owner, _token_address) = setup();
    // Soroban's panic message only shows the numeric error code, never the
    // enum variant name, so should_panic(expected = "WillNotFound") can
    // never match -- use try_get_will_status instead.
    assert_eq!(
        client.try_get_will_status(&9999),
        Err(Ok(crate::WillError::WillNotFound.into())),
    );
}
