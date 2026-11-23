#![cfg(test)]

use crate::{WillContract, WillContractClient, RateLimitConfig};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup() -> (Env, WillContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, WillContract);
    let client = WillContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin)
}

#[test]
fn test_rate_limit_per_address() {
    let (env, client, _admin) = setup();
    let user = Address::generate(&env);

    client.set_rate_limit_config(&RateLimitConfig {
        per_address_limit: 2,
        per_contract_limit: 100,
        window_seconds: 60,
    });

    client.record_operation(&user);
    client.record_operation(&user);

    let result = client.try_record_operation(&user);
    assert!(result.is_err(), "address limit should be enforced");
}

#[test]
fn test_rate_limit_per_contract() {
    let (env, client, _admin) = setup();

    client.set_rate_limit_config(&RateLimitConfig {
        per_address_limit: 100,
        per_contract_limit: 2,
        window_seconds: 60,
    });

    let user_a = Address::generate(&env);
    let user_b = Address::generate(&env);
    let user_c = Address::generate(&env);

    client.record_operation(&user_a);
    client.record_operation(&user_b);

    let result = client.try_record_operation(&user_c);
    assert!(result.is_err(), "contract limit should be enforced");
}

#[test]
fn test_rate_limit_configurable() {
    let (env, client, _admin) = setup();
    let user = Address::generate(&env);

    client.set_rate_limit_config(&RateLimitConfig {
        per_address_limit: 1,
        per_contract_limit: 100,
        window_seconds: 60,
    });

    client.record_operation(&user);
    assert!(client.try_record_operation(&user).is_err());

    client.set_rate_limit_config(&RateLimitConfig {
        per_address_limit: 5,
        per_contract_limit: 100,
        window_seconds: 60,
    });

    client.record_operation(&user);
    assert!(client.try_record_operation(&user).is_ok());
}

#[test]
fn test_rate_limit_window_resets() {
    let (env, client, _admin) = setup();
    let user = Address::generate(&env);

    client.set_rate_limit_config(&RateLimitConfig {
        per_address_limit: 1,
        per_contract_limit: 100,
        window_seconds: 60,
    });

    client.record_operation(&user);
    assert!(client.try_record_operation(&user).is_err());

    env.ledger().with_mut(|li| {
        li.timestamp += 61;
    });

    assert!(client.try_record_operation(&user).is_ok());
}
