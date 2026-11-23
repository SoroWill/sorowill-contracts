//! Oracle integration for the Will contract.
//!
//! Provides a trait for fetching external price data, a Soroban-backed
//! implementation that reads from a price-feed contract, and a fallback
//! price mechanism used when oracle data is unavailable or invalid.

use soroban_sdk::{contracttype, Address, Env, Symbol};

/// A price quote expressed as a fixed-point value with its decimals.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceQuote {
    /// Price scaled by `10^decimals`.
    pub price: i128,
    /// Number of decimals the `price` is scaled by.
    pub decimals: u32,
    /// Ledger timestamp at which the quote was produced.
    pub timestamp: u64,
}

/// Interface for fetching external price data.
pub trait Oracle {
    /// Fetch the current price for `asset`.
    ///
    /// Returns `None` when the oracle cannot provide a valid quote
    /// (e.g. unknown asset, stale data, or a failed cross-contract call).
    fn get_price(&self, env: &Env, asset: &Address) -> Option<PriceQuote>;
}

/// Oracle implementation backed by an on-chain price-feed contract.
///
/// The feed contract is expected to expose `price(asset) -> Option<PriceQuote>`.
pub struct PriceFeedOracle {
    pub feed: Address,
}

impl Oracle for PriceFeedOracle {
    fn get_price(&self, env: &Env, asset: &Address) -> Option<PriceQuote> {
        let client = PriceFeedClient::new(env, &self.feed);
        client.try_price(asset).ok().and_then(|res| res.ok())
    }
}

/// Minimal client for the price-feed contract.
#[soroban_sdk::contractclient(name = "PriceFeedClient")]
pub trait PriceFeed {
    fn price(env: Env, asset: Address) -> Option<PriceQuote>;
}

/// Fallback price source used when the oracle is unavailable or invalid.
pub struct FallbackOracle {
    pub fallback_price: i128,
    pub fallback_decimals: u32,
}

impl Oracle for FallbackOracle {
    fn get_price(&self, _env: &Env, _asset: &Address) -> Option<PriceQuote> {
        Some(PriceQuote {
            price: self.fallback_price,
            decimals: self.fallback_decimals,
            timestamp: 0,
        })
    }
}

/// Resolve a price for `asset`, preferring the primary oracle and falling
/// back to `fallback` when the primary returns no valid quote.
///
/// A quote is considered invalid when its price is non-positive.
pub fn resolve_price(
    env: &Env,
    primary: &impl Oracle,
    fallback: &impl Oracle,
    asset: &Address,
) -> Option<PriceQuote> {
    if let Some(quote) = primary.get_price(env, asset) {
        if quote.price > 0 {
            return Some(quote);
        }
    }
    fallback.get_price(env, asset).filter(|q| q.price > 0)
}

/// Emit an event when the oracle is consulted for `asset`.
pub fn emit_price_used(env: &Env, asset: &Address, quote: &PriceQuote) {
    env.events().publish(
        (Symbol::new(env, "oracle_price"), asset.clone()),
        (quote.price, quote.decimals, quote.timestamp),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    struct FixedOracle(Option<PriceQuote>);

    impl Oracle for FixedOracle {
        fn get_price(&self, _env: &Env, _asset: &Address) -> Option<PriceQuote> {
            self.0.clone()
        }
    }

    fn quote(price: i128) -> PriceQuote {
        PriceQuote { price, decimals: 7, timestamp: 100 }
    }

    #[test]
    fn uses_primary_when_valid() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let primary = FixedOracle(Some(quote(150)));
        let fallback = FixedOracle(Some(quote(99)));
        let resolved = resolve_price(&env, &primary, &fallback, &asset).unwrap();
        assert_eq!(resolved.price, 150);
    }

    #[test]
    fn falls_back_when_primary_missing() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let primary = FixedOracle(None);
        let fallback = FixedOracle(Some(quote(99)));
        let resolved = resolve_price(&env, &primary, &fallback, &asset).unwrap();
        assert_eq!(resolved.price, 99);
    }

    #[test]
    fn falls_back_when_primary_invalid() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let primary = FixedOracle(Some(quote(0)));
        let fallback = FixedOracle(Some(quote(42)));
        let resolved = resolve_price(&env, &primary, &fallback, &asset).unwrap();
        assert_eq!(resolved.price, 42);
    }

    #[test]
    fn returns_none_when_all_invalid() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let primary = FixedOracle(None);
        let fallback = FixedOracle(Some(quote(-1)));
        assert!(resolve_price(&env, &primary, &fallback, &asset).is_none());
    }

    #[test]
    fn fallback_oracle_returns_configured_price() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let oracle = FallbackOracle { fallback_price: 7, fallback_decimals: 2 };
        let resolved = oracle.get_price(&env, &asset).unwrap();
        assert_eq!(resolved.price, 7);
        assert_eq!(resolved.decimals, 2);
    }
}
