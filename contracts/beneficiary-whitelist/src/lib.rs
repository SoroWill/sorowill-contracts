#![no_std]

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, Address, Env};

#[contracttype]
enum Key {
    Admin,
    Member(Address),
}

#[contractevent]
pub struct MembershipChanged {
    #[topic]
    pub beneficiary: Address,
    pub allowed: bool,
}

#[contract]
pub struct BeneficiaryWhitelist;

#[contractimpl]
impl BeneficiaryWhitelist {
    pub fn __constructor(env: Env, admin: Address) {
        admin.require_auth();
        env.storage().instance().set(&Key::Admin, &admin);
    }

    /// Adds a beneficiary. Returns true when membership changes.
    pub fn add(env: Env, beneficiary: Address) -> bool {
        Self::set_membership(&env, beneficiary, true)
    }

    /// Removes a beneficiary. Returns true when membership changes.
    pub fn remove(env: Env, beneficiary: Address) -> bool {
        Self::set_membership(&env, beneficiary, false)
    }

    pub fn contains(env: Env, beneficiary: Address) -> bool {
        env.storage().persistent().has(&Key::Member(beneficiary))
    }
}

impl BeneficiaryWhitelist {
    fn set_membership(env: &Env, beneficiary: Address, allowed: bool) -> bool {
        let admin: Address = env.storage().instance().get(&Key::Admin).unwrap();
        admin.require_auth();
        let key = Key::Member(beneficiary.clone());
        let storage = env.storage().persistent();
        if storage.has(&key) == allowed {
            return false;
        }
        if allowed {
            storage.set(&key, &());
            storage.extend_ttl(&key, env.storage().max_ttl(), env.storage().max_ttl());
        } else {
            storage.remove(&key);
        }
        MembershipChanged {
            beneficiary,
            allowed,
        }
        .publish(env);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events};

    #[test]
    fn membership_requires_admin_and_emits_events_only_on_changes() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let member = Address::generate(&env);
        env.mock_all_auths();
        let id = env.register(BeneficiaryWhitelist, (&admin,));
        let client = BeneficiaryWhitelistClient::new(&env, &id);
        env.set_auths(&[]);
        assert!(client.try_add(&member).is_err());
        assert!(!client.contains(&member));

        env.mock_all_auths();
        assert!(client.add(&member));
        assert_eq!(env.events().all().events().len(), 1);
        assert_eq!(env.auths()[0].0, admin);
        assert!(!client.add(&member));
        assert!(env.events().all().events().is_empty());
        assert!(client.contains(&member));

        env.set_auths(&[]);
        assert!(client.try_remove(&member).is_err());
        assert!(client.contains(&member));
        env.mock_all_auths();
        assert!(client.remove(&member));
        assert_eq!(env.events().all().events().len(), 1);
        assert!(!client.remove(&member));
        assert!(env.events().all().events().is_empty());
        assert!(!client.contains(&member));
    }
}
