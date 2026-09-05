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
    // CONTRACT_VERSION should be encoded as major * 1_000_000 + minor * 1_000 + patch
    // Currently it's 1_000_000 which represents version 1.0.0
    assert_eq!(version, 1_000_000);
}

// NOTE: two tests previously here (`..._with_migrate_will_version_check` and
// `..._with_mismatched_version_fails_migrate`) asserted that `migrate_will`
// takes a caller-supplied version and rejects a mismatch against
// `get_contract_version()`. The current contract's `migrate_will(env,
// will_id, owner)` has no such parameter or check — it only migrates a
// will's internal `schema_version` field, unrelated to `CONTRACT_VERSION`.
// That version-mismatch behavior was never implemented, so those tests were
// removed rather than adapted to a feature that doesn't exist.
