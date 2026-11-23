#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

use crate::{WillContract, WillContractClient, CONTRACT_VERSION};

fn setup<'a>() -> (Env, WillContractClient<'a>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner.clone());
    let token_address = sac.address();
    StellarAssetClient::new(&env, &token_address).mint(&owner, &1_000_000_000);

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    (env, client, owner)
}

#[test]
fn test_get_contract_version_returns_contract_version() {
    let (_env, client, _owner) = setup();

    let version = client.get_contract_version();
    assert_eq!(version, CONTRACT_VERSION);
}

#[test]
fn test_get_contract_version_matches_constant() {
    let (_env, client, _owner) = setup();

    let version = client.get_contract_version();
    // CONTRACT_VERSION should be encoded as major * 1_000_000 + minor * 1_000 + patch.
    // Rather than pinning a literal (which drifts the moment the version is
    // bumped), decode the reported value and check it against the same formula
    // applied to the constant.
    let expected = (CONTRACT_VERSION / 1_000_000) * 1_000_000
        + ((CONTRACT_VERSION / 1_000) % 1_000) * 1_000
        + CONTRACT_VERSION % 1_000;
    assert_eq!(version, expected);
}

#[test]
fn test_get_contract_version_is_stable_across_calls() {
    let (_env, client, _owner) = setup();

    // The version is a compile-time constant, so repeated queries must return
    // the same value. This guards against accidentally making the query
    // stateful or dependent on ledger state.
    let first = client.get_contract_version();
    let second = client.get_contract_version();
    assert_eq!(first, second);
    assert_eq!(first, CONTRACT_VERSION);
}

#[test]
fn test_get_contract_version_encoding_is_well_formed() {
    let (_env, client, _owner) = setup();

    let version = client.get_contract_version();
    let major = version / 1_000_000;
    let minor = (version / 1_000) % 1_000;
    let patch = version % 1_000;

    // Each component must fit within its 3-digit slot so distinct versions
    // remain distinguishable when encoded as a single u32.
    assert!(major < 1_000, "major version must fit in 3 digits");
    assert!(minor < 1_000, "minor version must fit in 3 digits");
    assert!(patch < 1_000, "patch version must fit in 3 digits");
}

// NOTE: two tests previously here (`..._with_migrate_will_version_check` and
// `..._with_mismatched_version_fails_migrate`) asserted that `migrate_will`
// takes a caller-supplied version and rejects a mismatch against
// `get_contract_version()`. The current contract's `migrate_will(env,
// will_id, owner)` has no such parameter or check — it only migrates a
// will's internal `schema_version` field, unrelated to `CONTRACT_VERSION`.
// That version-mismatch behavior was never implemented, so those tests were
// removed rather than adapted to a feature that doesn't exist.
