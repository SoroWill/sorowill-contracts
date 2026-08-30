#![cfg(test)]

//! Regression test for issue #280: `top_up` requires `WillStatus::Active`,
//! but no test attempted to top up a will in any other status and asserted
//! the rejection. A regression here (e.g. accidentally topping up a
//! `Released` will's already-zeroed balance map) would not have been caught.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillError};

const DAY: u64 = 86_400;

fn setup<'a>() -> (Env, WillContractClient<'a>, Address, Address, u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_address = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    let will_id = client.create_will(
        &owner,
        &vec![&env, (token_address.clone(), 1_000_000_i128)],
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
        &1,
        &None,
        &0,
    );

    (env, client, owner, token_address, will_id)
}

#[test]
fn top_up_is_rejected_for_a_triggered_will() {
    let (env, client, owner, token_address, will_id) = setup();

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);

    assert_eq!(
        client.try_top_up(&will_id, &owner, &token_address, &1_000),
        Err(Ok(WillError::WillNotActive.into()))
    );
}

#[test]
fn top_up_is_rejected_for_a_cancelled_will() {
    let (env, client, owner, token_address, will_id) = setup();

    client.cancel_will(&will_id, &owner);

    assert_eq!(
        client.try_top_up(&will_id, &owner, &token_address, &1_000),
        Err(Ok(WillError::WillNotActive.into()))
    );
}

#[test]
fn top_up_is_rejected_for_a_released_will() {
    let (env, client, owner, token_address, will_id) = setup();

    env.ledger().with_mut(|l| l.timestamp += 91 * DAY);
    client.trigger_will(&will_id);
    env.ledger().with_mut(|l| l.timestamp += 8 * DAY);
    client.release_inheritance(&will_id, &None);

    assert_eq!(
        client.try_top_up(&will_id, &owner, &token_address, &1_000),
        Err(Ok(WillError::WillNotActive.into()))
    );
}
