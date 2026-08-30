//! Persistent storage helpers for the SoroWill contract.
//!
//! All will state lives in persistent storage keyed by a `DataKey`. Owner and
//! beneficiary indexes are maintained as `Vec<u64>` of will ids so the
//! contract can answer `get_wills_by_owner` / `get_wills_by_beneficiary`
//! without an off-chain indexer. Guardian votes are tracked per
//! `(will_id, guardian)` pair with a timestamp so they can expire over time,
//! and cleared independently when a guardian-release cycle resets.

use soroban_sdk::{contracttype, panic_with_error, Address, Env, Vec};

use crate::errors::WillError;
use crate::types::{
    Beneficiary, GuardianVoteReason, ProtocolStats, TokenLockedBalance, Will, WillStatus,
    WillStatusTransition,
};

/// Number of ledgers in one calendar day, assuming a **5-second average ledger
/// close time** on the Stellar network.
///
/// ```text
/// 86,400 seconds/day ÷ 5 seconds/ledger = 17,280 ledgers/day
/// ```
///
/// ## What to change if the network's close time shifts
///
/// If Stellar's average close time moves away from 5 seconds (it has drifted
/// historically as the network has evolved), update this constant to reflect
/// the new average:
///
/// ```text
/// new_value = 86_400 / new_average_close_time_in_seconds
/// ```
///
/// The change automatically propagates to `LIFETIME_THRESHOLD` and
/// `BUMP_AMOUNT` below, keeping the wall-clock safety margin intact without
/// requiring any further edits.
///
/// A redeployment is required to activate the updated value, because this
/// constant is compiled into the `.wasm` binary and is not configurable at
/// runtime.
///
/// ## Safety margin against close-time drift
///
/// `LIFETIME_THRESHOLD` and `BUMP_AMOUNT` are expressed as multiples of
/// `DAY_IN_LEDGERS` (30 days and 60 days respectively). This wide margin
/// deliberately over-provisions storage rent so that moderate close-time
/// drift — e.g. the average moving from 5 s to 6 s — does not immediately
/// imperil active wills: the bumped TTL would still cover roughly 50 real
/// days instead of 60, which is more than enough runway to notice and redeploy.
/// Only a sustained, large deviation (close time roughly doubling) would
/// shrink the effective window below a safe threshold.
const DAY_IN_LEDGERS: u32 = 17_280; // 86_400 s/day ÷ 5 s/ledger

/// Extend TTL once remaining lifetime drops below this many ledgers (~30 days
/// at the current 5-second close time; see `DAY_IN_LEDGERS` for how to adjust
/// this if the network's average close time changes).
const LIFETIME_THRESHOLD: u32 = DAY_IN_LEDGERS * 30;

/// Extend TTL out to this many ledgers when a bump is triggered (~60 days at
/// the current 5-second close time; see `DAY_IN_LEDGERS` for adjustment
/// guidance and the rationale for the extra safety margin).
const BUMP_AMOUNT: u32 = DAY_IN_LEDGERS * 60;

/// Seconds in a day, used to convert day-denominated expiry windows.
const SECONDS_PER_DAY: u64 = 86_400;

#[contracttype]
#[derive(Clone)]
pub(crate) enum DataKey {
    /// Monotonically increasing counter used to allocate will ids.
    NextWillId,
    /// Aggregate protocol statistics tracked on-chain.
    ProtocolStats,
    /// Full state of a will, keyed by its id.
    Will(u64),
    /// List of will ids owned by an address.
    OwnerWills(Address),
    /// List of will ids an address is named as a beneficiary of.
    BeneficiaryWills(Address),
    /// A guardian's vote record: stores `(timestamp, reason)` for time-weighted expiry.
    GuardianVote(u64, Address),
    /// A guardian's cancel-trigger vote record: separate namespace so release
    /// votes and cancel votes cannot interfere with each other.
    GuardianCancelVote(u64, Address),
    /// Audit trail of status transitions for a will.
    WillHistory(u64),
    /// Archived state of a will that has been Released or Cancelled.
    ArchivedWill(u64),
    /// Global index of all wills currently in `Triggered` status,
    /// used for efficient keeper/discovery queries.
    TriggeredWills,
}

