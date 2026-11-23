//! Append-only audit records, stored separately from the will lifecycle.
use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::storage::MAX_PAGE_SIZE;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmergencyAction {
    OwnerCheckIn,
    GuardianTriggerVote,
    GuardianCancelVote,
    MigrateWill,
    ArchiveWill,
    RetryPayout,
    RepairProtocolStats,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyActionRecord {
    pub sequence: u64,
    pub timestamp: u64,
    pub action: EmergencyAction,
    /// Authenticated actor, when the entry point requires one.
    pub actor: Option<Address>,
}

/// Each will has its own log. Protocol-wide maintenance has a separate log.
#[contracttype]
#[derive(Clone)]
pub enum ActionLogScope {
    Will(u64),
    Protocol,
}

#[contracttype]
enum Key {
    ActionSequence(ActionLogScope),
    ActionRecord(ActionLogScope, u64),
}

pub(crate) fn append(
    env: &Env,
    scope: ActionLogScope,
    action: EmergencyAction,
    actor: Option<Address>,
) {
    let storage = env.storage().persistent();
    let ttl = env.storage().max_ttl();
    let counter_key = Key::ActionSequence(scope.clone());
    let sequence = storage.get::<_, u64>(&counter_key).unwrap_or(0) + 1;
    let record_key = Key::ActionRecord(scope, sequence);
    storage.set(
        &record_key,
        &EmergencyActionRecord {
            sequence,
            timestamp: env.ledger().timestamp(),
            action,
            actor,
        },
    );
    storage.set(&counter_key, &sequence);
    storage.extend_ttl(&record_key, ttl, ttl);
    storage.extend_ttl(&counter_key, ttl / 2, ttl);
}

pub(crate) fn page(
    env: &Env,
    scope: ActionLogScope,
    after: Option<u64>,
    limit: u32,
) -> Vec<EmergencyActionRecord> {
    let mut records = Vec::new(env);
    let storage = env.storage().persistent();
    let offset = after.unwrap_or(0);
    for index in 1..=u64::from(limit.min(MAX_PAGE_SIZE)) {
        let Some(sequence) = offset.checked_add(index) else {
            break;
        };
        let Some(record) = storage.get(&Key::ActionRecord(scope.clone(), sequence)) else {
            break;
        };
        records.push_back(record);
    }
    records
}
