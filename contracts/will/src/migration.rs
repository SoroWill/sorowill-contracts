//! Contract migration helper functions (#779).
//!
//! Utilities for migrating stored data between contract versions. The helpers
//! here are deliberately storage-agnostic: they operate on a caller-supplied
//! [`MigrationStore`] so they can be exercised in tests (and dry-runs) without
//! touching live contract storage.
//!
//! The flow is:
//! 1. [`validate_version`] — check the stored version is compatible with the
//!    target version before doing any work.
//! 2. [`dry_run`] — trial-migrate every record and validate the result without
//!    committing anything.
//! 3. [`migrate`] — apply the migration, snapshotting prior state first.
//! 4. [`rollback`] — restore the snapshot if the migration fails or is
//!    rejected.

use soroban_sdk::{contracttype, symbol_short, Env, Map, Symbol, Vec};

/// Storage key under which the current schema version is recorded.
const VERSION_KEY: Symbol = symbol_short!("MIG_VER");

/// A single stored record that can be migrated between versions.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationRecord {
    pub key: u32,
    pub version: u32,
    pub value: u64,
}

/// Outcome of a migration attempt.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MigrationOutcome {
    /// Migration applied successfully.
    Applied,
    /// Dry-run completed and every record validated.
    Validated,
    /// Migration was rejected; prior state was restored.
    RolledBack,
}

/// Abstraction over the storage a migration operates on. Implemented by the
/// contract's storage layer in production and by an in-memory map in tests.
pub trait MigrationStore {
    fn read_version(&self) -> u32;
    fn write_version(&mut self, version: u32);
    fn read_records(&self) -> Vec<MigrationRecord>;
    fn write_records(&mut self, records: &Vec<MigrationRecord>);
}

/// Validation helpers.
pub mod validate {
    use super::*;

    /// Returns `true` when `from` can be migrated to `to` (strictly forward).
    pub fn validate_version(from: u32, to: u32) -> bool {
        to > from
    }

    /// Returns `true` when every record is at `expected` version and its value
    /// is non-zero (the integrity invariant for migrated data).
    pub fn validate_records(records: &Vec<MigrationRecord>, expected: u32) -> bool {
        for record in records.iter() {
            if record.version != expected || record.value == 0 {
                return false;
            }
        }
        true
    }
}

/// Pure transformation applied to a single record when moving to `to`.
fn migrate_record(record: &MigrationRecord, to: u32) -> MigrationRecord {
    MigrationRecord {
        key: record.key,
        version: to,
        value: record.value,
    }
}

/// Trial-migrate every record to `to` and validate the result without
/// committing anything. Returns `true` when the migration would succeed.
pub fn dry_run<S: MigrationStore>(store: &S, to: u32) -> bool {
    let from = store.read_version();
    if !validate::validate_version(from, to) {
        return false;
    }
    let mut migrated = Vec::new(&Env::default());
    for record in store.read_records().iter() {
        migrated.push_back(migrate_record(&record, to));
    }
    validate::validate_records(&migrated, to)
}

/// Apply the migration to `to`, snapshotting prior state so it can be rolled
/// back. Returns the snapshot on success so the caller can retain it.
pub fn migrate<S: MigrationStore>(
    store: &mut S,
    to: u32,
) -> Result<(u32, Vec<MigrationRecord>), MigrationOutcome> {
    let from = store.read_version();
    if !validate::validate_version(from, to) {
        return Err(MigrationOutcome::RolledBack);
    }

    // Snapshot prior state before mutating anything.
    let snapshot_version = from;
    let snapshot_records = store.read_records();

    let mut migrated = Vec::new(&Env::default());
    for record in snapshot_records.iter() {
        migrated.push_back(migrate_record(&record, to));
    }

    if !validate::validate_records(&migrated, to) {
        // Restore prior state on validation failure.
        store.write_version(snapshot_version);
        store.write_records(&snapshot_records);
        return Err(MigrationOutcome::RolledBack);
    }

    store.write_records(&migrated);
    store.write_version(to);
    Ok((snapshot_version, snapshot_records))
}

/// Restore a previously captured snapshot, undoing a migration.
pub fn rollback<S: MigrationStore>(
    store: &mut S,
    snapshot_version: u32,
    snapshot_records: &Vec<MigrationRecord>,
) {
    store.write_records(snapshot_records);
    store.write_version(snapshot_version);
}

/// Record the current schema version in contract storage.
pub fn set_version(env: &Env, version: u32) {
    env.storage().instance().set(&VERSION_KEY, &version);
}

/// Read the current schema version from contract storage (0 if unset).
pub fn get_version(env: &Env) -> u32 {
    env.storage().instance().get(&VERSION_KEY).unwrap_or(0)
}

/// In-memory [`MigrationStore`] used by tests and dry-runs.
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    pub version: u32,
    pub records: Map<u32, MigrationRecord>,
}

impl MemoryStore {
    pub fn new(env: &Env, version: u32) -> Self {
        Self {
            version,
            records: Map::new(env),
        }
    }

    pub fn insert(&mut self, record: MigrationRecord) {
        self.records.set(record.key, record);
    }
}

impl MigrationStore for MemoryStore {
    fn read_version(&self) -> u32 {
        self.version
    }

    fn write_version(&mut self, version: u32) {
        self.version = version;
    }

    fn read_records(&self) -> Vec<MigrationRecord> {
        let mut out = Vec::new(&Env::default());
        for (_, record) in self.records.iter() {
            out.push_back(record);
        }
        out
    }

    fn write_records(&mut self, records: &Vec<MigrationRecord>) {
        let env = Env::default();
        let mut next = Map::new(&env);
        for record in records.iter() {
            next.set(record.key, record);
        }
        self.records = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_store(env: &Env, version: u32) -> MemoryStore {
        let mut store = MemoryStore::new(env, version);
        store.insert(MigrationRecord {
            key: 1,
            version,
            value: 100,
        });
        store.insert(MigrationRecord {
            key: 2,
            version,
            value: 250,
        });
        store
    }

    #[test]
    fn validate_version_rejects_backwards() {
        assert!(validate::validate_version(1, 2));
        assert!(!validate::validate_version(2, 2));
        assert!(!validate::validate_version(2, 1));
    }

    #[test]
    fn dry_run_does_not_commit() {
        let env = Env::default();
        let store = sample_store(&env, 1);
        assert!(dry_run(&store, 2));
        assert_eq!(store.read_version(), 1);
    }

    #[test]
    fn migrate_then_rollback_restores_state() {
        let env = Env::default();
        let mut store = sample_store(&env, 1);
        let (snapshot_version, snapshot) = migrate(&mut store, 2).unwrap();
        assert_eq!(store.read_version(), 2);
        assert!(validate::validate_records(&store.read_records(), 2));

        rollback(&mut store, snapshot_version, &snapshot);
        assert_eq!(store.read_version(), 1);
        assert!(validate::validate_records(&store.read_records(), 1));
    }

    #[test]
    fn migrate_rejects_invalid_target() {
        let env = Env::default();
        let mut store = sample_store(&env, 2);
        assert_eq!(migrate(&mut store, 1), Err(MigrationOutcome::RolledBack));
        assert_eq!(store.read_version(), 2);
    }
}