/// The data stored for each guardian vote: the Unix timestamp when the vote
/// was cast and the reason code the guardian provided.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardianVoteRecord {
    pub timestamp: u64,
    pub reason: GuardianVoteReason,
}

/// Allocates and returns the next available will id, starting at `1`.
///
/// Also extends the contract instance's TTL on every call so the `NextWillId`
/// counter — and all other instance-storage entries — stay live for at least
/// [`LIFETIME_THRESHOLD`] more ledgers, with a bump to [`BUMP_AMOUNT`] when
/// that threshold is crossed. These constants mirror the values used for every
/// persistent-storage entry in the contract so the instance lifecycle is
/// managed consistently with the rest of the on-chain state.
pub fn next_will_id(env: &Env) -> u64 {
    let key = DataKey::NextWillId;
    let current: u64 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current + 1;
    env.storage().instance().set(&key, &next);
    // Keep the contract instance alive for as long as the longest-lived will
    // it could ever serve. Without an explicit bump here, the instance TTL is
    // only extended by whatever the platform applies automatically (if
    // anything), whereas every other storage entry in this codebase bumps its
    // own TTL. This call brings instance storage in line with that policy.
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    next
}

/// Loads the current protocol statistics from instance storage.
pub fn get_protocol_stats(env: &Env) -> ProtocolStats {
    let key = DataKey::ProtocolStats;
    env.storage()
        .instance()
        .get(&key)
        .unwrap_or_else(|| ProtocolStats {
            active_will_count: 0,
            total_locked_by_token: Vec::new(env),
        })
}

/// Persists protocol statistics in instance storage.
pub fn save_protocol_stats(env: &Env, stats: &ProtocolStats) {
    env.storage().instance().set(&DataKey::ProtocolStats, stats);
}

/// Increments the active-will counter by one.
pub fn increment_active_will_count(env: &Env) {
    let mut stats = get_protocol_stats(env);
    stats.active_will_count += 1;
    save_protocol_stats(env, &stats);
}

/// Decrements the active-will counter by one if it is greater than zero.
pub fn decrement_active_will_count(env: &Env) {
    let mut stats = get_protocol_stats(env);
    if stats.active_will_count > 0 {
        stats.active_will_count -= 1;
    }
    save_protocol_stats(env, &stats);
}

/// Adds or subtracts a token amount from the protocol's locked-balance
/// totals.
///
/// An entry whose `total_locked` returns to exactly zero is dropped rather
/// than kept as a permanent zero-balance row -- otherwise
/// `total_locked_by_token` grows by one entry per distinct token ever used,
/// forever, even for tokens with no current usage (#264).
pub fn adjust_locked_value(env: &Env, token: &Address, delta: i128) {
    let mut stats = get_protocol_stats(env);
    let mut found = false;
    let mut updated = Vec::new(env);
    for entry in stats.total_locked_by_token.iter() {
        if entry.token == *token {
            found = true;
            let next_total = entry.total_locked + delta;
            if next_total != 0 {
                updated.push_back(TokenLockedBalance {
                    token: entry.token.clone(),
                    total_locked: next_total,
                });
            }
        } else {
            updated.push_back(entry.clone());
        }
    }
    if !found && delta != 0 {
        updated.push_back(TokenLockedBalance {
            token: token.clone(),
            total_locked: delta,
        });
    }
    stats.total_locked_by_token = updated;
    save_protocol_stats(env, &stats);
}

/// Persists a will's state and refreshes its storage TTL.
/// Persists a will's state, refreshing its storage TTL unless the will has
/// reached a terminal state.
///
/// `Released` and `Cancelled` are terminal: every entry point that could touch
/// a will again first asserts it is `Active` or `Triggered`, so nothing can
/// ever mutate one. Extending the TTL of a terminal will buys 60 more days of
/// rent for an entry that only `get_will` can read, at a price proportional to
/// its size. The entry is still written — readers keep seeing the final state
/// for the remainder of the TTL bought by the will's last active operation —
/// it just stops being renewed.
pub fn save_will(env: &Env, will: &Will) {
    let key = DataKey::Will(will.id);
    env.storage().persistent().set(&key, will);
    if !matches!(will.status, WillStatus::Released | WillStatus::Cancelled) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }
}

