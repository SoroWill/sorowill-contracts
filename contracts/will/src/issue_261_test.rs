#![cfg(test)]

//! Regression test for issue #261: `create_will` validates a token address
//! by calling `try_decimals()` and rejecting only if that call itself
//! errors. A contract that implements `decimals()` but not `transfer`
//! correctly passes this probe and only fails later, at the actual
//! `transfer` call inside `create_will` -- as an abort, not a declared
//! `WillError`, since the whole call rolls back and no funds are ever
//! moved. This test documents that failure mode.

use soroban_sdk::{contract, contractimpl, testutils::Address as _, vec, Address, Env};

use crate::{Allocation, Beneficiary, WillContract, WillContractClient};

/// A contract that is superficially token-shaped -- it answers `decimals()`
/// like a real SEP-41 token -- but implements no other part of the
/// interface, in particular no `transfer`.
#[contract]
pub struct DecimalsOnlyToken;

#[contractimpl]
impl DecimalsOnlyToken {
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}

#[test]
fn create_will_aborts_at_transfer_not_at_the_decimals_probe() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let fake_token = env.register(DecimalsOnlyToken, ());

    let contract_id = env.register(WillContract, ());
    let client = WillContractClient::new(&env, &contract_id);

    // The decimals() probe succeeds (it's a real function on this contract),
    // so create_will proceeds past InvalidToken and only fails once it tries
    // to actually call `transfer`, which this contract does not implement.
    let result = client.try_create_will(
        &owner,
        &vec![&env, (fake_token, 1_000_i128)],
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

    // A host-level abort (unimplemented `transfer`), not a declared
    // WillError -- the probe cannot and does not claim to catch this.
    assert!(
        matches!(result, Err(Err(_))),
        "expected an abort at the transfer call, got {result:?}"
    );
}
