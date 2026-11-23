#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::oracle::{OracleClient, PriceData};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let oracle_id = env.register_contract(None, crate::oracle::Oracle);
    let client = OracleClient::new(&env, &oracle_id);
    client.initialize(&admin);
    (env, oracle_id, admin)
}

#[test]
fn test_price_within_bounds_accepted() {
    let (env, oracle_id, _admin) = setup();
    let client = OracleClient::new(&env, &oracle_id);
    let price = PriceData {
        price: 100_0000000,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&price);
    let stored = client.get_price();
    assert_eq!(stored.price, price.price);
}

#[test]
#[should_panic]
fn test_price_below_min_reverts() {
    let (env, oracle_id, _admin) = setup();
    let client = OracleClient::new(&env, &oracle_id);
    let price = PriceData {
        price: 0,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&price);
}

#[test]
#[should_panic]
fn test_price_above_max_reverts() {
    let (env, oracle_id, _admin) = setup();
    let client = OracleClient::new(&env, &oracle_id);
    let price = PriceData {
        price: i128::MAX,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&price);
}

#[test]
#[should_panic]
fn test_manipulation_detected_reverts() {
    let (env, oracle_id, _admin) = setup();
    let client = OracleClient::new(&env, &oracle_id);
    let base = PriceData {
        price: 100_0000000,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&base);
    // A sudden extreme jump should be flagged as manipulation.
    let manipulated = PriceData {
        price: 100_000_0000000,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&manipulated);
}

#[test]
fn test_manipulation_check_allows_normal_movement() {
    let (env, oracle_id, _admin) = setup();
    let client = OracleClient::new(&env, &oracle_id);
    let base = PriceData {
        price: 100_0000000,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&base);
    let next = PriceData {
        price: 105_0000000,
        timestamp: env.ledger().timestamp(),
    };
    client.set_price(&next);
    assert_eq!(client.get_price().price, next.price);
}