/// Loads a will by id, returning `WillError::WillNotFound` if it does not exist.
///
/// # Archived-entry limitation (issue #166)
///
/// Soroban's persistent-storage API does not expose *why* a key is absent.
/// This function therefore maps every `None` result to
/// [`WillError::WillNotFound`], which conflates three materially different
/// situations:
///
/// 1. **Never created** — the id was never allocated by `create_will`.
/// 2. **Explicitly archived** — the will reached a terminal state and was
///    moved to the `ArchivedWill` namespace by [`archive_will`].
/// 3. **TTL-archived by the network** — the will's persistent entry lapsed
///    (terminal wills no longer renew their TTL, see [`save_will`]) and was
///    archived by Soroban once its TTL hit zero.
///
/// With soroban-sdk 22 there is **no API to distinguish an archived-but-
/// existing entry from a truly absent one**: `has`/`get` treat archived keys
/// as missing (in the test host they surface a testing-only diagnostic; on
/// the live network an invocation that touches an archived entry fails at the
/// host level before contract code runs). Callers — including the SDK and app,
/// which treat [`WillError::WillNotFound`] as "no such will" — cannot tell a
/// genuinely nonexistent will apart from one that exists on-chain but needs to
/// be restored, and a will that was explicitly archived cannot be located via
/// this function at all.
///
/// A dedicated `WillArchived` error code is deferred to a follow-up: it is
/// not implementable with the current SDK version, so this documentation is
/// the contract's contract with its consumers until the storage API grows an
/// archived-entry probe.
pub fn load_will(env: &Env, will_id: u64) -> Result<Will, WillError> {
    let key = DataKey::Will(will_id);
    env.storage()
        .persistent()
        .get(&key)
        .ok_or(WillError::WillNotFound)
}

/// Adds `will_id` to the index list stored at `key`, if not already present.
///
/// Enforces [`MAX_WILLS_PER_INDEX`]: if the list is already at the cap and
/// `will_id` is not already in it, this panics with [`WillError::TooManyWills`]
/// before any write happens. This prevents the serialised `Vec<u64>` from
/// growing large enough to hit Soroban's per-entry size ceiling, which would
/// cause a mid-`create_will` failure after the token transfer has already
/// succeeded.
///
/// The write and the TTL bump are skipped when the id is already indexed: the
/// resulting entry would be byte-for-byte identical, so paying to store it
/// again buys nothing.
fn index_push(env: &Env, key: DataKey, will_id: u64) {
    let mut ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if !ids.contains(will_id) {
        if ids.len() >= MAX_WILLS_PER_INDEX {
            panic_with_error!(env, WillError::TooManyWills);
        }
        ids.push_back(will_id);
        env.storage().persistent().set(&key, &ids);
        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }
}

/// Checks, without writing anything, that indexing a brand-new will for
/// `owner` and each of `beneficiaries` would not exceed
/// [`MAX_WILLS_PER_INDEX`] for any of them.
///
/// A newly allocated will id can never already be present in an existing
/// index, so this only needs to check each index's current length against
/// the cap -- unlike [`index_push`], it never needs the will id itself.
///
/// Called by `create_will` before any token transfer, so a call that was
/// always going to fail this cap does so on the cheap in-contract check
/// rather than after the transfer already succeeded (#260).
pub fn assert_index_capacity(env: &Env, owner: &Address, beneficiaries: &Vec<Beneficiary>) {
    assert_single_index_capacity(env, &DataKey::OwnerWills(owner.clone()));
    for beneficiary in beneficiaries.iter() {
        assert_single_index_capacity(env, &DataKey::BeneficiaryWills(beneficiary.address.clone()));
    }
}

fn assert_single_index_capacity(env: &Env, key: &DataKey) {
    let ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| Vec::new(env));
    if ids.len() >= MAX_WILLS_PER_INDEX {
        panic_with_error!(env, WillError::TooManyWills);
    }
}

