//! Test-only malicious/reentrant SEP-41 token harness.
//!
//! This module exists specifically for reentrancy regression testing: it
//! implements just enough of the token interface (`mint`, `balance`,
//! `transfer`) to fund and pay out a will, and can be configured to call
//! back into an arbitrary `WillContract` entrypoint from inside `transfer`.
//! It is used to prove that the checks-effects-interactions ordering fixed
//! in issue #54 prevents a reentrant token from triggering a double payout
//! during `release_inheritance`.
#![cfg(test)]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

use crate::WillContractClient;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Balance(Address),
    ReentrancyTarget,
}

#[contracttype]
#[derive(Clone)]
struct ReentrancyTarget {
    will_contract: Address,
    will_id: u64,
}

#[contract]
pub struct MaliciousToken;

#[contractimpl]
impl MaliciousToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = DataKey::Balance(to);
        let balance: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(balance + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    /// `create_will` probes every token with a read-only `decimals()` call
    /// before transferring, so this mock must answer it like a real SEP-41
    /// token to be usable as a will's locked asset.
    pub fn decimals(_env: Env) -> u32 {
        7
    }

    /// Configures this token to attempt `release_inheritance(will_id)` on
    /// `will_contract` every time `transfer` is invoked, simulating a
    /// malicious or reentrant token contract.
    pub fn set_reentrancy_target(env: Env, will_contract: Address, will_id: u64) {
        env.storage().instance().set(
            &DataKey::ReentrancyTarget,
            &ReentrancyTarget {
                will_contract,
                will_id,
            },
        );
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let from_key = DataKey::Balance(from);
        let from_balance: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&from_key, &(from_balance - amount));

        let to_key = DataKey::Balance(to);
        let to_balance: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&to_key, &(to_balance + amount));

        if let Some(target) = env
            .storage()
            .instance()
            .get::<_, ReentrancyTarget>(&DataKey::ReentrancyTarget)
        {
            // Attempt to reenter mid-transfer. `try_release_inheritance`
            // must fail here: `distribute` commits `will.status =
            // Released` before this transfer fires, so a second attempt
            // is rejected with `WillNotTriggered` instead of paying out
            // twice.
            let will_client = WillContractClient::new(&env, &target.will_contract);
            let _ = will_client.try_release_inheritance(&target.will_id, &None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Allocation, Beneficiary, WillContract, WillContractClient, WillStatus};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        vec,
    };

    const DAY: u64 = 86_400;

    #[test]
    fn test_reentrant_release_inheritance_cannot_double_pay() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);

        let owner = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let token_id = env.register(MaliciousToken, ());
        let token_client = MaliciousTokenClient::new(&env, &token_id);
        token_client.mint(&owner, &1_000_000);

        let will_contract_id = env.register(WillContract, ());
        let will_client = WillContractClient::new(&env, &will_contract_id);

        let will_id = will_client.create_will(
            &owner,
            &vec![&env, (token_id.clone(), 1_000_000_i128)],
            &vec![
                &env,
                Beneficiary {
                    address: beneficiary.clone(),
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

        env.ledger()
            .set_timestamp(env.ledger().timestamp() + 91 * DAY);
        will_client.trigger_will(&will_id);

        env.ledger()
            .set_timestamp(env.ledger().timestamp() + 8 * DAY);

        // Arm the token to attempt a reentrant release during the payout.
        token_client.set_reentrancy_target(&will_contract_id, &will_id);

        will_client.release_inheritance(&will_id, &None);

        // The beneficiary must receive exactly one payout, not two.
        assert_eq!(token_client.balance(&beneficiary), 1_000_000);
        let will = will_client.get_will(&will_id);
        assert_eq!(will.status, WillStatus::Released);
    }
}
