#![cfg(test)]

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

use crate::oracle::{OracleClient, OracleError, PriceFeed};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let oracle_id = env.register_contract(None, OracleClient);
    (env, admin, oracle_id)
}

#[test]
fn test_set_and_get_price_feed() {
    let (env, admin, oracle_id) = setup();
    let client = OracleClient::new(&env, &oracle_id);

    let asset = Symbol::new(&env, "XLM");
    client.initialize(&admin);
    client.set_price(&admin, &asset, &123_456_789i128);

    let feed = client.get_price(&asset);
    assert_eq!(feed.price, 123_456_789i128);
    assert_eq!(feed.asset, asset);
}

#[test]
fn test_get_price_returns_fallback_when_missing() {
    let (env, admin, oracle_id) = setup();
    let client = OracleClient::new(&env, &oracle_id);

    let asset = Symbol::new(&env, "USDC");
    client.initialize(&admin);
    client.set_fallback_price(&admin, &asset, &1_000_000i128);

    let feed = client.get_price(&asset);
    assert_eq!(feed.price, 1_000_000i128);
    assert_eq!(feed.asset, asset);
}

#[test]
fn test_get_price_returns_fallback_when_invalid() {
    let (env, admin, oracle_id) = setup();
    let client = OracleClient::new(&env, &oracle_id);

    let asset = Symbol::new(&env, "BTC");
    client.initialize(&admin);
    client.set_fallback_price(&admin, &asset, &42_000_000i128);
    // A non-positive price is treated as invalid and must fall back.
    client.set_price(&admin, &asset, &0i128);

    let feed = client.get_price(&asset);
    assert_eq!(feed.price, 42_000_000i128);
}

#[test]
fn test_get_price_errors_without_feed_or_fallback() {
    let (env, admin, oracle_id) = setup();
    let client = OracleClient::new(&env, &oracle_id);

    let asset = Symbol::new(&env, "ETH");
    client.initialize(&admin);

    let result = client.try_get_price(&asset);
    assert_eq!(result, Err(Ok(OracleError::PriceUnavailable)));
}

#[test]
fn test_set_price_requires_admin() {
    let (env, admin, oracle_id) = setup();
    let client = OracleClient::new(&env, &oracle_id);
    let stranger = Address::generate(&env);

    let asset = Symbol::new(&env, "XLM");
    client.initialize(&admin);

    let result = client.try_set_price(&stranger, &asset, &1i128);
    assert_eq!(result, Err(Ok(OracleError::Unauthorized)));
}

#[test]
fn test_price_feed_struct_roundtrip() {
    let env = Env::default();
    let asset = Symbol::new(&env, "XLM");
    let feed = PriceFeed {
        asset: asset.clone(),
        price: 7i128,
    };
    assert_eq!(feed.asset, asset);
    assert_eq!(feed.price, 7i128);
}