/// Adds `will_id` to the index of wills owned by `owner`, if not already present.
pub fn index_by_owner(env: &Env, owner: &Address, will_id: u64) {
    index_push(env, DataKey::OwnerWills(owner.clone()), will_id);
}

/// Adds `will_id` to the index of wills `beneficiary` is named in, if not already present.
pub fn index_by_beneficiary(env: &Env, beneficiary: &Address, will_id: u64) {
    index_push(env, DataKey::BeneficiaryWills(beneficiary.clone()), will_id);
}

/// Removes `will_id` from the index of wills `beneficiary` is named in.
///
/// Used by `update_beneficiaries` to keep the reverse index accurate when a
/// beneficiary is dropped from a will. Returns without writing when the id is
/// not in the list, so an address that was never indexed costs a read instead
/// of a read plus a pointless rewrite.
///
/// Uses order-preserving removal to maintain the monotonic ordering invariant
/// that `paginate_ids` depends on.
///
/// The TTL is refreshed after every write so that an index entry that is only
/// ever pruned (never freshly added to) does not expire before the underlying
/// wills do.
pub fn remove_beneficiary_index(env: &Env, beneficiary: &Address, will_id: u64) {
    let key = DataKey::BeneficiaryWills(beneficiary.clone());
    let Some(ids) = env.storage().persistent().get::<_, Vec<u64>>(&key) else {
        return;
    };
    let Some(_index) = ids.first_index_of(will_id) else {
        return;
    };
    let mut new_ids = Vec::new(env);
    for id in ids.iter() {
        if id != will_id {
            new_ids.push_back(id);
        }
    }
    env.storage().persistent().set(&key, &new_ids);
    env.storage()
        .persistent()
        .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

/// Removes `will_id` from the index of wills owned by `owner`.
///
/// Used by `distribute` and `cancel_will` to keep the owner index accurate
/// after a will reaches a terminal state. Returns without writing when the id
/// is not in the list.
///
/// Uses order-preserving removal to maintain the monotonic ordering invariant
/// that `paginate_ids` depends on.
pub fn remove_owner_index(env: &Env, owner: &Address, will_id: u64) {
    let key = DataKey::OwnerWills(owner.clone());
    let Some(ids) = env.storage().persistent().get::<_, Vec<u64>>(&key) else {
        return;
    };
    let Some(_index) = ids.first_index_of(will_id) else {
        return;
    };
    let mut new_ids = Vec::new(env);
    for id in ids.iter() {
        if id != will_id {
            new_ids.push_back(id);
        }
    }
    env.storage().persistent().set(&key, &new_ids);
    env.storage()
        .persistent()
        .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

/// Returns the list of will ids owned by `owner`.
pub fn get_owner_wills(env: &Env, owner: &Address) -> Vec<u64> {
    let key = DataKey::OwnerWills(owner.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Returns the list of will ids `beneficiary` is named in.
pub fn get_beneficiary_wills(env: &Env, beneficiary: &Address) -> Vec<u64> {
    let key = DataKey::BeneficiaryWills(beneficiary.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Adds `will_id` to the global index of Triggered wills, if not already present.
pub fn index_triggered_will(env: &Env, will_id: u64) {
    let key = DataKey::TriggeredWills;
    let mut ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if !ids.contains(will_id) {
        ids.push_back(will_id);
        env.storage().persistent().set(&key, &ids);
        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }
}

/// Removes `will_id` from the global index of Triggered wills.
///
/// Called when a will transitions *away* from `Triggered` (e.g. via
/// `emergency_checkin` or `release_inheritance`). Returns without writing
/// when the id is not in the list, so the common case costs a read instead
/// of a read plus a rewrite.
pub fn unindex_triggered_will(env: &Env, will_id: u64) {
    let key = DataKey::TriggeredWills;
    let Some(mut ids) = env.storage().persistent().get::<_, Vec<u64>>(&key) else {
        return;
    };
    let Some(index) = ids.first_index_of(will_id) else {
        return;
    };
    ids.remove_unchecked(index);
    env.storage().persistent().set(&key, &ids);
}

/// Returns the full list of will ids currently in `Triggered` status.
pub fn get_triggered_wills(env: &Env) -> Vec<u64> {
    let key = DataKey::TriggeredWills;
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Returns the full vote record for `guardian` in the current trigger cycle
/// for `will_id`, or `None` if they have not voted.
pub fn get_guardian_vote(env: &Env, will_id: u64, guardian: &Address) -> Option<GuardianVoteRecord> {
    let key = DataKey::GuardianVote(will_id, guardian.clone());
    env.storage().persistent().get(&key)
}

/// Returns whether `guardian` has a non-expired vote in the current trigger
/// cycle for `will_id`. A vote is considered expired if `now - vote_timestamp`
/// exceeds `expiry_days * SECONDS_PER_DAY`.
pub fn has_guardian_voted(
    env: &Env,
    will_id: u64,
    guardian: &Address,
    now: u64,
    expiry_days: u64,
) -> bool {
    if let Some(record) = get_guardian_vote(env, will_id, guardian) {
        let expiry_secs = expiry_days * SECONDS_PER_DAY;
        now - record.timestamp <= expiry_secs
    } else {
        false
    }
}

/// Records that `guardian` has voted in the current trigger cycle for `will_id`,
/// storing the vote timestamp and reason code.
pub fn set_guardian_voted(
    env: &Env,
    will_id: u64,
    guardian: &Address,
    timestamp: u64,
    reason: GuardianVoteReason,
) {
    let key = DataKey::GuardianVote(will_id, guardian.clone());
    let record = GuardianVoteRecord { timestamp, reason };
    env.storage().persistent().set(&key, &record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

/// Clears all guardian votes cast against `will`, starting a fresh voting cycle.
///
/// Called whenever a will returns to `Active` (e.g. via `emergency_checkin`)
/// so that guardians can vote again in a subsequent incapacitation event.
///
/// `will.guardian_votes` is incremented in lockstep with every
/// [`set_guardian_voted`] and zeroed alongside every reset, so a zero count
/// means no `GuardianVote` entry exists for the current guardian list and the
/// removals — up to [`crate::MAX_GUARDIANS`] of them — can be skipped. This is
/// the common case: most wills never see a guardian vote at all.
pub fn reset_guardian_votes(env: &Env, will: &Will) {
    if will.guardian_votes == 0 {
        return;
    }
    for guardian in will.guardians.iter() {
        let key = DataKey::GuardianVote(will.id, guardian.address.clone());
        env.storage().persistent().remove(&key);
    }
}

/// Returns whether `guardian` has a non-expired cancel vote in the current
/// cycle for `will_id`.
pub fn has_guardian_cancel_voted(
    env: &Env,
    will_id: u64,
    guardian: &Address,
    now: u64,
    expiry_days: u64,
) -> bool {
    let key = DataKey::GuardianCancelVote(will_id, guardian.clone());
    if let Some(record) = env.storage().persistent().get::<_, GuardianVoteRecord>(&key) {
        let expiry_secs = expiry_days * SECONDS_PER_DAY;
        now - record.timestamp <= expiry_secs
    } else {
        false
    }
}

/// Records that `guardian` has cast a cancel-trigger vote for `will_id`.
pub fn set_guardian_cancel_voted(
    env: &Env,
    will_id: u64,
    guardian: &Address,
    timestamp: u64,
) {
    let key = DataKey::GuardianCancelVote(will_id, guardian.clone());
    let record = GuardianVoteRecord {
        timestamp,
        reason: GuardianVoteReason::Other,
    };
    env.storage().persistent().set(&key, &record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

/// Clears all guardian cancel-trigger votes for `will`, starting a fresh cycle.
///
/// Called when a cancel-trigger vote reaches quorum (returning the will to
/// `Active`) so the cancel-vote counters are clean for any future triggered state.
/// Skipped when `guardian_cancel_votes == 0` to avoid unnecessary storage reads.
pub fn reset_guardian_cancel_votes(env: &Env, will: &Will) {
    if will.guardian_cancel_votes == 0 {
        return;
    }
    for guardian in will.guardians.iter() {
        let key = DataKey::GuardianCancelVote(will.id, guardian.address.clone());
        env.storage().persistent().remove(&key);
    }
}

// ── Paginated index lookups ──────────────────────────────────────────────

/// Returns a bounded page of will ids from a `Vec<u64>` index.
///
/// If `cursor` is `Some(id)`, results start *after* that id (exclusive).
/// `limit` is capped at [`MAX_PAGE_SIZE`].
///
/// **ORDERING INVARIANT**: This function assumes the index vector stays
/// monotonically ordered (ascending). If entries are removed via
/// `remove_beneficiary_index` or `remove_owner_index`, they MUST use
/// order-preserving removal. Non-order-preserving removal (e.g. swap-with-last)
/// breaks the cursor contract, causing entries to be permanently skipped or
/// duplicated across pages.
pub fn paginate_ids(env: &Env, ids: &Vec<u64>, cursor: Option<u64>, limit: u32) -> Vec<u64> {
    let page_size = limit.min(MAX_PAGE_SIZE);
    let mut result = Vec::new(env);
    let mut skip = cursor.is_some();
    let cursor_val = cursor.unwrap_or(0);

    for id in ids.iter() {
        if skip {
            if id <= cursor_val {
                continue;
            }
            skip = false;
        }
        if result.len() >= page_size {
            break;
        }
        result.push_back(id);
    }
    result
}

/// Maximum number of will ids that a single address may have in its owner or
/// beneficiary index vector (`OwnerWills` / `BeneficiaryWills`).
///
/// Soroban persistent ledger entries have an upper bound on their serialized
/// size. An unbounded `Vec<u64>` index would eventually exceed that ceiling,
/// causing the append to fail mid-`create_will` — after the token transfer has
/// already succeeded — and permanently blocking any further creates for that
/// address. Capping the vector well below the theoretical limit prevents that
/// hard failure.
///
/// 1 000 entries × 8 bytes each = 8 KB of raw id data, comfortably inside
/// Soroban's max entry size and large enough to accommodate any realistic
/// on-chain use case.
///
/// When the cap is reached, [`WillError::TooManyWills`] is returned before the
/// token transfer happens, so no funds are ever locked in an un-indexable will.
#[cfg(not(test))]
pub const MAX_WILLS_PER_INDEX: u32 = 1_000;

/// Lowered cap used in unit tests so the limit can be exercised without
/// creating thousands of wills.
#[cfg(test)]
pub const MAX_WILLS_PER_INDEX: u32 = 5;

/// Maximum number of wills returned per page.
pub const MAX_PAGE_SIZE: u32 = 50;
/// Appends a status transition entry to `will_id`'s on-chain audit trail.
pub fn append_history(env: &Env, will_id: u64, transition: &WillStatusTransition) {
    let key = DataKey::WillHistory(will_id);
    let mut history: Vec<WillStatusTransition> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    history.push_back(transition.clone());
    env.storage().persistent().set(&key, &history);
    env.storage()
        .persistent()
        .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

/// Returns the full audit trail for `will_id`.
pub fn get_history(env: &Env, will_id: u64) -> Vec<WillStatusTransition> {
    let key = DataKey::WillHistory(will_id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Archives a will by moving it to the archived storage and removing it from
/// active storage and indexes. The archived will's TTL is not extended, so it
/// will eventually be garbage-collected by Soroban's state archival system.
pub fn archive_will(env: &Env, will: &Will) {
    // Move will to archived storage (no TTL extension)
    let archival_key = DataKey::ArchivedWill(will.id);
    env.storage().persistent().set(&archival_key, will);

    // Remove from active storage
    let active_key = DataKey::Will(will.id);
    env.storage().persistent().remove(&active_key);

    // Remove from owner index
    let owner_key = DataKey::OwnerWills(will.owner.clone());
    if let Some(ids) = env.storage().persistent().get::<_, Vec<u64>>(&owner_key) {
        let mut updated: Vec<u64> = Vec::new(env);
        for id in ids.iter() {
            if id != will.id {
                updated.push_back(id);
            }
        }
        env.storage().persistent().set(&owner_key, &updated);
    }

    // Remove from beneficiary indexes
    for beneficiary in will.beneficiaries.iter() {
        remove_beneficiary_index(env, &beneficiary.address, will.id);
    }
}

