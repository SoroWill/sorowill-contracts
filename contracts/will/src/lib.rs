// The deployed contract is `no_std`. The test and fuzzing harnesses need
// `std` (the Soroban test host, `proptest` and `libfuzzer-sys` all pull it
// in), so `std` is linked only for those configurations. The wasm build,
// which sees neither `cfg(test)` nor the `fuzzing` feature, stays `no_std`.
#![cfg_attr(not(any(test, feature = "fuzzing")), no_std)]
// `create_will` legitimately needs more than clippy's default 7-argument
// threshold (token/beneficiary/period/guardian/bounty/confirmation config
// all belong at creation time). `#[contractimpl]`'s generated dispatch code
// inherits that arg count on a synthetic item the attribute doesn't reach
// when placed on the source function or the impl block, so it's set crate-wide.
#![allow(clippy::too_many_arguments)]

//! SoroWill — a trustless on-chain inheritance and dead man's switch protocol
//! for Stellar Soroban.
//!
//! An owner locks one or more tokens (e.g. USDC, XLM, any SEP-41 asset) into
//! a `Will`, names beneficiaries with percentage shares, and periodically
//! calls [`WillContract::check_in`] to prove they are still active. If the
//! owner misses a check-in deadline, anyone may call
//! [`WillContract::trigger_will`] to start a grace period. The owner can
//! still call [`WillContract::emergency_checkin`] during the grace period to
//! prove they are alive and reset the countdown. If the grace period elapses
//! without an emergency check-in, anyone may call
//! [`WillContract::release_inheritance`] to split every locked token balance
//! among the beneficiaries proportionally to their configured percentages.
//!
//! Optionally, up to three guardians may be named on a will; any two of them
//! calling [`WillContract::guardian_trigger`] force an immediate release,
//! bypassing the check-in/grace-period flow entirely (e.g. if the owner is
//! known to be incapacitated). Guardians are named on the will at creation
//! time (or via [`WillContract::update_guardians`]) and may vote once the
//! guardian-list cooldown has elapsed. Guardian
//! votes expire after a configurable window so stale votes cannot combine
//! with fresh ones: when a vote ages out and the same guardian votes again,
//! the new vote *replaces* the expired one rather than adding to it, so one
//! guardian can never reach the quorum threshold alone (#372).
//!
//! Distribution is **push-only**: `distribute` transfers tokens directly to
//! each beneficiary in a single call. There is no pull mode and no
//! `claim_share` entry point. ([`WillContract::reveal_and_claim`] is unrelated
//! to this — it only pays out a hashed, not-yet-revealed beneficiary's
//! reserved share.)
//!
//! A grace period is a single configurable window: the will is released once,
//! in full, once that window has fully elapsed.

mod batch_check_in_limit;
mod errors;
mod events;
mod migration;
mod guardian_vote_freshness;
mod split_uniqueness_check;
mod storage;
mod types;

/// Reusable harness that drives entry points with arbitrary input and asserts
/// the contract's invariants. Shared by the `proptest` suite in
/// [`fuzz_test`] and by the `cargo-fuzz` targets under `fuzz/`.
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzz_harness;
/// Resource-cost profile for every public entry point. Measurement rather
/// than assertion — see the module docs for how to read the numbers.
#[cfg(test)]
mod profile;

#[cfg(test)]
mod fuzz_test;

/// Unit tests for the mixed percentage/fixed-amount `Allocation` model.
#[cfg(test)]
mod allocation_test;

/// Unit test asserting on the extended `will_created` event payload.
#[cfg(test)]
mod event_test;

/// Regression test: a beneficiary-list change must be the list actually
/// used when the will is later triggered and released.
#[cfg(test)]
mod beneficiary_lifecycle_test;

/// Cursor- and limit-based pagination regression tests for owner and
/// beneficiary lookups.
#[cfg(test)]
mod pagination_test;

/// Regression tests for paginated owner-status queries and related edge cases.
#[cfg(test)]
mod regression_test;

/// Malicious/reentrant SEP-41 token mock used for reentrancy regression
/// testing. See the module docs for details.
#[cfg(test)]
mod test_support;

/// Unit tests for the `will_created` event payload extended with
/// per-token breakdowns (see event_snapshot_test module docs).
#[cfg(test)]
mod event_snapshot_test;

/// Regression coverage for triggered-will lifecycle bookkeeping.
#[cfg(test)]
mod triggered_wills_test;

/// Guarded-release and cooldown regression tests for guardian voting.
#[cfg(test)]
mod guardian_cancel_test;

/// XDR spec fixture test for `create_will` encoding stability (#4).
#[cfg(test)]
mod test_xdr_spec;

/// Regression test for issue #190: ensure `distribute()` uses the overflow-safe
/// `proportional_share` helper for beneficiary-payout calculations.
#[cfg(test)]
mod distribute_overflow_safety_test;

/// Regression test for issue #183: `merge_wills` resets guardian state without
/// reinitialising the vote-weight accumulator.
#[cfg(test)]
mod issue_183_test;
#[cfg(test)]
mod issue_354_test;
#[cfg(test)]
mod issue_355_test;
#[cfg(test)]
mod issue_356_test;
#[cfg(test)]
mod issue_357_test;
/// Regression tests for issues #358-#361: the distinct-token cap in `top_up`,
/// the shared guardian-vote expiry rule, `limit == 0` pagination parity in
/// `get_wills_by_owner_and_status`, and renouncing the last beneficiary.
#[cfg(test)]
mod issue_358_test;
#[cfg(test)]
mod issue_359_test;
#[cfg(test)]
mod issue_360_test;
#[cfg(test)]
mod issue_361_test;
/// Regression tests for issues #378-#381: keeper-bounty token selection,
/// guardian dedup by address, hashed beneficiaries in a merge, and the merge
/// status transitions.
#[cfg(test)]
mod issue_378_381_test;
#[cfg(test)]
mod issue_414_test;

/// Regression test for issue #424: `get_time_until_deadline` returns a
/// negative `i64` (not a huge wrapped value) when the deadline is in the past.
#[cfg(test)]
mod issue_424_test;

/// Regression tests for issue #425: guardian-vote invariant after
/// `guardian_cancel_trigger` reaches quorum — vote state is cleared but the
/// guardian list and threshold are preserved for subsequent voting cycles.
#[cfg(test)]
mod issue_425_test;

/// Regression tests for issue #426: `distribute` produces consistent payouts
/// regardless of the order `Percentage` and `FixedAmount` beneficiaries appear
/// in the list.
#[cfg(test)]
mod issue_426_test;

#[cfg(test)]
mod issue_370_test;
#[cfg(test)]
mod issue_371_test;

/// Regression tests for issue #427: `get_protocol_stats().total_locked_by_token`
/// stays consistent across create / top-up / cancel / release operations.
#[cfg(test)]
mod issue_427_test;

/// Tests for issues #502, #503, #504 and #505, whose reported invariants are
/// each enforced by the current code; these pin the behaviour.
#[cfg(test)]
mod issues_502_505_test;

/// Regression tests for issue #499: topping up a token the will already
/// holds a balance for accumulates (`existing + amount`), never overwrites
/// or duplicates -- `will.balances` is a `Map`, whose keys are unique by
/// construction.
#[cfg(test)]
mod issue_499_test;

/// Regression tests for issue #500: check-ins now append a sequenced entry to
/// the same audit trail status transitions use, and every entry (check-in or
/// status transition) carries a monotonic per-will `seq` that survives
/// `MAX_HISTORY_ENTRIES` trimming without gaps or duplicates.
#[cfg(test)]
mod issue_500_test;

/// Regression test for issue #184: `merge_wills` refuses mismatched primary tokens.
#[cfg(test)]
mod issue_184_test;

/// Regression test for issue #185: `add_hashed_beneficiary` emits its lifecycle event.
#[cfg(test)]
mod issue_185_test;

/// Regression test for issue #186: hashed-beneficiary percentages must use the
/// same basis-point scale as the rest of the contract.
#[cfg(test)]
mod issue_186_test;

/// Regression test for issue #187: ensure `merge_wills` decrements the active
/// will count when marking the merged will as cancelled.
#[cfg(test)]
mod merge_active_count_test;

/// Regression test for issue #188: ensure `merge_beneficiaries` does not silently
/// drop a beneficiary whose merged share rounds down to 0 basis points.
#[cfg(test)]
mod merge_rounding_test;

/// Regression tests for issues #383 (leftover refund on `FixedAmount`-only
/// wills), #384 (`FixedAmount` denominated in the primary token) and #385
/// (`distribute`'s doc block was attached to `proportional_share`).
#[cfg(test)]
mod issue_383_385_test;

/// Regression test for issue #189: ensure `merge_wills` preserves `Allocation::FixedAmount`
/// beneficiaries' fixed-amount semantics instead of converting to `Allocation::Percentage`.
#[cfg(test)]
mod merge_fixed_amount_test;

/// Tests for the `update_guardians` / `update_will_settings` threshold-safety
/// check: a new guardian list that would leave `guardian_threshold` unreachable
/// must be rejected with `InvalidGuardianThreshold`.
#[cfg(test)]
mod update_guardians_threshold_test;

/// Regression test for issue #299: `top_up` supports adding a token that was
/// not locked at `create_will` time (new-token path).
#[cfg(test)]
mod issue_299_test;

/// Regression tests for issues #350-#353: duplicate-token rejection and
/// mirror/balances agreement (#350), real from/to statuses in the audit trail
/// (#351), `confirm_will`/`close_will` history entries (#352), and per-token
/// locked-value decrements on cancellation (#353).
#[cfg(test)]
mod issue_350_353_test;

/// Regression test: `has_guardian_voted` / `has_guardian_cancel_voted` use a
/// checked elapsed-time subtraction and cannot underflow when `now` precedes
/// the recorded vote timestamp.
#[cfg(test)]
mod issue_guardian_vote_underflow_test;

/// Regression tests for issue #372: expired guardian votes are dropped from the
/// `guardian_vote_weight` / `guardian_votes` tallies in both
/// `guardian_trigger` and `guardian_cancel_trigger`.
#[cfg(test)]
mod issue_368_test;
#[cfg(test)]
mod issue_369_test;
#[cfg(test)]
mod issue_372_test;

// The following test modules exist as files but were never wired into this
// module tree by the PRs that added them, so they silently never compiled or
// ran under `cargo test`.
#[cfg(test)]
mod archive_will_test;
#[cfg(test)]
mod batch_create_test;
#[cfg(test)]
mod clone_will_test;
#[cfg(test)]
mod confirm_will_test;
#[cfg(test)]
mod contract_interface_test;
#[cfg(test)]
mod create_will_doc_example_test;
#[cfg(test)]
mod entrypoint_coverage_test;
#[cfg(test)]
mod get_contract_version_test;
#[cfg(test)]
mod get_time_until_deadline_test;
#[cfg(test)]
mod get_will_history_test;
#[cfg(test)]
mod get_will_status_test;
#[cfg(test)]
mod get_wills_test;
#[cfg(test)]
mod hashed_beneficiary_test;
#[cfg(test)]
mod issue_259_test;
#[cfg(test)]
mod issue_260_test;
#[cfg(test)]
mod issue_261_test;
#[cfg(test)]
mod issue_262_test;
#[cfg(test)]
mod issue_263_test;
#[cfg(test)]
mod issue_264_test;
#[cfg(test)]
mod issue_265_test;
#[cfg(test)]
mod issue_266_test;
#[cfg(test)]
mod issue_272_test;
#[cfg(test)]
mod issue_277_test;
/// Regression tests for issue #501: `.github/scripts/check-contract-version.sh`
/// (referenced by CI and CONTRIBUTING.md but never actually committed) is
/// added alongside these, plus a post-deploy on-chain check in
/// `scripts/deploy-testnet.sh`, since a Soroban contract cannot read its own
/// deployed Wasm's metadata back at runtime.
#[cfg(test)]
mod issue_501_test;
#[cfg(test)]
mod issue_278_test;
#[cfg(test)]
mod issue_279_test;
#[cfg(test)]
mod issue_280_test;
#[cfg(test)]
mod issue_281_test;
#[cfg(test)]
mod issue_282_test;
#[cfg(test)]
mod issue_298_283_294_test;
#[cfg(test)]
mod issue_362_test;
#[cfg(test)]
mod issue_363_test;
#[cfg(test)]
mod issue_390_test;
#[cfg(test)]
mod issue_420_test;
#[cfg(test)]
mod issue_422_test;
#[cfg(test)]
mod migrate_will_test;
#[cfg(test)]
mod protocol_stats_test;
/// Regression tests for issue #498: `audit_protocol_stats`/`repair_protocol_stats`
/// (added for #441) had no test coverage at all, and `ProtocolStatsAudit` --
/// the type `audit_protocol_stats` returns -- was referenced but never
/// defined, a compile error fixed alongside these tests.
#[cfg(test)]
mod audit_protocol_stats_test;
#[cfg(test)]
mod renounce_validation_test;
#[cfg(test)]
mod set_delegate_test;
#[cfg(test)]
mod split_will_test;
#[cfg(test)]
mod issue_490_493_test;
// NOTE: `test.rs` (5800+ lines) is intentionally NOT wired in here. It
// predates the current multi-token/Allocation-enum contract API entirely
// (it exclusively uses a removed single-token `basis_points` signature) and
// references at least two features that no longer exist in lib.rs at all
// (`fallback_beneficiary`, `GraceTier`/`release_tier`). It appears to be
// dead code left over from before a major API redesign, later scrambled
// further by conflicting merges. Reactivating it would mean reimplementing
// removed contract functionality, which is out of scope for a merge-damage
// cleanup — left disconnected until someone decides what to do with it.
#[cfg(test)]
mod uncovered_entrypoints_test;
#[cfg(test)]
mod update_will_settings_test;
#[cfg(test)]
mod wills_by_owner_status_test;
#[cfg(test)]
mod issue_486_489_test;

use soroban_sdk::{
    contract, contractimpl, log, panic_with_error, symbol_short, token, Address, Bytes, Env, Map,
    Vec,
};

pub use errors::WillError;
pub use migration::CURRENT_SCHEMA_VERSION;
pub use types::{
    Allocation, Beneficiary, Guardian, GuardianVoteReason, HashedBeneficiary, OwnerStats,
    ProtocolStats, TokenLockedBalance, Will, WillStatus, WillStatusTransition,
    contract, contractimpl, panic_with_error, symbol_short, token, xdr::ToXdr, Address, Bytes, Env,
    Map, Vec,
};

pub use errors::WillError;
// Re-exported so the schema version has one definition (`storage`) and one
// import path (`crate::CURRENT_SCHEMA_VERSION`) for entry points and tests.
pub use storage::GuardianVoteRecord;
pub use storage::CURRENT_SCHEMA_VERSION;
// NOTE(ci-cleanup): a bad merge had triple-duplicated this list (Allocation,
// Beneficiary, ProtocolStats, Will, etc. each appearing three times), which
// also hid that ProtocolStatsAudit (used by audit_protocol_stats below) was
// never actually defined in types.rs -- fixed alongside this (#498).
pub use types::{
    Allocation, Beneficiary, Guardian, GuardianConsent, GuardianSpec, GuardianVoteReason,
    HashedBeneficiary, ProtocolStats, ProtocolStatsAudit, Will, WillPage, WillStatus,
    WillStatusTransition,
};

/// Semantic version of the contract logic, encoded as
/// `major * 1_000_000 + minor * 1_000 + patch`.
///
/// Bump this constant in every PR that changes observable contract behaviour
/// so that SDKs and apps can detect version mismatches at runtime via
/// [`WillContract::get_contract_version`].
///
/// Current value: **1.3.0** → `1_003_000`. 1.1.0 made `get_triggered_wills`
/// take a `(cursor, limit)` page instead of returning the whole index (#368);
/// 1.2.0 bound `reveal_and_claim` to the address in the pre-image (#369);
/// 1.3.0 added `WillStatusTransition::seq` and made `check_in`/
/// `batch_check_in` append audit-trail entries (#500).
pub const CONTRACT_VERSION: u32 = 1_003_000;

/// Number of seconds in a day, used to convert the day-denominated periods
/// stored on a `Will` into absolute ledger timestamps.
const SECONDS_PER_DAY: u64 = 86_400;

/// Maximum number of beneficiaries a single will may have: **10**.
///
/// Enforced by `create_will`, `batch_create_wills`, `update_beneficiaries`,
/// `update_will_settings` and `merge_wills`. Supplying more panics with
/// [`WillError::TooManyBeneficiaries`] (code `12`), or
/// [`WillError::MergeWouldExceedLimits`] (code `16`) for a merge.
///
/// The cap exists because `release_inheritance` pays every beneficiary in a
/// single transaction: each payout is a token transfer, and an unbounded list
/// could push the release past Soroban's per-transaction CPU, memory and
/// ledger-entry limits, leaving the funds permanently unreleasable. See the
/// README's "FAQ: why is there a beneficiary limit?" for details and
/// workarounds (e.g. splitting across several wills).
///
/// Re-exported at the crate root so off-chain tooling can reference the
/// canonical value without hardcoding a duplicate.
pub const MAX_BENEFICIARIES: u32 = 10;

/// Maximum number of guardians a single will may have.
///
/// A will can name at most this many guardian addresses. The value is an
/// upper bound for the guardian list and is enforced at `create_will` and
/// `update_guardians` time.
const MAX_GUARDIANS: u32 = 3;

/// Maximum length, in days, of a will's check-in or grace period (10 years).
///
/// Periods are converted to absolute timestamps by multiplying by
/// [`SECONDS_PER_DAY`]. Bounding them here guarantees that conversion can
/// never overflow the `u64` ledger timestamp, which would otherwise panic
/// outright — or, worse, produce a will whose deadline is unreachable, so
/// that `trigger_will` can never run and the locked balance can never be
/// released.
const MAX_PERIOD_DAYS: u64 = 3_650;
/// Maximum number of distinct tokens a single will may hold.
const MAX_TOKENS: u32 = 10;

/// Upper bound on beneficiary transfers a single `release_inheritance` (or
/// guardian-quorum release) performs (#491).
///
/// `distribute` makes at most one transfer per beneficiary per token, and a
/// will names at most [`MAX_BENEFICIARIES`] beneficiaries and holds at most
/// `MAX_TOKENS` tokens, so one release is bounded by this many beneficiary
/// payouts, plus at most one keeper bounty and one owner refund per token.
/// Distribution therefore needs no pagination: the work is bounded by
/// construction rather than by the caller.
pub const MAX_RELEASE_PAYOUTS: u32 = MAX_BENEFICIARIES * MAX_TOKENS;

/// Exact byte length of a hashed beneficiary's pre-image: a 32-byte address
/// fingerprint followed by a 32-byte salt chosen by the beneficiary at
/// registration time.
///
/// Layout: `sha256(xdr(beneficiary_address))[0..32] || salt(32)`. See
/// `reveal_and_claim`'s "Pre-image layout" section for the full description of
/// the fingerprint and why it is used in place of the address bytes (#369).
///
/// `reveal_and_claim` enforces this length *before* hashing and rejects
/// anything else with [`WillError::InvalidPreimageLength`], rather than letting
/// a wrong-length input fall through to a generic
/// [`WillError::InvalidPreimage`] after a wasted SHA-256 (#370). It then
/// requires the fingerprint half to match `claimant`
/// ([`WillError::PreimageAddressMismatch`], #369) — the pre-image is public
/// once broadcast, so without that binding anyone who observed it could replay
/// it with their own address and take the share. Fixed here because a
/// commitment is a SHA-256 digest, so an owner who registers a commitment over
/// a different-length pre-image can never produce a matching reveal.
pub const PREIMAGE_LENGTH: u32 = 64;

/// Byte length of the address half of a pre-image: the first 32 bytes. The
/// remainder of the [`PREIMAGE_LENGTH`]-byte pre-image is the salt.
///
/// A Soroban address does not serialise to 32 bytes, so this half holds a
/// 32-byte fingerprint of the address rather than the address itself. See
/// `reveal_and_claim`'s "Pre-image layout" section.
pub const PREIMAGE_ADDRESS_LENGTH: u32 = 32;

/// Number of distinct guardian votes required to force an early release.
///
/// This default threshold is used when a caller does not supply an explicit
/// `guardian_threshold` to `create_will`. Individual wills may override the
/// threshold within the range `1..=guardians.len()`.
///
/// **Invariant:** `GUARDIAN_THRESHOLD` must not exceed `MAX_GUARDIANS`.
/// If it did, no will could ever satisfy the default quorum because a will
/// can hold at most `MAX_GUARDIANS` guardians. The compile-time assertion
/// immediately below enforces this relationship so the two constants can
/// never drift apart silently during future refactors.
const GUARDIAN_THRESHOLD: u32 = 2;

/// Compile-time guard: the default guardian threshold must never exceed the
/// maximum number of guardians a will may hold. Violating this relationship
/// would make the default quorum permanently unreachable.
const _: () = assert!(
    GUARDIAN_THRESHOLD <= MAX_GUARDIANS,
    "GUARDIAN_THRESHOLD must be <= MAX_GUARDIANS; \
     a threshold that exceeds the guardian limit can never be reached"
);

/// Maximum number of wills that can be created in a single batch call.
const BATCH_MAX: u32 = 10;

/// Cooldown period in days after a guardian-list change before `guardian_trigger`
/// takes effect. Prevents a compromised owner from swapping guardians right
/// before attempting something malicious.
const GUARDIAN_COOLDOWN_DAYS: u64 = 7;

/// Maximum keeper bounty in basis points (100 bps = 1%).
const MAX_KEEPER_BOUNTY_BPS: u32 = 100;

/// Maximum number of ids that can be passed to `get_wills` in a single call.
const MAX_GET_WILLS_IDS: u32 = 50;

/// Maximum vote weight a single guardian may be given on a will.
///
/// `GuardianSpec::weight` is a caller-supplied `u32` with no inherent bound,
/// while both the quorum check in `update_guardians_weighted` and the live
/// tallies in `storage::recount_guardian_votes` are `u32`. Bounding the
/// per-guardian weight keeps a list of at most [`MAX_GUARDIANS`] guardians far
/// inside `u32` — the largest reachable total is `3 * MAX_GUARDIAN_WEIGHT` —
/// so a total-weight computation can no longer be pushed to overflow by a
/// caller (#356).
///
/// A million-fold weighting already expresses any practical preference between
/// guardians (say 1 against 1_000_000 for a single dominant guardian), so the
/// cap costs no real expressiveness. A weight of `0` is rejected with
/// [`WillError::InvalidGuardianThreshold`] rather than normalised to `1` (#489).
pub const MAX_GUARDIAN_WEIGHT: u32 = 1_000_000;

soroban_sdk::contractmeta!(
    key = "Description",
    val = "Trustless on-chain inheritance and dead man's switch protocol for Stellar Soroban"
);
// Kept in sync with CONTRACT_VERSION's semver-decoded form by
// issue_272_test.rs; bump both together.
soroban_sdk::contractmeta!(key = "Version", val = "1.3.0");
soroban_sdk::contractmeta!(
    key = "Homepage",
    val = "https://github.com/SoroWill/sorowill-contracts"
);

#[contract]
pub struct WillContract;

#[contractimpl]
impl WillContract {
    /// Creates a new will, locking one or more token balances in the contract.
    ///
    /// If `confirmation_delay_seconds` is **0** the will starts `Active`
    /// immediately (backwards-compatible behaviour). If it is **> 0** the will
    /// starts in `PendingConfirmation` and the owner must call `confirm_will`
    /// within that window; the check-in clock does not start until confirmation.
    ///
    /// # Parameters
    /// - `owner`: the address creating the will; must authorize this call.
    /// - `tokens`: a list of `(token_address, amount)` pairs to lock. Each
    ///   token address must be unique, each amount must be positive, and the
    ///   list must contain between 1 and `MAX_TOKENS` entries.
    /// - `beneficiaries`: 1 to `MAX_BENEFICIARIES` entries. Each entry carries
    ///   an [`Allocation`], which is one of two kinds:
    ///   - `Allocation::Percentage(bps)` — a share of the will's balance. **All
    ///     percentage allocations together must sum to exactly 10,000 basis
    ///     points**, and each individual percentage must be greater than
    ///     zero.
    ///   - `Allocation::FixedAmount(amount)` — an exact claim on the will's
    ///     **primary token** (the first entry of `tokens`). Fixed amounts do
    ///     not take part in the 10,000 bps sum, but they may not add up to more
    ///     than the will holds of that primary token
    ///     ([`WillError::FixedAmountExceedsBalance`]).
    ///
    ///   A list made up only of `Allocation::FixedAmount` entries may leave
    ///   value unallocated — whatever the fixed amounts do not claim is
    ///   refunded to the owner when the inheritance is released (see
    ///   [`Allocation`] and issue #383).
    /// - `checkin_period_days`: how many days the owner may go without checking
    ///   in; 1 to `MAX_PERIOD_DAYS`.
    /// - `grace_period_days`: how many days after being triggered the owner has
    ///   to prove they are alive; 1 to `MAX_PERIOD_DAYS`.
    /// - `guardians`: 0 to `MAX_GUARDIANS` distinct addresses that may jointly
    ///   force an early release.
    /// - `guardian_threshold`: number of guardian votes required to trigger.
    ///   **Ignored, and stored as `0`, when `guardians` is empty** — the
    ///   guardian mechanism is disabled without guardians, so any value would
    ///   describe an unreachable quorum (#363). When the list is non-empty the
    ///   value must be between 1 and `guardians.len()`.
    /// - `keeper_bounty_bps`: optional keeper bounty in basis points.
    ///
    /// # Returns
    /// The newly allocated will id.
    ///
    /// # Panics
    /// - [`WillError::ZeroAmount`] if any token amount is not positive.
    /// - [`WillError::TooManyBeneficiaries`] if the beneficiary/guardian/token lists are
    ///   empty or exceed their respective caps (at most [`MAX_BENEFICIARIES`] = 10
    ///   beneficiaries).
    /// - [`WillError::InvalidPercentages`] if beneficiary basis points do not sum to 10,000.
    /// - [`WillError::TooManyBeneficiaries`] if the beneficiary or guardian
    ///   lists are empty or exceed their respective caps.
    /// - [`WillError::InvalidTokenCount`] if the token list is empty or
    ///   exceeds `MAX_TOKENS`.
    /// - [`WillError::InvalidPercentages`] if the `Allocation::Percentage`
    ///   entries do not sum to 10,000, or any single percentage is zero.
    /// - [`WillError::FixedAmountExceedsBalance`] if the
    ///   `Allocation::FixedAmount` entries add up to more than the will holds
    ///   of its primary token.
    /// - [`WillError::DuplicateBeneficiary`] if the same address is supplied twice.
    /// - [`WillError::DuplicateGuardian`] if the same guardian is supplied twice.
    /// - [`WillError::DuplicateToken`] if the same token address appears more
    ///   than once in `tokens`.
    /// - [`WillError::InvalidGuardianThreshold`] if `guardians` is non-empty
    ///   and `guardian_threshold` is not in `1..=guardians.len()`. A
    ///   non-empty threshold with an *empty* `guardians` list is not an error:
    ///   the argument is ignored and `0` is stored (#363).
    /// - [`WillError::InvalidPeriod`] if either period is zero or exceeds
    ///   [`MAX_PERIOD_DAYS`].
    /// - [`WillError::InvalidToken`] if any supplied token address does not respond to a
    ///   read-only `decimals()` probe. This is a best-effort sanity check, not a
    ///   full SEP-41 compliance guarantee: a contract that implements `decimals()`
    ///   but not `transfer`/`balance` correctly will pass this probe and only fail
    ///   later, when the transfer below is actually attempted (which aborts the
    ///   whole call, so no funds are ever at risk -- it just means `InvalidToken`
    ///   is not a substitute for verifying a token address's full interface
    ///   out-of-band before calling `create_will`).
    ///
    /// # Examples
    ///
    /// The snippet below is the rustdoc copy. The **compiled** version of the
    /// same example lives in `create_will_doc_example_test.rs`
    /// (`doc_example_creates_a_single_beneficiary_will`), which `cargo test`
    /// actually runs — rustdoc blocks tagged `ignore` are never compiled, so
    /// this is what keeps the two from drifting apart again (#366).
    ///
    /// ```ignore
    /// // Set up the environment and register the contract (test harness only).
    /// let env = Env::default();
    /// env.mock_all_auths();
    /// let contract_id = env.register(WillContract, ());
    /// let client = WillContractClient::new(&env, &contract_id);
    ///
    /// // Mint some USDC to the owner via a Stellar Asset Contract.
    /// let owner = Address::generate(&env);
    /// let usdc_id = env.register_stellar_asset_contract_v2(owner.clone()).address();
    /// StellarAssetClient::new(&env, &usdc_id).mint(&owner, &1_000_000);
    ///
    /// let beneficiary = Address::generate(&env);
    ///
    /// // Create a will: lock 1 USDC, a single percentage beneficiary taking the
    /// // whole balance, 90-day check-in, 7-day grace period, no guardians.
    /// let will_id = client.create_will(
    ///     &owner,
    ///     &vec![&env, (usdc_id.clone(), 1_000_000_i128)],
    ///     &vec![
    ///         &env,
    ///         Beneficiary {
    ///             address: beneficiary.clone(),
    ///             // A percentage allocation is in basis points, and all
    ///             // percentage allocations together must sum to 10_000.
    ///             allocation: Allocation::Percentage(10_000),
    ///         },
    ///     ],
    ///     &90,  // checkin_period_days
    ///     &7,   // grace_period_days
    ///     &vec![&env],  // no guardians
    ///     &0,           // guardian_threshold is ignored when there are no guardians
    ///     &None,        // no keeper bounty
    ///     &0,           // confirmation_delay_seconds (0 = starts Active immediately)
    /// );
    ///
    /// let will = client.get_will(&will_id);
    /// assert_eq!(will.owner, owner);
    /// assert_eq!(will.status, WillStatus::Active);
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn create_will(
        env: Env,
        owner: Address,
        tokens: Vec<(Address, i128)>,
        beneficiaries: Vec<Beneficiary>,
        checkin_period_days: u64,
        grace_period_days: u64,
        guardians: Vec<Address>,
        guardian_threshold: u32,
        keeper_bounty_bps: Option<u32>,
        confirmation_delay_seconds: u64,
    ) -> u64 {
        owner.require_auth();

        // Validate keeper bounty if provided
        let keeper_bounty = match keeper_bounty_bps {
            Some(bps) if bps <= MAX_KEEPER_BOUNTY_BPS => bps,
            Some(_) => panic_with_error!(&env, WillError::KeeperBountyExceedsMax),
            None => 0,
        };

        if tokens.is_empty() || tokens.len() > MAX_TOKENS {
            panic_with_error!(&env, WillError::InvalidTokenCount);
        }
        // Reject duplicate token addresses up front, before any transfer
        // happens. The rustdoc for this function promises each token address
        // is unique; letting a duplicate through would additionally make the
        // legacy `balance` mirror (derived from the first entry) disagree with
        // the accumulated `balances` map (#350).
        let mut seen_tokens: Vec<Address> = Vec::new(&env);
        for (token_addr, _) in tokens.iter() {
            if seen_tokens.contains(&token_addr) {
                panic_with_error!(&env, WillError::DuplicateToken);
            }
            seen_tokens.push_back(token_addr);
        }
        assert_beneficiary_count(&env, &beneficiaries);
        assert_valid_guardians(&env, &owner, &guardians);
        assert_valid_periods(&env, checkin_period_days, grace_period_days);

        // Validate and normalise the guardian threshold.
        //
        // An empty guardian list disables the guardian mechanism entirely, so
        // the threshold is meaningless there: votes can never be cast, so any
        // stored value — including a large caller-supplied one — describes a
        // quorum that can never be reached *and* never evaluated. The argument
        // is therefore ignored for a guardian-less will and normalised to 0.
        //
        // Normalising (rather than persisting the raw argument) matters
        // beyond the guardian-less will itself: `update_guardians` and
        // `update_will_settings` reject a new non-empty guardian list whose
        // length is below the will's stored `guardian_threshold`, precisely so
        // the threshold can never become unreachable. A will created with no
        // guardians and a stray threshold of, say, 9 would therefore reject
        // the owner's very next ordinary call to add two or three guardians,
        // with a cause invisible from the call being made (#363).
        //
        // For a non-empty list the threshold is meaningful, so it must fall in
        // `1..=guardians.len()` and is stored as supplied.
        let guardian_threshold = if guardians.is_empty() {
            0
        } else {
            let threshold_range = 1..=guardians.len();
            if !threshold_range.contains(&guardian_threshold) {
                panic_with_error!(&env, WillError::InvalidGuardianThreshold);
            }
            guardian_threshold
        };

        // Convert addresses to Guardian structs with weight 1 and build balances.
        let mut guardian_structs: Vec<Guardian> = Vec::new(&env);
        for addr in guardians.iter() {
            guardian_structs.push_back(Guardian {
                address: addr,
                weight: 1,
                consent: GuardianConsent::Pending,
            });
        }

        // Checked before any transfer: a call that was always going to fail
        // MAX_WILLS_PER_INDEX for the owner or a beneficiary should fail on
        // this cheap in-contract check, not after the token transfer below
        // has already succeeded (#260).
        storage::assert_index_capacity(&env, &owner, &beneficiaries);

        // Validate amounts and build the balances map.
        let mut balances: Map<Address, i128> = Map::new(&env);
        for (token_addr, amount) in tokens.iter() {
            if amount <= 0 {
                panic_with_error!(&env, WillError::ZeroAmount);
            }
            // Probe the token interface with a read-only `decimals()` call
            // before attempting any transfer. A non-token address (or any
            // contract that does not implement SEP-41) will fail here with a
            // clear `InvalidToken` error rather than an opaque host-level
            // cross-contract failure deep inside `transfer`.
            if token::Client::new(&env, &token_addr)
                .try_decimals()
                .is_err()
            {
                panic_with_error!(&env, WillError::InvalidToken);
            }
            // Transfer this token from the owner into the contract.
            token::Client::new(&env, &token_addr).transfer(
                &owner,
                &env.current_contract_address(),
                &amount,
            );
            // Duplicates were rejected above, so this is a plain insert; the
            // additive accumulation is kept so the intent stays explicit.
            let prev = balances.get(token_addr.clone()).unwrap_or(0);
            balances.set(token_addr, prev + amount);
        }

        // Fixed amounts are denominated in the will's primary token, which is
        // the first entry of `tokens` (mirrored into `Will::token` below).
        let (primary_token, _) = tokens.get_unchecked(0);
        assert_valid_allocations(
            &env,
            &beneficiaries,
            primary_token_balance(&balances, &primary_token),
        );

        let will_id = storage::next_will_id(&env);
        let now = env.ledger().timestamp();

        let token_count = balances.len();
        for beneficiary in beneficiaries.iter() {
            storage::index_by_beneficiary(&env, &beneficiary.address, will_id);
        }

        // Determine initial status and confirmation deadline (#43).
        let (status, confirmation_deadline) = if confirmation_delay_seconds > 0 {
            (
                WillStatus::PendingConfirmation,
                Some(now + confirmation_delay_seconds),
            )
        } else {
            (WillStatus::Active, None)
        };

        // `token`/`balance` mirror the first locked token for backward
        // compatibility with single-token helpers (merge_wills, split_will,
        // reveal_and_claim); `balances` above is the authoritative
        // multi-token source of truth. Derive the mirror from the accumulated
        // map (rather than re-reading the first `tokens` entry) so `balance`
        // can never disagree with `balances[token]` (#350).
        let (primary_token, _) = tokens.get_unchecked(0);
        let primary_amount = primary_token_balance(&balances, &primary_token);

        let will = Will {
            id: will_id,
            owner: owner.clone(),
            balances,
            token: primary_token,
            is_native: false,
            balance: primary_amount,
            beneficiaries,
            hashed_beneficiaries: Vec::new(&env),
            checkin_period_days,
            grace_period_days,
            last_checkin: now,
            trigger_time: None,
            confirmation_deadline,
            status,
            guardians: guardian_structs,
            guardian_vote_weight: 0,
            guardian_votes: 0,
            guardian_cancel_vote_weight: 0,
            guardian_cancel_votes: 0,
            guardian_threshold,
            guardian_list_updated_at: now,
            schema_version: CURRENT_SCHEMA_VERSION,
            keeper_bounty_bps: keeper_bounty,
            delegate: None,
        };
        storage::save_will(&env, &will);
        storage::index_by_owner(&env, &owner, will_id);
        storage::increment_active_will_count(&env);

        // Increment locked value for each token in this will
        for (token_addr, amount) in tokens.iter() {
            storage::adjust_locked_value(&env, &token_addr, amount);
        }

        // Record the will's real initial status as both endpoints of this
        // self-transition: `Active -> Active` when it starts immediately, and
        // `PendingConfirmation -> PendingConfirmation` when a confirmation
        // delay is in effect. Recording a hardcoded `Active` here would hide
        // the real `PendingConfirmation` state from `get_will_history` (#351).
        record_transition(
            &env,
            will_id,
            status,
            status,
            &owner,
            symbol_short!("create"),
        );

        events::will_created(
            &env,
            will_id,
            &owner,
            token_count,
            &will.beneficiaries,
            now + checkin_period_days * SECONDS_PER_DAY,
        );

        will_id
    }

    // -----------------------------------------------------------------------
    // Issue #43 — confirm_will
    // -----------------------------------------------------------------------

    /// Transitions a will from `PendingConfirmation` to `Active`, starting the
    /// check-in clock. Must be called by the owner within the confirmation window.
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if the will does not exist.
    /// - [`WillError::NotOwner`] if `owner` does not own the will.
    /// - [`WillError::WillNotConfirmed`] if the will is not `PendingConfirmation`.
    /// - [`WillError::ConfirmationWindowExpired`] if the confirmation deadline has passed.
    /// - [`WillError::TooManyBeneficiaries`], [`WillError::InvalidPercentages`],
    ///   [`WillError::DuplicateBeneficiary`] or [`WillError::FixedAmountExceedsBalance`]
    ///   if the stored beneficiary list no longer passes the same validation
    ///   `create_will` applied (issue #448).
    ///
    /// # Re-validation at confirmation time (#493)
    ///
    /// - **Ownership** is checked against the will's *stored* owner on every
    ///   call (`load_owned` plus `owner.require_auth()`), never against
    ///   anything captured at creation. No entry point reassigns an existing
    ///   will's `owner`; if one is ever added, a confirmation by the previous
    ///   owner is rejected with [`WillError::NotOwner`].
    /// - **Beneficiaries** are re-validated as described above. Soroban
    ///   exposes no host function to test whether a Stellar account exists,
    ///   so account existence cannot be verified on-chain. A beneficiary
    ///   that cannot receive a token at release time is instead handled by
    ///   `distribute`'s failed-payout record and
    ///   [`Self::retry_failed_payout`] (#459), so it never aborts the release
    ///   for everyone else.
    pub fn confirm_will(env: Env, will_id: u64, owner: Address) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);

        if will.status != WillStatus::PendingConfirmation {
            panic_with_error!(&env, WillError::WillNotConfirmed);
        }

        let now = env.ledger().timestamp();
        if let Some(deadline) = will.confirmation_deadline {
            if now > deadline {
                panic_with_error!(&env, WillError::ConfirmationWindowExpired);
            }
        }

        // Re-validate the beneficiary list against the will's *current*
        // balances before activating it (#448). The list was validated at
        // `create_will` time, but it may have changed during the confirmation
        // delay. Confirming an invalid list would produce an Active will that
        // can never be released cleanly.
        if will.beneficiaries.is_empty() || will.beneficiaries.len() > MAX_BENEFICIARIES {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }
        assert_valid_allocations(&env, &will.beneficiaries, total_balance(&will.balances));
        assert_valid_percentages(&env, &will.beneficiaries, &will.hashed_beneficiaries);

        will.status = WillStatus::Active;
        will.last_checkin = now;
        will.confirmation_deadline = None;
        storage::save_will(&env, &will);

        // Record the confirmation in the audit trail so `get_will_history`
        // can reconstruct the full lifecycle (#352).
        record_transition(
            &env,
            will_id,
            WillStatus::PendingConfirmation,
            WillStatus::Active,
            &owner,
            symbol_short!("confirm"),
        );

        events::will_confirmed(&env, will_id, &owner);
    }

    // -----------------------------------------------------------------------
    // Core lifecycle
    // -----------------------------------------------------------------------

    /// Resets the check-in countdown for `will_id`. Must be called by the
    /// will's owner or the designated delegate, and the will must be `Active`.
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if no will exists with this id.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::NotOwner`] if `caller` is neither the owner nor the delegate.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Continuing from a will created with create_will …
    /// // Advance time to just before the deadline, then check in.
    /// env.ledger().with_mut(|l| l.timestamp += 80 * 86_400); // 80 days later
    ///
    /// client.check_in(&will_id, &owner);
    ///
    /// // The countdown resets; the will is still Active.
    /// assert_eq!(client.get_will(&will_id).status, WillStatus::Active);
    /// ```
    pub fn check_in(env: Env, will_id: u64, caller: Address) {
        caller.require_auth();
        let mut will = load_will(&env, will_id);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);
        assert_owner_or_delegate(&env, &will, &caller);

        let now = env.ledger().timestamp();
        will.last_checkin = now;
        let next_deadline = now + will.checkin_period_days * SECONDS_PER_DAY;
        storage::save_will(&env, &will);

        // Record the check-in in the same audit trail status transitions use
        // (#500). `status` does not change, so this is a same-status entry
        // (`Active` -> `Active`); what matters is that every event that moves
        // `last_checkin` -- not just the ones that also move `status` -- gets
        // a sequenced entry, so a reader of the trail never sees a status
        // change following a check-in that left no trace of when it happened.
        record_transition(
            &env,
            will_id,
            WillStatus::Active,
            WillStatus::Active,
            &caller,
            symbol_short!("checkin"),
        );

        events::check_in(&env, will_id, &caller, next_deadline);
    }

    /// Sets or replaces the delegate address for `will_id`. Only callable
    /// by the owner while the will is `Active`. Pass `None` to clear.
    pub fn set_delegate(env: Env, will_id: u64, owner: Address, delegate: Option<Address>) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        will.delegate = delegate.clone();
        storage::save_will(&env, &will);

        if let Some(ref addr) = delegate {
            events::delegate_set(&env, will_id, &owner, addr);
        } else {
            events::delegate_cleared(&env, will_id, &owner);
        }
    }

    /// Batch check-in across multiple wills in a single transaction.
    /// All wills must be owned by `owner` and in `Active` status.
    /// Panics if any will ID is invalid, not owned by `owner`, or not `Active`.
    ///
    /// At most `batch_check_in_limit::MAX_BATCH_CHECK_IN` (50) IDs are accepted
    /// per call; longer inputs panic with [`WillError::BatchTooLarge`]. Every ID
    /// must also be distinct — a repeated ID panics with
    /// [`WillError::DuplicateWillId`] rather than being processed once per
    /// occurrence, which would otherwise rewrite the same will and emit a
    /// redundant `check_in` event for each repeat (#355).
    ///
    /// # Panics
    /// - [`WillError::BatchTooLarge`] if more than
    ///   `batch_check_in_limit::MAX_BATCH_CHECK_IN` ids are supplied.
    /// - [`WillError::DuplicateWillId`] if the same will id appears more than once.
    /// - [`WillError::WillNotFound`] if any id names no will.
    /// - [`WillError::NotOwner`] if any of those wills is not owned by `owner`.
    /// - [`WillError::WillNotActive`] if any of those wills is not `Active`.
    pub fn batch_check_in(env: Env, will_ids: Vec<u64>, owner: Address) {
        owner.require_auth();
        batch_check_in_limit::assert_within_limit(&env, will_ids.len());
        // Reject repeats before any storage write, so a batch can never be
        // partially applied and the `batch_checkin` count always matches the
        // number of distinct wills actually checked in (#355).
        batch_check_in_limit::assert_no_duplicates(&env, &will_ids);
        let now = env.ledger().timestamp();
        let count = will_ids.len();

        for will_id in will_ids.iter() {
            let mut will = load_owned(&env, will_id, &owner);
            assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

            will.last_checkin = now;
            let next_deadline = now + will.checkin_period_days * SECONDS_PER_DAY;
            storage::save_will(&env, &will);

            // Same per-will audit entry as the single-will `check_in` path,
            // so a batched check-in is indistinguishable in the trail from
            // one done individually (#500).
            record_transition(
                &env,
                will_id,
                WillStatus::Active,
                WillStatus::Active,
                &owner,
                symbol_short!("checkin"),
            );

            events::check_in(&env, will_id, &owner, next_deadline);
        }

        events::batch_checkin(&env, &owner, &will_ids, count);
    }

    /// Starts the grace period for `will_id` once the check-in deadline has
    /// passed. Callable by anyone.
    ///
    /// The grace period is anchored to the missed check-in deadline
    /// (`last_checkin + checkin_period_days`), which is stored in
    /// `trigger_time` — not to the ledger time at which this function happens
    /// to be called (#457). Release therefore becomes available at exactly
    /// `last_checkin + checkin_period_days + grace_period_days`, however late
    /// the trigger is submitted. If the trigger arrives after that point the
    /// will is immediately releasable.
    ///
    /// # Panics
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::CheckinNotDue`] if the check-in deadline has not passed yet.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Continuing from a will created with a 90-day check-in period …
    /// // Advance past the check-in deadline without calling check_in.
    /// env.ledger().with_mut(|l| l.timestamp += 91 * 86_400); // 91 days later
    ///
    /// // Anyone can call trigger_will once the deadline is missed.
    /// client.trigger_will(&will_id);
    ///
    /// assert_eq!(client.get_will(&will_id).status, WillStatus::Triggered);
    /// ```
    pub fn trigger_will(env: Env, will_id: u64) {
        let mut will = load_will(&env, will_id);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        let now = env.ledger().timestamp();
        let deadline = will.last_checkin + will.checkin_period_days * SECONDS_PER_DAY;
        if now < deadline {
            panic_with_error!(&env, WillError::CheckinNotDue);
        }

        // Anchor the grace period to the missed check-in deadline, not to
        // whenever a keeper happened to call `trigger_will` (#457). This makes
        // the release time a pure function of the owner's last check-in, so a
        // delayed trigger can neither postpone nor accelerate the release.
        will.status = WillStatus::Triggered;
        will.trigger_time = Some(deadline);
        let grace_period_ends = grace_deadline(&will);
        storage::save_will(&env, &will);

        storage::index_triggered_will(&env, will_id);

        record_transition(
            &env,
            will_id,
            WillStatus::Active,
            WillStatus::Triggered,
            &env.current_contract_address(),
            symbol_short!("trigger"),
        );

        events::will_triggered(&env, will_id, grace_period_ends);
    }

    /// Cancels an in-progress trigger during the grace period, proving the
    /// owner is alive, and resets the check-in countdown.
    ///
    /// The grace deadline second belongs to the owner: this call is valid while
    /// `now <= trigger_time + grace_period_days * SECONDS_PER_DAY` and panics
    /// with [`WillError::GracePeriodExpired`] strictly after it. The rule in
    /// [`Self::release_inheritance`] is the exact complement of this one, so
    /// precisely one of the two succeeds at every timestamp (#354).
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodExpired`] if the grace period has already
    ///   elapsed, i.e. `now` is strictly greater than the grace deadline.
    pub fn emergency_checkin(env: Env, will_id: u64, owner: Address) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(
            &env,
            &will,
            WillStatus::Triggered,
            WillError::WillNotTriggered,
        );

        // #751 — this used to also bind a local `grace_deadline` via
        // `grace_period_end(&env, &will)`, which shadowed the `grace_deadline`
        // function below and made `grace_deadline(&will)` a call on a `u64`
        // (a compile error). `grace_deadline()` is the one shared helper every
        // other grace-period check (release_inheritance, reveal_and_claim,
        // get_time_until_deadline) already goes through, per its own doc
        // comment, so this now does too; the removed call added no coverage
        // `assert_status(.., Triggered, ..)` above doesn't already guarantee
        // (a `Triggered` will always has `trigger_time` set).
        let now = env.ledger().timestamp();
        if now > grace_deadline(&will) {
            panic_with_error!(&env, WillError::GracePeriodExpired);
        }

        // Clear the vote markers before zeroing the counter: the counter is
        // what tells `reset_guardian_votes` whether there is anything to clear.
        storage::reset_guardian_votes(&env, &will);
        storage::reset_guardian_cancel_votes(&env, &will);

        will.status = WillStatus::Active;
        will.trigger_time = None;
        will.last_checkin = now;
        will.guardian_vote_weight = 0;
        will.guardian_votes = 0;
        will.guardian_cancel_vote_weight = 0;
        will.guardian_cancel_votes = 0;
        storage::save_will(&env, &will);

        storage::unindex_triggered_will(&env, will_id);

        record_transition(
            &env,
            will_id,
            WillStatus::Triggered,
            WillStatus::Active,
            &owner,
            symbol_short!("emerg"),
        );

        let next_deadline = now + will.checkin_period_days * SECONDS_PER_DAY;
        events::emergency_checkin(&env, will_id, &owner, next_deadline);
    }

    /// Distributes all token balances to beneficiaries proportionally to
    /// their configured percentages. Callable by anyone once the grace
    /// period has fully elapsed.
    ///
    /// The grace deadline second itself is **not** releasable: this call
    /// requires `now` to be strictly greater than
    /// `trigger_time + grace_period_days * SECONDS_PER_DAY` and panics with
    /// [`WillError::GracePeriodNotExpired`] at or before it. The boundary second
    /// therefore belongs to the owner's [`Self::emergency_checkin`], which
    /// accepts `now <= deadline`. Exactly one of the two succeeds at any
    /// timestamp, so the outcome no longer depends on which transaction the
    /// ledger happens to order first (#354).
    ///
    /// Tokens are transferred directly to each beneficiary ("push" mode);
    /// there is no pull mode and no `claim_share` entry point. Whatever the
    /// visible beneficiaries' allocations do not claim — a will made up only
    /// of `Allocation::FixedAmount` entries, or the share a renounced
    /// percentage beneficiary leaves behind (#362) — is refunded to the owner
    /// rather than stranded in the contract (#383).
    ///
    /// Splits are computed from `will.beneficiaries` as it stands at the
    /// moment this call executes, not as it stood when [`trigger_will`] ran —
    /// see [`renounce_beneficiary`]'s docs for the full interaction with an
    /// in-progress grace period.
    ///
    /// # Atomicity and re-validation (#490)
    ///
    /// The whole call runs as one atomic Soroban invocation: no other
    /// transaction can trigger, check in, or otherwise change this will
    /// between the status check below and the transfers in `distribute`, so
    /// the will is effectively locked for the duration of the call. As defense
    /// in depth, the will is nevertheless re-read from storage and its status
    /// re-validated immediately before any funds move.
    ///
    /// # Bounded work (#491)
    ///
    /// A will names at most [`MAX_BENEFICIARIES`] beneficiaries and holds at
    /// most `MAX_TOKENS` tokens, so one release performs at most
    /// [`MAX_RELEASE_PAYOUTS`] beneficiary transfers. Both caps are enforced
    /// whenever the lists are set and re-checked here. If a release ever did
    /// exceed the transaction budget, the atomic invocation would roll back
    /// entirely rather than leave beneficiaries half-paid.
    ///
    /// # Panics
    /// - [`WillError::TooManyBeneficiaries`] / [`WillError::InvalidTokenCount`]
    ///   if the will names more than [`MAX_BENEFICIARIES`] beneficiaries or
    ///   holds more than `MAX_TOKENS` tokens (#491).
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`, or is
    ///   `Triggered` without a recorded `trigger_time`.
    /// - [`WillError::GracePeriodNotExpired`] if
    ///   `env.ledger().timestamp() < trigger_time + grace_period_days` — the
    ///   grace period deadline has not passed yet.
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodNotExpired`] if `now` is at or before the grace
    ///   deadline, i.e. the grace period has not strictly elapsed yet.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Continuing from a triggered will with a 7-day grace period …
    /// // Advance past the grace period without calling emergency_checkin.
    /// env.ledger().with_mut(|l| l.timestamp += 8 * 86_400); // 8 days after trigger
    ///
    /// // Anyone can release once the grace period has fully elapsed.
    /// client.release_inheritance(&will_id, &None);
    ///
    /// // Funds have been distributed; the will is now Released.
    /// assert_eq!(client.get_will(&will_id).status, WillStatus::Released);
    /// // Each beneficiary's token balance reflects their basis-point share.
    /// ```
    pub fn release_inheritance(env: Env, will_id: u64, caller: Option<Address>) {
        let mut will = load_will(&env, will_id);
        assert_status(
            &env,
            &will,
            WillStatus::Triggered,
            WillError::WillNotTriggered,
        );

        // Timing invariant (#442): funds may only leave via this path once
        // the grace period has fully elapsed, i.e. `now >= grace_period_end`.
        // Strictly greater than is rejected: the boundary second belongs to
        // the owner's `emergency_checkin`, so exactly one of these two entry
        // points is valid at any timestamp and the outcome cannot depend on
        // transaction order within a ledger (#354). `emergency_checkin` and
        // `guardian_cancel_trigger` enforce the same deadline from the other
        // side, so all three agree on when the grace period is over.
        //
        // #751 — this used to also bind its own local `grace_deadline` via
        // `grace_period_end(&env, &will)` and re-check `now <= grace_deadline`
        // nested inside this same condition, which both shadowed the
        // `grace_deadline` function used in the outer check and -- because
        // the outer `if` was never actually closed -- silently made this
        // function's entire body (the state transition and the `distribute`
        // call below) conditional on the grace period *not yet* having
        // expired, the exact opposite of this function's purpose.
        let now = env.ledger().timestamp();
        if now < grace_deadline(&will) {
            panic_with_error!(&env, WillError::GracePeriodNotExpired);
        }

        record_transition(
            &env,
            will_id,
            WillStatus::Triggered,
            WillStatus::Released,
            &env.current_contract_address(),
            symbol_short!("release"),
        );

        // Release the protocol's locked-value total for *every* token the will
        // held, not just the primary-token mirror. `distribute` pays the
        // balances out to beneficiaries, so without this the totals stay
        // inflated at their pre-release value forever — the same class of bug
        // #353 fixed for `cancel_will`, which was the only other terminal path
        // that moved value out of a will.
        //
        // `cancel_will` does the mirror of this: it decrements before mutating
        // the will, keeping the changes-then-interactions ordering, because
        // `distribute` performs the token transfers below.
        for (token_addr, balance) in will.balances.iter() {
            if balance > 0 {
                storage::adjust_locked_value(&env, &token_addr, -balance);
            }
        }

        // Re-validate status and size immediately before any funds move
        // (#490, #491). See `assert_release_preconditions`.
        assert_release_preconditions(&env, will_id, &will);

        distribute(&env, &mut will, &caller);
    }

    /// Delivers a payout that failed during a previous `distribute` call
    /// (#459) — a beneficiary share, an owner refund, or a keeper bounty that
    /// could not be transferred at the time, e.g. because the token was
    /// paused or the recipient was frozen/unauthorized.
    ///
    /// Permissionless, like triggering a release: anyone may call this for
    /// anyone, since it can only deliver funds to the exact recipient
    /// `distribute` already computed and reserved for them, not redirect them
    /// anywhere else. Deliberately does not load or validate the parent will
    /// at all -- the recorded amount is self-sufficient -- so this keeps
    /// working even after the will has been archived.
    ///
    /// # Panics
    /// - [`WillError::NoFailedPayout`] if no failed payout is recorded for
    ///   this exact `(will_id, token, recipient)` tuple (never failed,
    ///   already retried successfully, or the arguments don't match).
    /// - Propagates the token contract's own panic if this attempt also
    ///   fails; the record is left in place for a future retry.
    pub fn retry_failed_payout(env: Env, will_id: u64, token: Address, recipient: Address) {
        let amount = match storage::get_failed_payout(&env, will_id, &token, &recipient) {
            Some(amount) => amount,
            None => panic_with_error!(&env, WillError::NoFailedPayout),
        };

        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        storage::remove_failed_payout(&env, will_id, &token, &recipient);
        events::payout_retried(&env, will_id, &token, &recipient, amount);
    }

    /// Cancels the will and refunds every locked token balance to the owner.
    /// Only possible while the will is `Active`, i.e. before it has ever
    /// been triggered by a missed check-in (an owner who is mid-grace-period
    /// must first call `emergency_checkin` to return the will to `Active`).
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is neither `Active` nor
    ///   `PendingConfirmation`.
    pub fn cancel_will(env: Env, will_id: u64, owner: Address) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);

        // Allow cancellation from both Active and PendingConfirmation (#43).
        if will.status != WillStatus::Active && will.status != WillStatus::PendingConfirmation {
            panic_with_error!(&env, WillError::WillNotActive);
        }

        // Snapshot the balances before mutating state (checks-effects-interactions).
        let contract_address = env.current_contract_address();
        let token_count = will.balances.len();
        // Capture balances for transfer after state is committed.
        let balances_snapshot = will.balances.clone();
        // `cancel_will` accepts both `Active` and `PendingConfirmation`, so the
        // audit trail must record whichever state the will actually cancelled
        // from rather than a hardcoded `Active` (#351).
        let prior_status = will.status;

        // --- EFFECTS: mutate state and persist before any external calls ---
        storage::decrement_active_will_count(&env);
        storage::adjust_locked_for_balances(&env, &balances_snapshot, -1);
        // Decrement the protocol locked-value total for *every* token the will
        // held, not just the primary-token mirror, so a multi-token
        // cancellation does not leave the other tokens' totals permanently
        // inflated (#353).
        for (token_addr, balance) in balances_snapshot.iter() {
            if balance > 0 {
                storage::adjust_locked_value(&env, &token_addr, -balance);
            }
        }

        will.balance = 0;
        will.balances = Map::new(&env);
        will.status = WillStatus::Cancelled;

        // Prune stale index entries (#70): remove the will from the owner
        // index and from every beneficiary's reverse index so that
        // get_wills_by_owner / get_wills_by_beneficiary no longer return it.
        storage::remove_owner_index(&env, &owner, will_id);
        for beneficiary in will.beneficiaries.iter() {
            storage::remove_beneficiary_index(&env, &beneficiary.address, will_id);
        }

        storage::save_will(&env, &will);

        record_transition(
            &env,
            will_id,
            prior_status,
            WillStatus::Cancelled,
            &owner,
            symbol_short!("cancel"),
        );

        // --- INTERACTIONS: external token transfers happen after state is settled ---
        for (token_addr, balance) in balances_snapshot.iter() {
            if balance > 0 {
                token::Client::new(&env, &token_addr).transfer(&contract_address, &owner, &balance);
            }
        }

        events::will_cancelled(&env, will_id, &owner, token_count);
    }

    /// Explicitly marks a `Released` will as `Settled`, completing the
    /// archival step separate from the payout moment. Only the owner may
    /// close a will, and only after it has been released.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotReleased`] if the will is not `Released`.
    pub fn close_will(env: Env, will_id: u64, owner: Address) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(
            &env,
            &will,
            WillStatus::Released,
            WillError::WillNotReleased,
        );

        will.status = WillStatus::Settled;
        storage::save_will(&env, &will);

        // Record the Released -> Settled transition so the audit trail from
        // `get_will_history` has no gaps (#352).
        record_transition(
            &env,
            will_id,
            WillStatus::Released,
            WillStatus::Settled,
            &owner,
            symbol_short!("close"),
        );

        events::will_closed(&env, will_id, &owner);
    }

    /// Replaces the beneficiary list for `will_id`. Only possible while the
    /// will is `Active`. The new basis points must sum to exactly 10,000.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::TooManyBeneficiaries`] if the new list is empty or too large.
    /// - [`WillError::InvalidPercentages`] if the new basis points do not sum to 10,000.
    /// - [`WillError::DuplicateBeneficiary`] if the same address is supplied twice.
    pub fn update_beneficiaries(
        env: Env,
        will_id: u64,
        owner: Address,
        beneficiaries: Vec<Beneficiary>,
    ) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        assert_beneficiary_count(&env, &beneficiaries);
        assert_valid_allocations(&env, &beneficiaries, total_balance(&will.balances));
        if beneficiaries.is_empty() || beneficiaries.len() > MAX_BENEFICIARIES {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }
        assert_valid_allocations(
            &env,
            &beneficiaries,
            primary_token_balance(&will.balances, &will.token),
        );

        // Only addresses that actually join or leave the will need their
        // reverse index touched. Unconditionally removing every old address
        // and re-adding every new one costs a storage read and write per
        // address even when the lists are identical — which is the common
        // case, since most updates only re-cut the percentages. Membership is
        // decided against the two lists already in memory, at no storage cost.
        for old in will.beneficiaries.iter() {
            if !names_address(&beneficiaries, &old.address) {
                storage::remove_beneficiary_index(&env, &old.address, will_id);
            }
        }
        for new_beneficiary in beneficiaries.iter() {
            if !names_address(&will.beneficiaries, &new_beneficiary.address) {
                storage::index_by_beneficiary(&env, &new_beneficiary.address, will_id);
            }
        }

        will.beneficiaries = beneficiaries;
        storage::save_will(&env, &will);

        events::beneficiaries_updated(
            &env,
            will_id,
            &owner,
            will.beneficiaries.len(),
            &will.beneficiaries,
        );
    }

    /// Allows a beneficiary to renounce their inheritance share in advance.
    /// The renouncing beneficiary is removed from the beneficiary list, and
    /// their percentage is redistributed proportionally among the remaining
    /// **percentage-based** beneficiaries.
    ///
    /// # Where the renounced share goes
    ///
    /// A `Percentage` share can only be redistributed to another
    /// `Percentage` entry, so what happens to it depends on who is left:
    ///
    /// - **At least one `Percentage` beneficiary remains** — the renounced
    ///   basis points are redistributed across them in proportion to their
    ///   current shares, with the last one absorbing the rounding remainder so
    ///   the total stays exactly 10,000 bps.
    /// - **No `Percentage` beneficiary remains** (a will that mixed one
    ///   `Percentage` entry with `FixedAmount` entries, where the percentage
    ///   holder is the one renouncing) — the renunciation is still accepted,
    ///   and the share is **returned to the owner** at release rather than
    ///   reassigned (#362). `assert_valid_allocations` deliberately allows a
    ///   `FixedAmount`-only list to leave headroom, and `distribute` refunds
    ///   that headroom to the owner once no percentage beneficiary is left to
    ///   absorb it (#383); the same path covers a share reserved for a hashed
    ///   beneficiary added later (#181/#186). The remaining `FixedAmount`
    ///   beneficiaries are paid exactly what they are entitled to and are never
    ///   over-paid.
    ///
    /// A renounced `FixedAmount` share needs no redistribution at all:
    /// `distribute` computes fixed payouts from the beneficiary list as it
    /// stands at release, so removing the entry simply leaves that amount to
    /// the remaining percentage beneficiaries.
    ///
    /// Only callable by a named beneficiary while the will is in `Active` or
    /// `Triggered` status. After renunciation, the will is saved but status
    /// transitions are not recorded (it's a beneficiary action, not a status change).
    ///
    /// # Interaction with an in-progress `Triggered` grace period
    ///
    /// [`trigger_will`] does not snapshot the beneficiary list: it only flips
    /// `status` to `Triggered` and records `trigger_time`. `will.beneficiaries`
    /// therefore stays live storage for the entire grace period, and
    /// [`release_inheritance`] reads it fresh at release time rather than
    /// reading whatever the list looked like at the moment of triggering.
    /// This is intentional — it is what makes it possible for a beneficiary
    /// to renounce *after* a will has been triggered (during the grace
    /// period) and still have the payout split adjust for them — but it also
    /// means the effective split is not finalized until
    /// [`release_inheritance`] actually runs. Any renunciation submitted
    /// before that call, including one made moments before release, changes
    /// every remaining beneficiary's share immediately and irreversibly:
    /// there is no separate confirmation step and no snapshot to roll back
    /// to. **Renunciation is final once the `beneficiary_renounced` event is
    /// emitted** — the contract offers no entry point that undoes it.
    ///
    /// To make the effective split reconstructable without cross-referencing
    /// timestamps, the event carries the full redistributed beneficiary list
    /// and the will's `trigger_time`: `Some(t)` identifies the grace cycle
    /// the renunciation applies to (the one opened by the trigger at `t`),
    /// and `None` means the will was still `Active` (#487).
    ///
    /// # Parameters
    /// - `will_id`: the will to renounce beneficiary status from
    /// - `beneficiary`: the address renouncing their share; must authorize this call
    ///   and must be named as a beneficiary in the will
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if the will does not exist.
    /// - [`WillError::BeneficiaryNotFound`] if `beneficiary` is not named in the will.
    /// - [`WillError::WillNotActive`] if the will is not in `Active` or `Triggered` status.
    /// - [`WillError::TooManyBeneficiaries`] if `beneficiary` is the will's only
    ///   beneficiary. A will must always name between 1 and
    ///   [`MAX_BENEFICIARIES`] beneficiaries — the same bound `create_will` and
    ///   `update_beneficiaries` enforce — so the last remaining beneficiary
    ///   cannot renounce; a will with no beneficiaries would leave
    ///   `release_inheritance` with nobody to pay and its balance stranded. The
    ///   owner can still replace the beneficiary through
    ///   `update_beneficiaries` (#361).
    /// - [`WillError::InvalidPercentages`] / [`WillError::FixedAmountExceedsBalance`]
    ///   if the redistributed allocations fail validation (defensive: the
    ///   redistribution keeps every percentage share summing to 10,000 bps).
    pub fn renounce_beneficiary(env: Env, will_id: u64, beneficiary: Address) {
        beneficiary.require_auth();
        let mut will = load_will(&env, will_id);

        // Allow renunciation only in Active or Triggered status
        if will.status != WillStatus::Active && will.status != WillStatus::Triggered {
            panic_with_error!(&env, WillError::WillNotActive);
        }

        // Find and remove the renouncing beneficiary
        let mut found_index: Option<usize> = None;
        let mut renounced_allocation: Option<Allocation> = None;
        for (index, b) in will.beneficiaries.iter().enumerate() {
            if b.address == beneficiary {
                found_index = Some(index);
                renounced_allocation = Some(b.allocation);
                break;
            }
        }

        let index = match found_index {
            Some(i) => i,
            None => panic_with_error!(&env, WillError::BeneficiaryNotFound),
        };

        // Create new beneficiary list without the renouncing beneficiary
        let mut new_beneficiaries: Vec<Beneficiary> = Vec::new(&env);
        for (i, b) in will.beneficiaries.iter().enumerate() {
            if i != index {
                new_beneficiaries.push_back(b);
            }
        }

        // A will must always keep at least one beneficiary: `create_will` and
        // `update_beneficiaries` both require 1..=MAX_BENEFICIARIES, and a will
        // with none would leave `release_inheritance` with nobody to pay, so
        // the balance could never be distributed and would stay locked in the
        // contract (#361). `assert_valid_allocations` below cannot catch this —
        // an empty list trivially satisfies every check it makes.
        if new_beneficiaries.is_empty() {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }

        // A renounced `FixedAmount` share needs no redistribution: `distribute`
        // computes fixed payouts dynamically from the current beneficiary
        // list, so simply removing the entry leaves more of the balance for
        // percentage-based beneficiaries automatically. Only a renounced
        // `Percentage` share needs its basis points redistributed explicitly.
        let renounced_basis_points = match renounced_allocation {
            Some(Allocation::Percentage(bp)) => bp,
            _ => 0,
        };

        if renounced_basis_points > 0 {
            // Redistribute the renounced basis points proportionally across
            // the remaining percentage-based beneficiaries.
            let mut remaining_basis_points: u32 = 0;
            for b in new_beneficiaries.iter() {
                if let Allocation::Percentage(bp) = b.allocation {
                    remaining_basis_points = remaining_basis_points.saturating_add(bp);
                }
            }

            if remaining_basis_points > 0 {
                // #486: the redistributed shares must add up to exactly the
                // percentage total held before the renunciation, and never to
                // more than 10_000 bps. Every non-last percentage entry takes
                // the floor of its proportional share, so the floors can only
                // under-shoot; the last percentage entry then takes whatever is
                // left of this capped target. It therefore absorbs the rounding
                // remainder as a non-negative top-up (overage, never underage),
                // however unevenly the renounced share divides — e.g. a prime
                // number of basis points.
                let target_total: u32 = remaining_basis_points
                    .saturating_add(renounced_basis_points)
                    .min(10_000);

                let mut percentage_total: u32 = 0;
                for b in new_beneficiaries.iter() {
                    if let Allocation::Percentage(_) = b.allocation {
                        percentage_total += 1;
                    }
                }

                let mut updated_beneficiaries: Vec<Beneficiary> = Vec::new(&env);
                let mut percentage_seen: u32 = 0;
                let mut running_total: u32 = 0;
                for beneficiary_entry in new_beneficiaries.iter() {
                    match beneficiary_entry.allocation {
                        Allocation::Percentage(bp) => {
                            percentage_seen += 1;
                            let new_bp = if percentage_seen == percentage_total {
                                // Last percentage beneficiary: the rest of the
                                // capped target. `checked_sub` reports an
                                // over-allocation by the earlier entries as a
                                // typed error instead of an arithmetic trap.
                                match target_total.checked_sub(running_total) {
                                    Some(rest) => rest,
                                    None => {
                                        panic_with_error!(&env, WillError::InvalidPercentages)
                                    }
                                }
                            } else {
                                // floor(renounced * bp / remaining), in u128 so
                                // the product cannot overflow.
                                let portion = (renounced_basis_points as u128 * bp as u128)
                                    / remaining_basis_points as u128;
                                bp.saturating_add(portion as u32)
                            };
                            running_total = match running_total.checked_add(new_bp) {
                                Some(sum) => sum,
                                None => panic_with_error!(&env, WillError::InvalidPercentages),
                            };
                            updated_beneficiaries.push_back(Beneficiary {
                                address: beneficiary_entry.address.clone(),
                                allocation: Allocation::Percentage(new_bp),
                            });
                        }
                        fixed => {
                            updated_beneficiaries.push_back(Beneficiary {
                                address: beneficiary_entry.address.clone(),
                                allocation: fixed,
                            });
                        }
                    }
                }

                // Hard cap, independent of `assert_valid_allocations` below.
                if running_total > 10_000 {
                    panic_with_error!(&env, WillError::InvalidPercentages);
                }

                will.beneficiaries = updated_beneficiaries;
            } else {
                // No percentage beneficiary is left to absorb the renounced
                // share (e.g. the will mixed one `Percentage` entry with
                // `FixedAmount` entries and that percentage beneficiary was
                // the one renouncing).
                //
                // The shortened list is accepted as-is rather than rejected or
                // rewritten (#362). `assert_valid_allocations` below permits a
                // `FixedAmount`-only list that leaves headroom, so the
                // renounced basis points are simply no longer claimed by any
                // visible beneficiary — the corresponding balance stays in the
                // will and is returned to the **owner** at release by
                // `distribute`'s refund path (#383), or reserved for a hashed
                // beneficiary added later (#181/#186) if one ever is. Nothing
                // is stranded in the contract and no remaining beneficiary is
                // over-paid; the owner is the only party who loses value
                // relative to a will where the share had been redistributed.
                //
                // Rejecting instead would make a beneficiary's own irrevocable
                // renunciation fail for a reason invisible in the call, and
                // folding it into a `FixedAmount` entry would silently inflate
                // a fixed claim that its holder agreed to in a different token
                // amount. Returning it to the owner keeps the will's accounting
                // honest and matches what a `FixedAmount`-only will already
                // does with unclaimed headroom.
                will.beneficiaries = new_beneficiaries;
            }
        } else {
            // The renounced share was a fixed amount, so there are no basis
            // points to redistribute: `distribute` recomputes fixed payouts
            // from the shortened list, and the freed balance simply goes to
            // the remaining percentage beneficiaries.
            will.beneficiaries = new_beneficiaries;
        }

        // Update indexes: remove the renouncing beneficiary from the reverse index
        storage::remove_beneficiary_index(&env, &beneficiary, will_id);

        // Validate the redistributed beneficiary allocations
        assert_valid_allocations(
            &env,
            &will.beneficiaries,
            primary_token_balance(&will.balances, &will.token),
        );

        // Get the owner for event emission (not changed)
        let owner = will.owner.clone();
        storage::save_will(&env, &will);

        events::beneficiary_renounced(
            &env,
            will_id,
            &beneficiary,
            &owner,
            &will.beneficiaries,
            will.trigger_time,
        );
    }

    /// Replaces the guardian list for `will_id`. Only possible while the will
    /// is `Active`. Any votes cast against the previous guardian list are
    /// cleared so every updated list starts a fresh voting cycle. Consent
    /// entries for the old guardians are also cleared.
    ///
    /// Records the current timestamp as `guardian_list_updated_at` so that
    /// [`guardian_trigger`] enforces a cooldown before the new list takes
    /// effect (see [`GUARDIAN_COOLDOWN_DAYS`]).
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::TooManyBeneficiaries`] if more than `MAX_GUARDIANS`
    ///   guardians are supplied.
    /// - [`WillError::InvalidGuardianThreshold`] if the new guardian list is
    ///   non-empty and the will's current `guardian_threshold` exceeds the new
    ///   list length (i.e. the threshold would become permanently unreachable).
    ///   This entry point cannot change the threshold itself; the owner must
    ///   call [`Self::update_guardians_weighted`], whose optional
    ///   `guardian_threshold` argument sets it, to pick a threshold that suits
    ///   the new list — either before or after shrinking it.
    /// - [`WillError::GuardianCancelInProgress`] if guardian-cancel votes are
    ///   still recorded against the current guardian list (#488).
    pub fn update_guardians(env: Env, will_id: u64, owner: Address, guardians: Vec<Address>) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        assert_valid_guardians(&env, &owner, &guardians);
        assert_no_guardian_cancel_in_flight(&env, &will);

        // Reject any update that would leave the existing threshold unreachable.
        // An empty guardian list disables the guardian mechanism entirely, so
        // threshold is irrelevant there. For a non-empty list the threshold must
        // remain in 1..=new_len — the same invariant enforced at create_will time.
        if !guardians.is_empty() {
            let new_len = guardians.len();
            if will.guardian_threshold > new_len {
                panic_with_error!(&env, WillError::InvalidGuardianThreshold);
            }
        }

        let now = env.ledger().timestamp();
        storage::reset_guardian_votes(&env, &will);
        storage::reset_guardian_cancel_votes(&env, &will);
        let mut guardian_structs: Vec<Guardian> = Vec::new(&env);
        for addr in guardians.iter() {
            guardian_structs.push_back(Guardian {
                address: addr,
                weight: 1,
                consent: GuardianConsent::Pending,
            });
        }
        will.guardians = guardian_structs;
        will.guardian_votes = 0;
        will.guardian_cancel_vote_weight = 0;
        will.guardian_cancel_votes = 0;
        will.guardian_list_updated_at = now;
        will.guardian_vote_weight = 0;
        storage::save_will(&env, &will);

        events::guardians_updated(&env, will_id, &owner, &will.guardians);
    }

    /// Updates the guardian list with custom per-guardian vote weights.
    /// Only callable by the owner while the will is `Active`.
    ///
    /// # Parameters
    /// - `will_id`: the will to update
    /// - `owner`: the will's owner; must authorize this call
    /// - `guardians`: list of `GuardianSpec` entries containing address and weight
    /// - `guardian_threshold`: optional threshold required for quorum
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::InvalidGuardianThreshold`] if a guardian's weight exceeds
    ///   [`MAX_GUARDIAN_WEIGHT`], if the weights sum to more than `u32::MAX`, or
    ///   if the resulting threshold is not in `1..=total_weight`. The sum is
    ///   accumulated with [`u32::checked_add`], so an unrepresentable total is
    ///   reported as a typed error rather than an arithmetic trap (#356).
    ///   Also raised if any guardian's `weight` is `0`: zero weights are
    ///   rejected rather than silently normalised to `1`, so the threshold is
    ///   always validated against the weights actually stored (#489).
    /// - [`WillError::GuardianCancelInProgress`] if guardian-cancel votes are
    ///   still recorded against the current guardian list (#488).
    pub fn update_guardians_weighted(
        env: Env,
        will_id: u64,
        owner: Address,
        guardians: Vec<GuardianSpec>,
        guardian_threshold: Option<u32>,
    ) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        let mut addrs: Vec<Address> = Vec::new(&env);
        for g in guardians.iter() {
            addrs.push_back(g.address.clone());
        }
        assert_valid_guardians(&env, &owner, &addrs);
        assert_no_guardian_cancel_in_flight(&env, &will);

        let threshold = guardian_threshold.unwrap_or(will.guardian_threshold);
        if !guardians.is_empty() {
            // Quorum in guardian_trigger/guardian_cancel is checked against
            // accumulated vote *weight*, not vote count, so the threshold must
            // be validated against the guardians' total weight rather than
            // their count.
            //
            // Weights are caller-supplied, so the sum is accumulated with
            // `checked_add` rather than `sum()`: a few large weights would
            // otherwise overflow the `u32` total and, with the release
            // profile's `overflow-checks` on, abort the whole call with an
            // opaque arithmetic trap instead of a typed error the caller can
            // act on (#356). The per-guardian `MAX_GUARDIAN_WEIGHT` cap is
            // enforced in the same loop and keeps the sum well inside `u32`;
            // `checked_add` is kept as the guarantee that no future change to
            // that cap can reintroduce a trap here.
            let mut total_weight: u32 = 0;
            for g in guardians.iter() {
                // A weight of zero is rejected outright rather than silently
                // promoted: a caller passing a zero-weight guardian expecting
                // it to be excluded from quorum would otherwise see it
                // promoted to weight 1, skewing total_weight and the
                // resulting threshold range in a way they did not intend (#489).
                if g.weight == 0 {
                    panic_with_error!(&env, WillError::InvalidGuardianThreshold);
                }
                if g.weight > MAX_GUARDIAN_WEIGHT {
                    panic_with_error!(&env, WillError::InvalidGuardianThreshold);
                }
                match total_weight.checked_add(g.weight) {
                    Some(sum) => total_weight = sum,
                    None => panic_with_error!(&env, WillError::InvalidGuardianThreshold),
                }
            }
            let threshold_range = 1..=total_weight;
            if !threshold_range.contains(&threshold) {
                panic_with_error!(&env, WillError::InvalidGuardianThreshold);
            }
        }

        let now = env.ledger().timestamp();
        storage::reset_guardian_votes(&env, &will);
        storage::reset_guardian_cancel_votes(&env, &will);
        let mut guardian_structs: Vec<Guardian> = Vec::new(&env);
        for g in guardians.iter() {
            // g.weight is already validated non-zero above when the list is
            // non-empty; when empty this loop never runs.
            guardian_structs.push_back(Guardian {
                address: g.address,
                weight: g.weight,
                consent: GuardianConsent::Pending,
            });
        }
        will.guardians = guardian_structs;
        will.guardian_threshold = threshold;
        will.guardian_votes = 0;
        will.guardian_vote_weight = 0;
        will.guardian_cancel_votes = 0;
        will.guardian_cancel_vote_weight = 0;
        will.guardian_list_updated_at = now;
        storage::save_will(&env, &will);

        events::guardians_updated(&env, will_id, &owner, &will.guardians);
    }

    /// Updates the check-in and/or grace period for an active will.
    /// Only callable by the owner while the will is `Active`.
    ///
    /// # Parameters
    /// - `will_id`: the will to update
    /// - `owner`: the will's owner; must authorize this call
    /// - `checkin_period_days`: new check-in period (optional); if specified,
    ///   must be 1 to `MAX_PERIOD_DAYS`
    /// - `grace_period_days`: new grace period (optional); if specified,
    ///   must be 1 to `MAX_PERIOD_DAYS`
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::InvalidPeriod`] if either period is zero or exceeds
    ///   [`MAX_PERIOD_DAYS`].
    ///
    /// # Events
    /// Emits [`events::periods_updated`] whose `next_deadline` field is the
    /// deadline [`Self::trigger_will`] actually enforces, namely
    /// `last_checkin + checkin_period_days * SECONDS_PER_DAY`. This function
    /// never touches `last_checkin`, so that value is independent of when the
    /// owner happens to call: an indexer that schedules reminders from the event
    /// stays in sync with the on-chain rule instead of drifting later by however
    /// long it has been since the last check-in (#357).
    pub fn update_periods(
        env: Env,
        will_id: u64,
        owner: Address,
        checkin_period_days: Option<u64>,
        grace_period_days: Option<u64>,
    ) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        // Update checkin period if provided
        if let Some(new_checkin) = checkin_period_days {
            let valid = 1..=MAX_PERIOD_DAYS;
            if !valid.contains(&new_checkin) {
                panic_with_error!(&env, WillError::InvalidPeriod);
            }
            will.checkin_period_days = new_checkin;
        }

        // Update grace period if provided
        if let Some(new_grace) = grace_period_days {
            let valid = 1..=MAX_PERIOD_DAYS;
            if !valid.contains(&new_grace) {
                panic_with_error!(&env, WillError::InvalidPeriod);
            }
            will.grace_period_days = new_grace;
        }

        // `trigger_will` derives the deadline from `last_checkin`, which this call
        // leaves untouched, so the event must carry that same value rather than
        // `now + period` — otherwise the published deadline is later than the
        // enforced one by the age of the current check-in (#357).
        let next_deadline = will.last_checkin + will.checkin_period_days * SECONDS_PER_DAY;
        storage::save_will(&env, &will);

        events::periods_updated(
            &env,
            will_id,
            &owner,
            will.checkin_period_days,
            will.grace_period_days,
            next_deadline,
        );
    }

    /// Atomically updates multiple will settings (beneficiaries, guardians, and periods)
    /// in a single transaction. Only callable by the owner while the will is `Active`.
    ///
    /// Any unspecified field (passed as `None`) is left unchanged. This allows callers
    /// to update only the settings they need without specifying the others.
    ///
    /// # Parameters
    /// - `will_id`: the will to update
    /// - `owner`: the will's owner; must authorize this call
    /// - `beneficiaries`: new beneficiary list (optional); if specified, must be valid
    /// - `guardians`: new guardian list (optional); if specified, must be valid
    /// - `checkin_period_days`: new check-in period (optional)
    /// - `grace_period_days`: new grace period (optional)
    ///
    /// # Events
    /// Always emits [`events::will_settings_updated`] with an `updated_fields`
    /// `Vec<Symbol>` listing every field that changed (`"benef"`, `"guard"`,
    /// `"checkin"`, `"grace"`). This is the canonical way to detect which settings
    /// were modified in a single call.
    ///
    /// When `guardians` is `Some(…)`, this function **also** emits
    /// [`events::guardians_updated`] (topic `"guardup"`) so that off-chain consumers
    /// subscribed to that topic are notified consistently regardless of whether the
    /// guardian change was made through [`Self::update_guardians`] or through this
    /// composite entry point.
    ///
    /// Like [`Self::update_periods`], a period change here leaves `last_checkin`
    /// alone, so the enforced check-in deadline moves to
    /// `last_checkin + checkin_period_days * SECONDS_PER_DAY`. No deadline is
    /// published by this function; read the resulting one with
    /// [`Self::get_time_until_deadline`], or use [`Self::update_periods`] if the
    /// consumer also needs the `periodu` event (#357).
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - Any validation error from the specific update functions.
    pub fn update_will_settings(
        env: Env,
        will_id: u64,
        owner: Address,
        beneficiaries: Option<Vec<Beneficiary>>,
        guardians: Option<Vec<Address>>,
        checkin_period_days: Option<u64>,
        grace_period_days: Option<u64>,
    ) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        let mut updated_fields: Vec<soroban_sdk::Symbol> = Vec::new(&env);

        // Update beneficiaries if provided
        if let Some(new_beneficiaries) = beneficiaries {
            assert_beneficiary_count(&env, &new_beneficiaries);
            assert_valid_allocations(&env, &new_beneficiaries, total_balance(&will.balances));
            if new_beneficiaries.is_empty() || new_beneficiaries.len() > MAX_BENEFICIARIES {
                panic_with_error!(&env, WillError::TooManyBeneficiaries);
            }
            assert_valid_allocations(
                &env,
                &new_beneficiaries,
                primary_token_balance(&will.balances, &will.token),
            );

            // Update reverse indexes
            for old in will.beneficiaries.iter() {
                if !names_address(&new_beneficiaries, &old.address) {
                    storage::remove_beneficiary_index(&env, &old.address, will_id);
                }
            }
            for new_beneficiary in new_beneficiaries.iter() {
                if !names_address(&will.beneficiaries, &new_beneficiary.address) {
                    storage::index_by_beneficiary(&env, &new_beneficiary.address, will_id);
                }
            }

            will.beneficiaries = new_beneficiaries;
            updated_fields.push_back(symbol_short!("benef"));
        }

        // Update guardians if provided
        let guardians_changed = if let Some(new_guardians) = guardians {
            assert_valid_guardians(&env, &owner, &new_guardians);
            assert_no_guardian_cancel_in_flight(&env, &will);

            // Same threshold invariant enforced in update_guardians: a non-empty
            // new list must not leave the existing guardian_threshold unreachable.
            if !new_guardians.is_empty() && will.guardian_threshold > new_guardians.len() {
                panic_with_error!(&env, WillError::InvalidGuardianThreshold);
            }

            let now = env.ledger().timestamp();
            storage::reset_guardian_votes(&env, &will);
            storage::reset_guardian_cancel_votes(&env, &will);
            let mut guardian_structs: Vec<Guardian> = Vec::new(&env);
            for addr in new_guardians.iter() {
                guardian_structs.push_back(Guardian {
                    address: addr,
                    weight: 1,
                    consent: GuardianConsent::Pending,
                });
            }
            will.guardians = guardian_structs;
            will.guardian_votes = 0;
            will.guardian_vote_weight = 0;
            will.guardian_cancel_votes = 0;
            will.guardian_cancel_vote_weight = 0;
            will.guardian_list_updated_at = now;
            updated_fields.push_back(symbol_short!("guard"));
            true
        } else {
            false
        };

        // Update checkin period if provided
        if let Some(new_checkin) = checkin_period_days {
            let valid = 1..=MAX_PERIOD_DAYS;
            if !valid.contains(&new_checkin) {
                panic_with_error!(&env, WillError::InvalidPeriod);
            }
            will.checkin_period_days = new_checkin;
            updated_fields.push_back(symbol_short!("checkin"));
        }

        // Update grace period if provided
        if let Some(new_grace) = grace_period_days {
            let valid = 1..=MAX_PERIOD_DAYS;
            if !valid.contains(&new_grace) {
                panic_with_error!(&env, WillError::InvalidPeriod);
            }
            will.grace_period_days = new_grace;
            updated_fields.push_back(symbol_short!("grace"));
        }

        // Save the will with all updates applied
        storage::save_will(&env, &will);

        // Emit the consolidated settings event so consumers can inspect which
        // fields changed in a single subscription.
        events::will_settings_updated(&env, will_id, &owner, &updated_fields);

        // Also emit the dedicated `guardians_updated` event so that off-chain
        // consumers subscribed to that topic (e.g. to invalidate cached guardian
        // consent state) receive the notification consistently, regardless of
        // whether the guardian change was made through `update_guardians` or
        // through this composite entry point.
        if guardians_changed {
            events::guardians_updated(&env, will_id, &owner, &will.guardians);
        }
    }

    /// Adds `amount` of a specific `token` to an existing will's locked
    /// balance. Only possible while the will is `Active`. The token does not
    /// need to have been part of the original `create_will` call — new tokens
    /// can be added via `top_up`, up to the same [`MAX_TOKENS`]
    /// distinct-token cap `create_will` enforces.
    ///
    /// Topping up a `token` the will already holds a balance for is not a
    /// distinct code path: `will.balances` is a `Map<Address, i128>`, whose
    /// keys are unique by construction, so there is no way for a token to
    /// appear twice regardless of how many times `top_up` is called for it.
    /// Each call reads the existing entry (`0` if genuinely new) and writes
    /// back `existing + amount` — repeated top-ups of the same token always
    /// accumulate; they never overwrite or duplicate (#499).
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::ZeroAmount`] if `amount` is not positive.
    /// - [`WillError::InvalidTokenCount`] if `token` is not already in the
    ///   will's `balances` map and that map already holds [`MAX_TOKENS`]
    ///   distinct tokens (#358). Topping up a token the will already holds is
    ///   always allowed, including at the cap, because it does not grow the
    ///   map.
    pub fn top_up(env: Env, will_id: u64, owner: Address, token: Address, amount: i128) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        if amount <= 0 {
            panic_with_error!(&env, WillError::ZeroAmount);
        }

        // Bound the token count exactly as `create_will` does. `distribute`
        // and `cancel_will` iterate every entry of `balances`, so an unbounded
        // map would let the owner add one distinct token at a time until a
        // release or a refund no longer fits in a Soroban transaction, leaving
        // the will's funds effectively unreachable (#358). Below the cap there
        // is nothing to check; at the cap an already-held token is still
        // allowed because topping it up does not grow the map.
        if will.balances.len() >= MAX_TOKENS && !will.balances.contains_key(token.clone()) {
            panic_with_error!(&env, WillError::InvalidTokenCount);
        }

        // Snapshot values needed after state mutation (checks-effects-interactions).
        let prev = will.balances.get(token.clone()).unwrap_or(0);
        let new_balance = prev + amount;

        // --- EFFECTS: update state and persist before the external transfer ---
        will.balances.set(token.clone(), new_balance);
        // `will.balance` is a legacy mirror of `will.balances[will.token]` kept
        // for backward compatibility until callers migrate fully to the
        // multi-token map; it must be kept in sync here or every reader that
        // still trusts it (e.g. split_will's balance check, reveal_and_claim's
        // share computation) will silently operate on a stale figure.
        if token == will.token {
            will.balance = new_balance;
        }
        storage::save_will(&env, &will);

        // Increment locked value for this token
        storage::adjust_locked_value(&env, &token, amount);

        // --- INTERACTIONS: external token transfer after state is committed ---
        token::Client::new(&env, &token).transfer(&owner, &env.current_contract_address(), &amount);

        events::top_up(&env, will_id, &owner, &token, amount, new_balance);
    }

    /// Returns the contract version as a `u32` encoded semver value:
    /// `major * 1_000_000 + minor * 1_000 + patch`.
    ///
    /// SDKs and apps can call this to detect version mismatches before
    /// submitting transactions that depend on specific contract behaviour.
    ///
    /// # Why this returns a compiled-in constant, not something read back out
    /// of the deployed Wasm at call time (#501)
    ///
    /// A Soroban contract's execution environment has no host function that
    /// lets a contract inspect its own deployed Wasm binary's custom sections
    /// (which is where `soroban_sdk::contractmeta!`'s `"Version"` entry,
    /// declared further up this file, actually lives) -- `CONTRACT_VERSION`
    /// *is* "the binary" as far as on-chain execution can observe: it is
    /// compiled directly into the function body that returns it, so there is
    /// no separate copy inside the same binary it could drift from at
    /// runtime. What *can* drift, because they are three independent literals
    /// a human edits, are:
    /// - `CONTRACT_VERSION` here vs. the crate `version` in
    ///   `contracts/will/Cargo.toml` -- checked by `cargo test` via
    ///   `issue_501_test.rs`'s `CARGO_PKG_VERSION` comparison, and again by
    ///   `.github/scripts/check-contract-version.sh` (run in CI from
    ///   `test.yml`).
    /// - `CONTRACT_VERSION` here vs. the `contractmeta!(key = "Version", ...)`
    ///   string literal -- checked by `issue_272_test.rs`.
    /// - The source `CONTRACT_VERSION` that was compiled vs. the Wasm binary
    ///   actually deployed on a given network -- source-level checks cannot
    ///   catch a stale or wrong artifact being deployed, so
    ///   `scripts/deploy-testnet.sh` calls `get_contract_version` on the
    ///   freshly deployed contract immediately after deployment and fails the
    ///   deploy if it disagrees with the source constant, rather than
    ///   silently recording a contract id that doesn't match what was built.
    pub fn get_contract_version(_env: Env) -> u32 {
        CONTRACT_VERSION
    }

    /// Returns the full on-chain state of `will_id`.
    pub fn get_will(env: Env, will_id: u64) -> Will {
        load_will(&env, will_id)
    }

    /// Returns just the lifecycle status of `will_id`.
    ///
    /// Note: This function still loads the full `Will` struct from persistent
    /// storage and deserializes it. The dominant cost is the storage read and
    /// deserialization, not the return-value encoding. Use this method instead
    /// of [`Self::get_will`] only when you need the status and do not require
    /// other will fields.
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if no will exists with this id.
    pub fn get_will_status(env: Env, will_id: u64) -> WillStatus {
        load_will(&env, will_id).status
    }

    /// Returns the number of seconds until `will_id`'s next relevant
    /// deadline, or `None` if the will's current status has no deadline.
    ///
    /// The deadline depends on status:
    /// - `Active`: seconds until the check-in deadline
    ///   (`last_checkin + checkin_period_days`).
    /// - `Triggered`: seconds until the grace period expires
    ///   (`trigger_time + grace_period_days`, where `trigger_time` is the
    ///   missed check-in deadline — see [`Self::trigger_will`]).
    /// - `PendingConfirmation`, `Released`, `Cancelled`, `Settled`: no deadline
    ///   applies; returns `None`.
    ///
    /// The returned value is negative when the deadline has already passed
    /// (e.g. an `Active` will whose check-in deadline elapsed but which has
    /// not yet been `trigger_will`-ed, or a `Triggered` will whose grace
    /// period has expired but has not yet been released) — callers should
    /// treat any non-positive value as "actionable now" rather than treating
    /// only `None` as the past-due signal.
    ///
    /// A `Triggered` will reporting exactly `0` is sitting on its grace
    /// deadline second, which still belongs to the owner:
    /// [`Self::emergency_checkin`] succeeds there and [`Self::release_inheritance`]
    /// does not until the following second (#354). A non-positive value
    /// therefore still means the owner should be alerted, not that the estate
    /// is already releasable.
    ///
    /// Note: This function still loads the full `Will` struct from persistent
    /// storage and deserializes it. The dominant cost is the storage read and
    /// deserialization, not the return-value encoding. Use this method instead
    /// of [`Self::get_will`] only when you need the deadline and do not require
    /// other will fields.
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if no will exists with this id.
    pub fn get_time_until_deadline(env: Env, will_id: u64) -> Option<i64> {
        let will = load_will(&env, will_id);
        let now = env.ledger().timestamp() as i64;

        match will.status {
            WillStatus::Active => {
                let deadline =
                    will.last_checkin as i64 + (will.checkin_period_days * SECONDS_PER_DAY) as i64;
                Some(deadline - now)
            }
            WillStatus::Triggered => {
                // `trigger_will` is the only path that sets `WillStatus::Triggered`,
                // and it always sets `trigger_time` to `Some(now)` in the same
                // write. `trigger_time` should therefore never be `None` here;
                // the `unwrap_or` exists only as a defensive fallback in case a
                // future entry point ever saves a `Triggered` will without it,
                // in which case it deliberately degrades to `last_checkin`
                // (understating the elapsed grace period) rather than panicking.
                let trigger_time = will.trigger_time.unwrap_or(will.last_checkin) as i64;
                let deadline = trigger_time + (will.grace_period_days * SECONDS_PER_DAY) as i64;
                Some(deadline - now)
            }
            WillStatus::PendingConfirmation
            | WillStatus::Released
            | WillStatus::Cancelled
            | WillStatus::Settled => None,
        }
    }

    /// Returns `guardian`'s current vote record for `will_id`'s active
    /// trigger cycle -- the timestamp their `guardian_trigger` vote was cast
    /// and the reason they gave -- or `None` if they have not voted, or their
    /// vote has since expired past the will's grace period.
    ///
    /// Lets a guardian's own dashboard show "you already voted" state
    /// directly from chain state, without replaying `guardian_voted` events
    /// off-chain (#263).
    ///
    /// Expiry is decided by the same [`storage::vote_is_live`] rule
    /// `has_guardian_voted` and `has_guardian_cancel_voted` use, so this read
    /// can never panic on a record whose timestamp is later than the current
    /// ledger time (#359). Unlike those quorum counters, a record timestamped
    /// after `now` is reported as-is here rather than hidden: this is a
    /// read-only query over stored state, and swallowing the record would make
    /// a skewed ledger clock look like "no vote was ever cast".
    pub fn get_guardian_vote_status(
        env: Env,
        will_id: u64,
        guardian: Address,
    ) -> Option<GuardianVoteRecord> {
        let will = load_will(&env, will_id);
        let record = storage::get_guardian_vote(&env, will_id, &guardian)?;
        let now = env.ledger().timestamp();
        if storage::vote_is_live(now, record.timestamp, will.grace_period_days) {
            Some(record)
        } else {
            None
        }
    }

    /// Returns aggregate protocol statistics for all wills currently tracked on-chain.
    ///
    /// The counters are maintained incrementally and reflect only successful
    /// operations: a failed invocation rolls back all of its writes, so it
    /// can never be counted. See [`ProtocolStats`] for the consistency
    /// invariant and [`Self::audit_protocol_stats`] to verify it.
    pub fn get_protocol_stats(env: Env) -> ProtocolStats {
        storage::get_protocol_stats(&env)
    }

    /// Read-only invariant check (#441): recomputes the protocol stats by
    /// walking every will in storage and compares them with the stored,
    /// incrementally maintained counters.
    ///
    /// Tokens are compared by total, so a token present in one side with a
    /// zero total and absent from the other still counts as consistent.
    ///
    /// Cost grows linearly with the number of wills ever created; once that
    /// exceeds the per-invocation budget, run it via simulation only.
    pub fn audit_protocol_stats(env: Env) -> ProtocolStatsAudit {
        let stored = storage::get_protocol_stats(&env);
        let computed = storage::recompute_protocol_stats(&env);
        let consistent = stats_match(&stored, &computed);
        ProtocolStatsAudit {
            stored,
            computed,
            consistent,
        }
    }

    /// Repairs bookkeeping drift (#441) by overwriting the stored protocol
    /// stats with the values recomputed from storage, and returns them.
    ///
    /// Callable by anyone: the written values are derived purely from
    /// on-chain will state, so a caller cannot influence them. Tokens whose
    /// recomputed total is zero are dropped from the stored list.
    pub fn repair_protocol_stats(env: Env) -> ProtocolStats {
        let computed = storage::recompute_protocol_stats(&env);
        let mut total_locked_by_token = Vec::new(&env);
        for entry in computed.total_locked_by_token.iter() {
            if entry.total_locked != 0 {
                total_locked_by_token.push_back(entry);
            }
        }
        let repaired = ProtocolStats {
            active_will_count: computed.active_will_count,
            total_locked_by_token,
        };
        storage::save_protocol_stats(&env, &repaired);
        repaired
    }

    /// Returns the list of will ids currently in `Triggered` status.
    /// Returns a page of the will ids currently in `Triggered` status.
    ///
    /// This is the on-chain index that lets keeper bots and monitoring tools
    /// efficiently discover wills that are past their check-in deadline and
    /// within their grace period, without having to replay every
    /// `will_triggered` event off-chain.
    ///
    /// # Parameters
    /// - `cursor`: optional will id to paginate after (exclusive). Pass `None`
    ///   or `0` for the first page.
    /// - `limit`: maximum number of ids to return. Capped at
    ///   [`storage::MAX_PAGE_SIZE`].
    ///
    /// # Paging
    ///
    /// Pass the last id of the previous page back as `cursor` to fetch the
    /// next one; page until a page comes back shorter than the `limit` you
    /// asked for (or empty), which means you have seen the whole index.
    ///
    /// # Bounding
    ///
    /// The index only ever holds wills that are *currently* `Triggered`:
    /// `emergency_checkin`, `guardian_cancel_trigger`, `release_inheritance`,
    /// `guardian_trigger`, `cancel_will`, and `archive_will` all remove the
    /// id again, using order-preserving removal so paging never skips or
    /// repeats an entry. A global hard cap is deliberately **not** applied
    /// here — any single address could otherwise exhaust it and break
    /// `trigger_will` for the whole protocol. See `storage::index_triggered_will`
    /// for the full bounding strategy.
    pub fn get_triggered_wills(env: Env, cursor: Option<u64>, limit: u32) -> Vec<u64> {
        storage::get_triggered_wills_page(&env, cursor, limit)
    }

    /// Returns a page of wills owned by `owner`.
    ///
    /// Supports bounded pagination to avoid hitting Soroban resource limits
    /// for addresses with many wills.
    ///
    /// # Parameters
    /// - `owner`: the address to query wills for.
    /// - `cursor`: optional will id to paginate after (exclusive). Pass `None`
    ///   or `0` for the first page.
    /// - `limit`: maximum number of wills to return. Capped at
    ///   [`storage::MAX_PAGE_SIZE`].
    ///
    /// For totals across all pages (will count, locked value per token) call
    /// [`Self::get_owner_stats`] instead of iterating every page.
    pub fn get_wills_by_owner(
        env: Env,
        owner: Address,
        cursor: Option<u64>,
        limit: u32,
    ) -> Vec<Will> {
        let ids = storage::get_owner_wills(&env, &owner);
        let page = storage::paginate_ids(&env, &ids, cursor, limit);
        let mut wills = Vec::new(&env);
        for id in page.iter() {
            wills.push_back(match storage::load_will(&env, id) {
                Ok(will) => will,
                Err(e) => panic_with_error!(&env, e),
            });
        }
        wills
    }

    /// Returns aggregate statistics for every will owned by `owner`
    /// (issue #447).
    ///
    /// Complements the paginated [`Self::get_wills_by_owner`]: a client can
    /// fetch one page of wills plus this summary in two calls instead of
    /// walking every page to compute totals. The owner index is capped at
    /// `storage::MAX_WILLS_PER_INDEX`, which bounds the work done here.
    ///
    /// `total_wills` counts every indexed will regardless of status;
    /// `active_wills` and `total_locked_by_token` cover only non-terminal
    /// wills (`PendingConfirmation`, `Active`, `Triggered`), matching how
    /// [`ProtocolStats`] counts locked value.
    pub fn get_owner_stats(env: Env, owner: Address) -> OwnerStats {
        let ids = storage::get_owner_wills(&env, &owner);
        let mut active_wills: u32 = 0;
        let mut locked: Map<Address, i128> = Map::new(&env);
        for id in ids.iter() {
            let will = match storage::load_will(&env, id) {
                Ok(w) => w,
                Err(e) => panic_with_error!(&env, e),
            };
            if matches!(
                will.status,
                WillStatus::PendingConfirmation | WillStatus::Active | WillStatus::Triggered
            ) {
                active_wills += 1;
                for (token_addr, amount) in will.balances.iter() {
                    let prev = locked.get(token_addr.clone()).unwrap_or(0);
                    locked.set(token_addr, prev + amount);
                }
            }
        }

        let mut total_locked_by_token = Vec::new(&env);
        for (token, total_locked) in locked.iter() {
            total_locked_by_token.push_back(TokenLockedBalance {
                token,
                total_locked,
            });
        }

        OwnerStats {
            total_wills: ids.len(),
            active_wills,
            total_locked_by_token,
        }
    }

    /// Returns a page of wills owned by `owner` with the given `status`.
    ///
    /// Supports bounded pagination to avoid hitting Soroban resource limits
    /// for addresses with many wills. The status filter is applied *before*
    /// the page is cut, so a page is only shorter than `limit` when there are
    /// no further matching wills (#455).
    ///
    /// Prefer [`Self::get_wills_by_owner_and_status_page`], which also returns
    /// the total number of matches and an explicit `next_cursor`.
    ///
    /// # Parameters
    /// - `owner`: the address to query wills for.
    /// - `status`: the will status to filter by.
    /// - `cursor`: optional will id to paginate after (exclusive). Pass `None`
    ///   or `0` for the first page.
    /// - `limit`: maximum number of wills to return. Capped at
    ///   [`storage::MAX_PAGE_SIZE`]. A `limit` of `0` returns an empty page,
    ///   matching [`Self::get_wills_by_owner`] and
    ///   [`Self::get_wills_by_beneficiary`], which build their pages through
    ///   [`storage::paginate_ids`] (#360).
    pub fn get_wills_by_owner_and_status(
        env: Env,
        owner: Address,
        status: WillStatus,
        cursor: Option<u64>,
        limit: u32,
    ) -> Vec<Will> {
        wills_by_owner_and_status_page(&env, &owner, status, cursor, limit).wills
    }

    /// Returns a page of wills owned by `owner` with the given `status`,
    /// together with the total number of matching wills and the cursor for
    /// the next page (#455).
    ///
    /// Callers can page through every match with a single query per page and
    /// know up front how many results exist:
    ///
    /// ```ignore
    /// let mut cursor = None;
    /// loop {
    ///     let page = client.get_wills_by_owner_and_status_page(&owner, &status, &cursor, &20);
    ///     // page.total_count is the same on every page.
    ///     handle(page.wills);
    ///     match page.next_cursor {
    ///         Some(c) => cursor = Some(c),
    ///         None => break,
    ///     }
    /// }
    /// ```
    ///
    /// # Parameters
    /// - `owner`: the address to query wills for.
    /// - `status`: the will status to filter by.
    /// - `cursor`: optional will id to paginate after (exclusive). Pass `None`
    ///   or `0` for the first page.
    /// - `limit`: maximum number of wills to return. Capped at
    ///   [`storage::MAX_PAGE_SIZE`].
    pub fn get_wills_by_owner_and_status_page(
        env: Env,
        owner: Address,
        status: WillStatus,
        cursor: Option<u64>,
        limit: u32,
    ) -> WillPage {
        let ids = storage::get_owner_wills(&env, &owner);
        let page_size = limit.min(storage::MAX_PAGE_SIZE);
        let mut wills = Vec::new(&env);
        let cursor_val = cursor.unwrap_or(0);
        let skip = cursor.is_some();
        let mut skipping = skip;

        for id in ids.iter() {
            // Check the page size *before* considering another will, exactly
            // like `storage::paginate_ids` does: with `limit == 0` the loop
            // exits immediately and the page comes back empty, instead of
            // collecting one will before a post-push size check could stop it
            // (#360).
            if wills.len() >= page_size {
                break;
            }
            if skipping {
                if id <= cursor_val {
                    continue;
                }
                skipping = false;
            }
            let will = match storage::load_will(&env, id) {
                Ok(w) => w,
                Err(e) => panic_with_error!(&env, e),
            };
            if will.status == status {
                wills.push_back(will);
            }
        }
        wills
    }

    /// Returns wills `beneficiary` is named in, with optional pagination.
    ///
    /// # Parameters
    /// - `beneficiary`: the address to query for
    /// - `cursor`: optional will id to start pagination after (exclusive)
    /// - `limit`: maximum number of wills to return (capped at MAX_PAGE_SIZE)
    ///
    /// # Pagination
    /// To fetch all wills in pages:
    /// 1. Call with `cursor=None, limit=N`
    /// 2. If result has N wills, call again with `cursor=last_will_id`
    /// 3. Repeat until result has fewer than N wills
    pub fn get_wills_by_beneficiary(
        env: Env,
        beneficiary: Address,
        cursor: Option<u64>,
        limit: u32,
    ) -> Vec<Will> {
        let ids = storage::get_beneficiary_wills(&env, &beneficiary);
        let paginated_ids = storage::paginate_ids(&env, &ids, cursor, limit);
        let mut wills = Vec::new(&env);
        for id in paginated_ids.iter() {
            wills.push_back(match storage::load_will(&env, id) {
                Ok(will) => will,
                Err(e) => panic_with_error!(&env, e),
            });
        }
        wills
    }

    /// Fetches a caller-chosen set of wills by their ids in a single call.
    ///
    /// This is useful for application dashboards that already know a handful
    /// of relevant will ids (e.g. collected from prior events) and want a
    /// fresh read of just those wills without re-deriving the owner/beneficiary
    /// indexes.
    ///
    /// # Parameters
    /// - `ids`: the list of will ids to fetch. Must not exceed
    ///   [`MAX_GET_WILLS_IDS`] entries.
    ///
    /// # Returns
    /// A `Vec<Will>` containing only the wills that exist. Any id that does
    /// not map to a stored will is silently skipped (no panic). The result
    /// preserves the input order, minus the missing ids.
    ///
    /// **Duplicate ids are not deduplicated.** If the same id appears more
    /// than once in `ids`, the corresponding `Will` struct is returned once
    /// per occurrence. Callers performing client-side aggregation (e.g.
    /// summing balances across the returned batch) must deduplicate the input
    /// ids themselves to avoid double-counting.
    ///
    /// **Skipping vs. panicking:** the owner/beneficiary index functions
    /// (`get_wills_by_owner`, `get_wills_by_beneficiary`) also skip missing
    /// ids for the same reason — stale index entries can arise after a will
    /// is cancelled or released. `get_wills` follows the same convention so
    /// callers can safely pass any id without error-handling overhead.
    ///
    /// # Panics
    /// - [`WillError::TooManyIds`] if `ids.len()` exceeds `MAX_GET_WILLS_IDS`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Fetch a specific set of will ids, including one that does not exist.
    /// let wills = client.get_wills(&vec![&env, will_id_a, will_id_b, 9999_u64]);
    ///
    /// // Only the two real wills are returned; 9999 is silently skipped.
    /// assert_eq!(wills.len(), 2);
    ///
    /// // Passing the same id twice yields two copies of the same Will.
    /// let dupes = client.get_wills(&vec![&env, will_id_a, will_id_a]);
    /// assert_eq!(dupes.len(), 2);
    /// assert_eq!(dupes.get(0).unwrap().id, will_id_a);
    /// assert_eq!(dupes.get(1).unwrap().id, will_id_a);
    /// ```
    pub fn get_wills(env: Env, ids: Vec<u64>) -> Vec<Will> {
        if ids.len() > MAX_GET_WILLS_IDS {
            panic_with_error!(&env, WillError::TooManyIds);
        }
        let mut wills = Vec::new(&env);
        for id in ids.iter() {
            if let Ok(will) = storage::load_will(&env, id) {
                wills.push_back(will);
            }
        }
        wills
    }

    /// Casts a guardian vote to force an early release of `will_id`, for use
    /// when the owner is known to be incapacitated. Once
    /// `guardian_threshold` distinct guardians have voted, all balances are
    /// immediately distributed to beneficiaries, bypassing the check-in and
    /// grace-period flow entirely.
    ///
    /// Keeper bounties are never paid on a guardian-triggered release:
    /// `distribute` is called with `keeper = None`, so `keeper_bounty_bps`
    /// has no effect in this path. Only [`release_inheritance`]'s
    /// caller-supplied `Option<Address>` can trigger a bounty payment.
    ///
    /// Enforces a cooldown after a guardian-list change: if the current
    /// guardian list was updated less than [`GUARDIAN_COOLDOWN_DAYS`] days ago,
    /// the vote is rejected with [`WillError::GuardianCooldownActive`].
    ///
    /// # Parameters
    /// - `reason`: the reason the guardian is casting the vote.
    ///
    /// # Reason codes are informational metadata — there is no on-chain consensus requirement
    ///
    /// Each guardian supplies their own [`GuardianVoteReason`] independently.
    /// The contract records that reason in the guardian's
    /// [`storage::GuardianVoteRecord`] for off-chain auditing, but it plays
    /// **no role in the quorum calculation**: the only on-chain invariant
    /// checked is `guardian_votes >= guardian_threshold`. As a direct
    /// consequence:
    ///
    /// - Two (or more) guardians may vote with **different, even contradictory**
    ///   reason codes and still reach quorum.  For example, one guardian may
    ///   vote [`GuardianVoteReason::Deceased`] while another votes
    ///   [`GuardianVoteReason::Incapacitated`] — the will is released once
    ///   the threshold is met regardless.
    /// - There is no mechanism that prevents a guardian from choosing
    ///   [`GuardianVoteReason::Other`] for any situation, including ones
    ///   covered by a more specific code.
    ///
    /// This is an intentional consequence of the trustless design: the
    /// contract cannot verify off-chain evidence, so it makes no attempt to
    /// do so.  The `reason` field exists to give beneficiaries and auditors a
    /// human-readable signal about *why* guardians acted — it is not a
    /// binding commitment or a consensus input.
    ///
    /// # Panics
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::NotGuardian`] if `guardian` is not one of the will's guardians.
    /// - [`WillError::AlreadyVoted`] if `guardian` already voted in this cycle.
    /// - [`WillError::GuardianCooldownActive`] if the guardian-list cooldown has not elapsed.
    ///
    /// # Vote expiry and recounting
    ///
    /// A vote stops counting once it is older than the will's
    /// `grace_period_days`, after which the same guardian may vote again. When
    /// they do, the accumulated `guardian_vote_weight` / `guardian_votes` are
    /// **recomputed from the vote records that are still live**, so the new vote
    /// replaces the expired one rather than being added on top of it (#372). One
    /// guardian therefore cannot reach `guardian_threshold` alone by voting once
    /// per expiry window. The same recounting applies to
    /// [`guardian_cancel_trigger`]'s cancel-vote counters.
    pub fn guardian_trigger(env: Env, will_id: u64, guardian: Address, reason: GuardianVoteReason) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        // Enforce guardian-list cooldown.
        let now = env.ledger().timestamp();
        let cooldown_seconds = GUARDIAN_COOLDOWN_DAYS * SECONDS_PER_DAY;
        let cooldown_ends = will.guardian_list_updated_at + cooldown_seconds;
        if now < cooldown_ends {
            panic_with_error!(&env, WillError::GuardianCooldownActive);
        }

        let weight = match will.guardians.iter().find(|g| g.address == guardian) {
            Some(g) => {
                if g.consent != GuardianConsent::Accepted {
                    panic_with_error!(&env, WillError::GuardianNotConsented);
                }
                g.weight
            }
            None => panic_with_error!(&env, WillError::NotGuardian),
        };

        let expiry_days = will.grace_period_days;
        if storage::has_guardian_voted(&env, will_id, &guardian, now, expiry_days) {
            panic_with_error!(&env, WillError::AlreadyVoted);
        }

        // Record (or refresh an expired) vote, then recompute the tally from
        // the per-guardian records so each guardian counts at most once and
        // expired votes drop out (#458). A running `+= 1` would let a guardian
        // whose earlier vote expired be counted twice.
        storage::set_guardian_voted(&env, will_id, &guardian, now, reason);
        let (tally_weight, tally_count) =
            storage::tally_guardian_votes(&env, &will, now, expiry_days);
        will.guardian_vote_weight = tally_weight;
        will.guardian_votes = tally_count;
        // Recount from the current guardian list instead of incrementing a
        // running total (#453), so votes from removed, rejected or expired
        // guardians can never count toward the threshold.
        let (votes, vote_weight) =
            tally_guardian_votes(&env, &will, now, storage::has_guardian_voted);
        will.guardian_votes = votes;
        will.guardian_vote_weight = vote_weight;
        // Recount from the vote records that are still live rather than adding
        // to the persisted counters: a record that has already aged past the
        // expiry window no longer counts, so voting again after expiry replaces
        // the old vote rather than stacking on top of it (#372). Without this a
        // single guardian could vote once per grace period and reach the
        // threshold alone.
        let (live_weight, live_votes) =
            storage::recount_guardian_votes(&env, &will, now, expiry_days);
        will.guardian_vote_weight = live_weight;
        will.guardian_votes = live_votes;
        storage::save_will(&env, &will);

        events::guardian_voted(&env, will_id, &guardian, weight, will.guardian_vote_weight);

        if will.guardian_vote_weight >= will.guardian_threshold
            && guardian_vote_freshness::live_guardian_vote_weight(&env, will_id, &will, now)
                >= will.guardian_threshold
        {
            record_transition(
                &env,
                will_id,
                WillStatus::Active,
                WillStatus::Released,
                &guardian,
                symbol_short!("gtrigr"),
            );
            distribute(&env, &mut will, &None);
        }
    }

    /// Casts a guardian vote to **cancel** an in-progress trigger and return
    /// the will to `Active` status. This mirrors [`guardian_trigger`] but
    /// operates on the opposite outcome: instead of forcing a release, a quorum
    /// of guardians can collectively decide that the owner is still alive and
    /// the trigger was premature.
    ///
    /// Vote records are stored under a separate `GuardianCancelVote` key so
    /// that a guardian's release-vote and their cancel-vote are completely
    /// independent — casting one does **not** prevent casting the other, but
    /// a single guardian's vote cannot count toward both outcomes
    /// simultaneously (each namespace is deduplicated on its own).
    ///
    /// When the accumulated cancel-vote weight reaches `guardian_threshold`,
    /// the will is returned to `Active`, `last_checkin` is reset to now
    /// (starting a fresh check-in countdown), and all cancel-vote records are
    /// cleared.
    ///
    /// # Grace period
    ///
    /// A cancel vote is only meaningful *during* the grace period, and this
    /// entrypoint enforces the same deadline [`emergency_checkin`] does: once
    /// `trigger_time + grace_period_days` has passed, the cancel is rejected
    /// with [`WillError::GracePeriodExpired`] and the will can no longer be
    /// rewound to `Active`. Without that check a guardian quorum could undo an
    /// expired trigger *after* the funds had already become releasable through
    /// [`release_inheritance`], and could repeat the trick every check-in cycle
    /// to block the release indefinitely (#373).
    ///
    /// # Parameters
    /// - `will_id`: the will whose trigger should be cancelled.
    /// - `guardian`: the guardian casting the cancel vote; must authorize.
    ///
    /// # Panics
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodExpired`] if the will's grace period has
    ///   already elapsed (#373).
    /// - [`WillError::NotGuardian`] if `guardian` is not one of the will's guardians.
    /// - [`WillError::AlreadyVoted`] if `guardian` already cast a cancel vote in this cycle.
    /// - [`WillError::GuardianCooldownActive`] if the guardian-list cooldown has not elapsed.
    pub fn guardian_cancel_trigger(env: Env, will_id: u64, guardian: Address) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);
        assert_status(
            &env,
            &will,
            WillStatus::Triggered,
            WillError::WillNotTriggered,
        );

        let now = env.ledger().timestamp();

        // The grace period is the window in which a trigger may still be undone.
        // Past it the estate is releasable via `release_inheritance`, so a
        // cancel quorum must not be able to rewind the will to `Active` and
        // restart the check-in clock (#373) -- same rule `emergency_checkin`
        // enforces, so a guardian quorum and the owner are held to one deadline.
        let trigger_time = will.trigger_time.unwrap_or(0);
        let grace_deadline = trigger_time + will.grace_period_days * SECONDS_PER_DAY;
        if now > grace_deadline {
            panic_with_error!(&env, WillError::GracePeriodExpired);
        }

        // Enforce guardian-list cooldown (same rule as guardian_trigger).
        let cooldown_seconds = GUARDIAN_COOLDOWN_DAYS * SECONDS_PER_DAY;
        let cooldown_ends = will.guardian_list_updated_at + cooldown_seconds;
        if now < cooldown_ends {
            panic_with_error!(&env, WillError::GuardianCooldownActive);
        }

        // Verify the caller is a named guardian and capture their weight.
        let weight = match will.guardians.iter().find(|g| g.address == guardian) {
            Some(g) => {
                if g.consent != GuardianConsent::Accepted {
                    panic_with_error!(&env, WillError::GuardianNotConsented);
                }
                g.weight
            }
            None => panic_with_error!(&env, WillError::NotGuardian),
        };

        // Deduplicate within the cancel-vote namespace only.
        let expiry_days = will.grace_period_days;
        if storage::has_guardian_cancel_voted(&env, will_id, &guardian, now, expiry_days) {
            panic_with_error!(&env, WillError::AlreadyVoted);
        }

        storage::set_guardian_cancel_voted(&env, will_id, &guardian, now);
        let (tally_weight, tally_count) =
            storage::tally_guardian_cancel_votes(&env, &will, now, expiry_days);
        will.guardian_cancel_vote_weight = tally_weight;
        will.guardian_cancel_votes = tally_count;
        // Recount from the current guardian list (#453); see `guardian_trigger`.
        let (votes, vote_weight) =
            tally_guardian_votes(&env, &will, now, storage::has_guardian_cancel_voted);
        will.guardian_cancel_votes = votes;
        will.guardian_cancel_vote_weight = vote_weight;
        // Same recount-from-live-records rule as `guardian_trigger`: an expired
        // cancel vote is replaced, not accumulated, so one guardian cannot reach
        // the cancel threshold alone by voting once per grace period (#372).
        let (live_weight, live_votes) =
            storage::recount_guardian_cancel_votes(&env, &will, now, expiry_days);
        will.guardian_cancel_vote_weight = live_weight;
        will.guardian_cancel_votes = live_votes;
        storage::save_will(&env, &will);

        events::guardian_cancel_voted(
            &env,
            will_id,
            &guardian,
            weight,
            will.guardian_cancel_vote_weight,
        );

        if will.guardian_cancel_vote_weight >= will.guardian_threshold {
            // Quorum reached: reset the will to Active, mirror emergency_checkin.
            storage::reset_guardian_cancel_votes(&env, &will);
            // Also clear any in-progress release votes so the release cycle
            // starts clean if the will is ever triggered again.
            storage::reset_guardian_votes(&env, &will);

            will.status = WillStatus::Active;
            will.trigger_time = None;
            will.last_checkin = now;
            will.guardian_vote_weight = 0;
            will.guardian_votes = 0;
            will.guardian_cancel_vote_weight = 0;
            will.guardian_cancel_votes = 0;
            storage::save_will(&env, &will);

            storage::unindex_triggered_will(&env, will_id);

            record_transition(
                &env,
                will_id,
                WillStatus::Triggered,
                WillStatus::Active,
                &guardian,
                symbol_short!("gcancel"),
            );

            let next_deadline = now + will.checkin_period_days * SECONDS_PER_DAY;
            events::guardian_cancelled_trigger(&env, will_id, &guardian, next_deadline);
        }
    }

    /// Allows a named guardian to accept their role on a will.
    ///
    /// A guardian must call this to explicitly accept their role before they can
    /// vote via [`guardian_trigger`]. This ensures guardians actively consent
    /// before they can force an early release of funds.
    ///
    /// # Parameters
    /// - `will_id`: the will to accept guardianship for
    /// - `guardian`: the guardian address accepting the role; must authorize
    ///
    /// # Events
    /// Emits [`events::guardian_accepted_role`] (topic `"gaccept"`) with the
    /// accepting guardian as the payload, after the consent change is saved.
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if the will does not exist.
    /// - [`WillError::NotGuardian`] if `guardian` is not named on this will.
    /// - [`WillError::WillNotActive`] if the will is in a terminal status
    ///   (`Released`, `Cancelled` or `Settled`).
    /// - [`WillError::InvalidConsentTransition`] if the guardian already
    ///   `Rejected` their role, which is irrevocable.
    pub fn accept_guardian_role(env: Env, will_id: u64, guardian: Address) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);
        assert_consent_changeable(&env, &will);

        let mut found = false;
        let mut updated_guardians: Vec<Guardian> = Vec::new(&env);
        for g in will.guardians.iter() {
            if g.address == guardian {
                // `Rejected` is terminal: a guardian who declined must be
                // re-appointed through `update_guardians` (which resets the
                // list to `Pending`) rather than flipping consent back (#374).
                if g.consent == GuardianConsent::Rejected {
                    panic_with_error!(&env, WillError::InvalidConsentTransition);
                }
                // Already accepted: nothing to write, so skip the storage
                // update entirely rather than rewriting an identical entry.
                if g.consent == GuardianConsent::Accepted {
                    return;
                }
                updated_guardians.push_back(Guardian {
                    address: g.address.clone(),
                    weight: g.weight,
                    consent: GuardianConsent::Accepted,
                });
                found = true;
            } else {
                updated_guardians.push_back(g.clone());
            }
        }

        if !found {
            panic_with_error!(&env, WillError::NotGuardian);
        }

        will.guardians = updated_guardians;
        storage::save_will(&env, &will);

        events::guardian_accepted_role(&env, will_id, &guardian);
    }

    /// Allows a named guardian to reject their role on a will.
    ///
    /// A guardian can reject their role to prevent themselves from voting via
    /// [`guardian_trigger`]. Once rejected, the guardian cannot vote and
    /// [`accept_guardian_role`] can no longer undo it: `Rejected` is terminal
    /// for that guardian entry. The only way back to `Pending` is the owner
    /// re-appointing them through `update_guardians` / `update_guardians_weighted`.
    ///
    /// # Consent state machine
    ///
    /// | from \ to | `Pending` | `Accepted` | `Rejected` |
    /// |-----------|-----------|------------|------------|
    /// | `Pending`  | — | `Accepted` | `Rejected` |
    /// | `Accepted` | — | no-op     | `Rejected` |
    /// | `Rejected` | — | `InvalidConsentTransition` | no-op |
    ///
    /// `Rejected` is terminal: a guardian who declined is not asked again, so
    /// silently flipping them back to `Accepted` would resurrect a decision
    /// they already made.
    ///
    /// # Parameters
    /// - `will_id`: the will to reject guardianship for
    /// - `guardian`: the guardian address rejecting the role; must authorize
    ///
    /// If the guardian already cast a trigger or cancel vote in the current
    /// cycle, that vote is withdrawn: the stored vote record is removed and
    /// its weight is deducted from `guardian_vote_weight` /
    /// `guardian_cancel_vote_weight` so a withdrawn guardian can no longer
    /// contribute toward quorum (#374).
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if the will does not exist.
    /// - [`WillError::NotGuardian`] if `guardian` is not named on this will.
    /// - [`WillError::WillNotActive`] if the will is in a terminal status
    ///   (`Released`, `Cancelled` or `Settled`).
    pub fn reject_guardian_role(env: Env, will_id: u64, guardian: Address) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);
        assert_consent_changeable(&env, &will);

        let now = env.ledger().timestamp();
        let mut found = false;
        let mut updated_guardians: Vec<Guardian> = Vec::new(&env);
        for g in will.guardians.iter() {
            if g.address == guardian {
                if g.consent == GuardianConsent::Rejected {
                    // Already rejected: nothing to change, so skip the write.
                    return;
                }
                updated_guardians.push_back(Guardian {
                    address: g.address.clone(),
                    weight: g.weight,
                    consent: GuardianConsent::Rejected,
                });
                found = true;
            } else {
                updated_guardians.push_back(g.clone());
            }
        }

        if !found {
            panic_with_error!(&env, WillError::NotGuardian);
        }

        // Withdraw any vote the guardian had already cast in this cycle, so a
        // rejected guardian stops contributing weight toward quorum (#374).
        let weight = g_weight(&will, &guardian);
        if storage::has_guardian_voted(&env, will_id, &guardian, now, will.grace_period_days) {
            storage::clear_guardian_vote(&env, will_id, &guardian);
            will.guardian_vote_weight = will.guardian_vote_weight.saturating_sub(weight);
            will.guardian_votes = will.guardian_votes.saturating_sub(1);
        }
        if storage::has_guardian_cancel_voted(&env, will_id, &guardian, now, will.grace_period_days)
        {
            storage::clear_guardian_cancel_vote(&env, will_id, &guardian);
            will.guardian_cancel_vote_weight =
                will.guardian_cancel_vote_weight.saturating_sub(weight);
            will.guardian_cancel_votes = will.guardian_cancel_votes.saturating_sub(1);
        }

        will.guardians = updated_guardians;
        storage::save_will(&env, &will);

        events::guardian_rejected_role(&env, will_id, &guardian);
    }

    // ── #21: Will cloning / templates ────────────────────────────────────

    /// Clones an existing will's configuration into a new will with fresh
    /// token balances.
    ///
    /// Copies beneficiaries, guardian list, check-in period, and grace period
    /// from the source will. The new will gets a fresh balance (funded by the
    /// `tokens` parameter), a new id, and starts with `Active` status and a
    /// fresh check-in deadline.
    ///
    /// The guardian list is copied with every consent reset to
    /// [`GuardianConsent::Pending`] (addresses and vote weights are preserved),
    /// exactly like [`create_will`]: a guardian must be asked about the clone
    /// before they can vote on it, so consent recorded on the source will does
    /// not carry over (#375).
    ///
    /// The source will must be `Active` or `Triggered`. Cloning is
    /// deliberately *not* allowed from a `Cancelled`, `Released`, or
    /// `Settled` source: an owner who let a will resolve to one of those
    /// terminal states may have done so specifically because the
    /// beneficiary/guardian configuration no longer reflects their wishes
    /// (e.g. cancelling because a beneficiary is no longer trusted), and
    /// silently letting that configuration be reused as a template for a
    /// brand-new, separately-funded will would be surprising. Callers who
    /// want to reuse an old configuration from a terminal will must supply
    /// the beneficiary/guardian lists to [`create_will`] directly, which
    /// forces a conscious re-entry of the data instead of an implicit copy.
    /// The owner must authorize this call.
    ///
    /// # Parameters
    /// - `source_will_id`: the id of the will to clone configuration from.
    /// - `owner`: the address creating the new will.
    /// - `tokens`: token balances to lock in the new will (same format as
    ///   [`create_will`]).
    ///
    /// # Returns
    /// The newly allocated will id.
    ///
    /// The clone's audit trail is seeded with a `create` transition exactly like
    /// [`create_will`], so `get_will_history` starts with the same entry
    /// regardless of which creation path produced the will.
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if the source will does not exist.
    /// - [`WillError::WillNotActive`] if the source will is not `Active` or
    ///   `Triggered`.
    /// - [`WillError::ZeroAmount`] if any token amount is not positive.
    /// - [`WillError::InvalidTokenCount`] if the token list is empty or too large.
    /// - [`WillError::FixedAmountExceedsBalance`] if the source's
    ///   `Allocation::FixedAmount` beneficiaries no longer fit the new balance.
    #[allow(clippy::too_many_arguments)]
    pub fn clone_will(
        env: Env,
        source_will_id: u64,
        owner: Address,
        tokens: Vec<(Address, i128)>,
    ) -> u64 {
        owner.require_auth();

        if tokens.is_empty() || tokens.len() > MAX_TOKENS {
            panic_with_error!(&env, WillError::InvalidTokenCount);
        }

        let source = load_will(&env, source_will_id);
        if source.status != WillStatus::Active && source.status != WillStatus::Triggered {
            panic_with_error!(&env, WillError::WillNotActive);
        }

        // Re-run the same owner-not-a-guardian / no-duplicate-guardian check
        // every other will-creation path runs. Safe today only because
        // `source.guardians` was already validated when the source will was
        // created; re-checking here means a future tightening of
        // assert_valid_guardians's rules can't silently skip wills created
        // via clone_will (#262).
        let mut source_guardian_addresses: Vec<Address> = Vec::new(&env);
        for guardian in source.guardians.iter() {
            source_guardian_addresses.push_back(guardian.address.clone());
        }
        assert_valid_guardians(&env, &owner, &source_guardian_addresses);

        // Build balances map and transfer tokens from the owner.
        let mut balances: Map<Address, i128> = Map::new(&env);
        for (token_addr, amount) in tokens.iter() {
            if amount <= 0 {
                panic_with_error!(&env, WillError::ZeroAmount);
            }
            token::Client::new(&env, &token_addr).transfer(
                &owner,
                &env.current_contract_address(),
                &amount,
            );
            let prev = balances.get(token_addr.clone()).unwrap_or(0);
            balances.set(token_addr, prev + amount);
        }

        // Re-validate `FixedAmount` beneficiaries against the clone's new
        // balance (#239): funding a clone with less than the original
        // fixed-amount commitments must fail loudly here rather than
        // silently under-paying at distribute() time. Fixed amounts are
        // denominated in the clone's primary token, the first entry of
        // `tokens` (#384).
        let (primary_token, primary_amount) = tokens.get_unchecked(0);
        assert_valid_allocations(
            &env,
            &source.beneficiaries,
            primary_token_balance(&balances, &primary_token),
        );

        let will_id = storage::next_will_id(&env);
        let now = env.ledger().timestamp();
        let token_count = balances.len();

        for beneficiary in source.beneficiaries.iter() {
            storage::index_by_beneficiary(&env, &beneficiary.address, will_id);
        }

        let will = Will {
            id: will_id,
            owner: owner.clone(),
            balances,
            token: primary_token,
            is_native: false,
            balance: primary_amount,
            beneficiaries: source.beneficiaries.clone(),
            hashed_beneficiaries: Vec::new(&env),
            checkin_period_days: source.checkin_period_days,
            grace_period_days: source.grace_period_days,
            last_checkin: now,
            trigger_time: None,
            confirmation_deadline: None,
            status: WillStatus::Active,
            guardians: reset_guardian_consent(&env, &source.guardians),
            guardian_vote_weight: 0,
            guardian_votes: 0,
            guardian_cancel_vote_weight: 0,
            guardian_cancel_votes: 0,
            guardian_threshold: source.guardian_threshold,
            guardian_list_updated_at: now,
            schema_version: CURRENT_SCHEMA_VERSION,
            keeper_bounty_bps: source.keeper_bounty_bps,
            delegate: None,
        };
        storage::save_will(&env, &will);
        storage::index_by_owner(&env, &owner, will_id);
        storage::increment_active_will_count(&env);
        storage::adjust_locked_for_balances(&env, &will.balances, 1);

        // Seed the audit trail with the same `create` transition every other
        // creation path records, so `get_will_history` starts with one entry
        // regardless of how the will came into being (#376).
        record_transition(
            &env,
            will_id,
            WillStatus::Active,
            WillStatus::Active,
            &owner,
            symbol_short!("create"),
        );

        events::will_created(
            &env,
            will_id,
            &owner,
            token_count,
            &will.beneficiaries,
            now + source.checkin_period_days * SECONDS_PER_DAY,
        );
        events::will_cloned(&env, source_will_id, will_id, &owner);

        will_id
    }

    // ── #19: Batch will creation ─────────────────────────────────────────

    /// Creates multiple wills in a single transaction.
    ///
    /// Each entry in `will_specs` is a tuple of:
    /// - `tokens`: `(token_address, amount)` pairs to lock.
    /// - `beneficiaries`: beneficiary list with basis-point shares.
    /// - `checkin_period_days`: check-in period in days.
    /// - `grace_period_days`: grace period in days.
    /// - `guardians`: guardian address list.
    ///
    /// The owner must authorize the entire call. All wills are created under
    /// the same `owner`.
    ///
    /// Each will's audit trail is seeded with a `create` transition exactly
    /// like [`create_will`], so `get_will_history` starts with the same
    /// entry regardless of which creation path produced the will.
    ///
    /// # Returns
    /// A `Vec<u64>` of newly allocated will ids, one per spec.
    ///
    /// # Panics
    /// - [`WillError::TooManyBeneficiaries`] if the batch is empty or exceeds
    ///   [`BATCH_MAX`], or if any individual spec violates the
    ///   beneficiary/guardian caps.
    /// - [`WillError::InvalidTokenCount`] if any individual spec's token list
    ///   is empty or exceeds `MAX_TOKENS`.
    /// - Any error that [`create_will`] would panic with for an individual spec.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn batch_create_wills(
        env: Env,
        owner: Address,
        will_specs: Vec<(
            Vec<(Address, i128)>,
            Vec<Beneficiary>,
            u64,
            u64,
            Vec<Address>,
            u32,
        )>,
    ) -> Vec<u64> {
        owner.require_auth();

        if will_specs.is_empty() || will_specs.len() > BATCH_MAX {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }

        let mut ids = Vec::new(&env);
        for spec in will_specs.iter() {
            let (
                tokens,
                beneficiaries,
                checkin_period_days,
                grace_period_days,
                guardians,
                guardian_threshold,
            ) = spec;

            // Inline the validation + creation logic (mirrors create_will)
            // to avoid re-authorizing per will.
            if tokens.is_empty() || tokens.len() > MAX_TOKENS {
                panic_with_error!(&env, WillError::InvalidTokenCount);
            }
            // Mirror `create_will`'s duplicate-token rejection so a batch spec
            // can never produce a will whose `balance` mirror disagrees with
            // its `balances` map (#350).
            let mut seen_tokens: Vec<Address> = Vec::new(&env);
            for (token_addr, _) in tokens.iter() {
                if seen_tokens.contains(&token_addr) {
                    panic_with_error!(&env, WillError::DuplicateToken);
                }
                seen_tokens.push_back(token_addr);
            }
            assert_beneficiary_count(&env, &beneficiaries);
            assert_valid_guardians(&env, &owner, &guardians);
            assert_valid_periods(&env, checkin_period_days, grace_period_days);
            // Mirror the `create_will` normalisation: an empty guardian list
            // ignores the supplied threshold and stores 0, since a threshold
            // with no guardians describes a quorum that can never be reached
            // and would later block `update_guardians` (#363). A non-empty list
            // requires threshold in 1..=guardians.len().
            let guardian_threshold = if guardians.is_empty() {
                0
            } else {
                let threshold_range = 1..=guardians.len();
                if !threshold_range.contains(&guardian_threshold) {
                    panic_with_error!(&env, WillError::InvalidGuardianThreshold);
                }
                guardian_threshold
            };

            let mut balances: Map<Address, i128> = Map::new(&env);
            for (token_addr, amount) in tokens.iter() {
                if amount <= 0 {
                    panic_with_error!(&env, WillError::ZeroAmount);
                }
                token::Client::new(&env, &token_addr).transfer(
                    &owner,
                    &env.current_contract_address(),
                    &amount,
                );
                let prev = balances.get(token_addr.clone()).unwrap_or(0);
                balances.set(token_addr, prev + amount);
            }

            // Fixed amounts are denominated in the primary token, the first
            // entry of `tokens` (#384).
            let (primary_token, _) = tokens.get_unchecked(0);
            assert_valid_allocations(
                &env,
                &beneficiaries,
                primary_token_balance(&balances, &primary_token),
            );

            let mut guardian_structs: Vec<Guardian> = Vec::new(&env);
            for addr in guardians.iter() {
                guardian_structs.push_back(Guardian {
                    address: addr,
                    weight: 1,
                    consent: GuardianConsent::Pending,
                });
            }

            let will_id = storage::next_will_id(&env);
            let now = env.ledger().timestamp();
            let token_count = balances.len();

            for beneficiary in beneficiaries.iter() {
                storage::index_by_beneficiary(&env, &beneficiary.address, will_id);
            }

            let (primary_token, _) = tokens.get_unchecked(0);
            let primary_amount = primary_token_balance(&balances, &primary_token);

            let will = Will {
                id: will_id,
                owner: owner.clone(),
                balances,
                token: primary_token,
                is_native: false,
                balance: primary_amount,
                beneficiaries,
                hashed_beneficiaries: Vec::new(&env),
                checkin_period_days,
                grace_period_days,
                last_checkin: now,
                trigger_time: None,
                confirmation_deadline: None,
                status: WillStatus::Active,
                guardians: guardian_structs,
                guardian_vote_weight: 0,
                guardian_votes: 0,
                guardian_cancel_vote_weight: 0,
                guardian_cancel_votes: 0,
                guardian_threshold,
                guardian_list_updated_at: now,
                schema_version: CURRENT_SCHEMA_VERSION,
                keeper_bounty_bps: 0,
                delegate: None,
            };
            storage::save_will(&env, &will);
            storage::index_by_owner(&env, &owner, will_id);
            storage::increment_active_will_count(&env);
            storage::adjust_locked_for_balances(&env, &will.balances, 1);

            record_transition(
                &env,
                will_id,
                WillStatus::Active,
                WillStatus::Active,
                &owner,
                symbol_short!("create"),
            );

            events::will_created(
                &env,
                will_id,
                &owner,
                token_count,
                &will.beneficiaries,
                now + checkin_period_days * SECONDS_PER_DAY,
            );

            ids.push_back(will_id);
        }

        events::batch_created(&env, &owner, &ids);
        ids
    }

    /// Migrates a will to the latest schema version. The owner must authorize
    /// this call. This is an owner-initiated per-will migration that allows
    /// users to opt-in to new contract versions without being forced to do so.
    ///
    /// Runs every step in [`migration::upgrade`] between the will's stored
    /// `schema_version` and [`CURRENT_SCHEMA_VERSION`] and persists the
    /// result. Wills written with an older layout are still readable before
    /// migrating (see [`migration::decode_will`]); migrating rewrites them in
    /// the current layout.
    ///
    /// # Current behavior (v0 → v1)
    /// Sets the schema_version field to 1.
    /// # Current behavior is a placeholder
    /// [`CURRENT_SCHEMA_VERSION`] (defined in [`storage`] and re-exported at
    /// the crate root) is `1`, and every will created by this
    /// contract version is already stamped with `schema_version:
    /// CURRENT_SCHEMA_VERSION` at creation time (see [`create_will`] and
    /// [`batch_create_wills`]). Because of that, `old_version >=
    /// CURRENT_SCHEMA_VERSION` is true for any will this contract could
    /// actually produce, so the early return below is taken unconditionally
    /// and the body never runs in practice — there is no real migration
    /// wired up yet. The `will.schema_version = CURRENT_SCHEMA_VERSION` line
    /// and the emitted `will_migrated` event exist only as the scaffold a
    /// future schema bump will hang real field transformations off of; a
    /// will could only reach this function with `old_version <
    /// CURRENT_SCHEMA_VERSION` after a future contract upgrade raises
    /// [`storage::CURRENT_SCHEMA_VERSION`] and defines an actual v1 → v2
    /// (or later) transformation here. Because that constant is the single
    /// source of truth, the entry point and the storage layer can never
    /// disagree about the current version.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotFound`] if the will does not exist.
    pub fn migrate_will(env: Env, will_id: u64, owner: Address) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);

        let old_version = will.schema_version;

        // Check if already on current version
        if old_version >= CURRENT_SCHEMA_VERSION {
            return;
        }

        // Apply version-specific migrations in sequence
        will = migration::upgrade(&env, will);

        storage::save_will(&env, &will);
        events::will_migrated(&env, will_id, &owner, old_version, CURRENT_SCHEMA_VERSION);
    }

    /// Merges two active wills owned by the same address into a single will.
    ///
    /// The merge policy is:
    /// - The surviving will (will_id_a) receives the combined balance.
    /// - Beneficiaries from both wills are merged, with percentages recalculated
    ///   proportionally based on the combined balance. If a beneficiary appears
    ///   in both wills, their percentages are summed first, then recalculated.
    /// - Guardians are combined by **address** (#379). An address named on both
    ///   wills yields a single entry: its weight becomes the greater of the two
    ///   (so the surviving will's already-validated `guardian_threshold` stays
    ///   reachable, and the weight is still counted exactly once toward quorum),
    ///   and its consent becomes the more advanced of the two, ranked `Accepted`
    ///   > `Pending` > `Rejected`. A guardian `Rejected` on both wills stays
    ///   `Rejected` and cannot vote until the owner re-appoints them through
    ///   `update_guardians`.
    /// - Check-in period: use the minimum (most conservative).
    /// - Grace period: use the maximum (most conservative).
    /// - The consumed will (will_id_b) is marked as Cancelled with zero balance.
    ///
    /// Both wills record a `merge` transition: the survivor as `Active` ->
    /// `Active` (a merge rewrites its balance, beneficiaries, guardians and
    /// periods) and the consumed will as `Active` -> `Cancelled`, so
    /// `get_will_history` shows why its balance moved (#381).
    ///
    /// # Parameters
    /// - `owner`: the owner of both wills; must authorize this call.
    /// - `will_id_a`: the will that survives and receives the merged state.
    /// - `will_id_b`: the will that is consumed (marked Cancelled).
    ///
    /// # Panics
    /// - [`WillError::NotSameOwner`] if the two wills have different owners.
    /// - [`WillError::NotOwner`] if `owner` does not own both wills.
    /// - [`WillError::WillNotBothActive`] if either will is not in `Active` status.
    /// - [`WillError::SameWillId`] if `will_id_a` equals `will_id_b`.
    /// - [`WillError::MergeWithHashedBeneficiaries`] if either will still has a
    ///   hashed beneficiary that has not revealed and claimed. Only *visible*
    ///   beneficiaries are merged, so a commitment on the consumed will would
    ///   otherwise be dropped along with its committed percentage while its
    ///   balance moved to the survivor — stranding that beneficiary's claim —
    ///   and the survivor's percentages would then apply to the larger combined
    ///   balance (#380). The owner must let every commitment reveal and claim
    ///   first (or cancel that will) and then merge. Hashed entries that have
    ///   already claimed are inert and do not block a merge.
    /// - [`WillError::MergeWouldExceedLimits`] if merging would exceed MAX_BENEFICIARIES or MAX_GUARDIANS limits.
    /// - [`WillError::InvalidPercentages`] if recalculating percentages fails.
    pub fn merge_wills(env: Env, owner: Address, will_id_a: u64, will_id_b: u64) {
        owner.require_auth();

        if will_id_a == will_id_b {
            panic_with_error!(&env, WillError::SameWillId);
        }

        // Both wills must belong to the same owner, and that owner must be the
        // authorized caller. Checking the pair first means a caller who owns
        // only one of the two ids learns nothing about the other will beyond
        // "not yours", and can never fold their will into someone else's.
        let mut will_a = load_will(&env, will_id_a);
        let mut will_b = load_will(&env, will_id_b);
        if will_a.owner != will_b.owner {
            panic_with_error!(&env, WillError::NotSameOwner);
        }
        if will_a.owner != owner {
            panic_with_error!(&env, WillError::NotOwner);
        }
        let mut will_a = load_owned(&env, will_id_a, &owner);
        let mut will_b = load_will(&env, will_id_b);
        if will_b.owner != owner {
            panic_with_error!(&env, WillError::NotSameOwner);
        }

        assert_status(
            &env,
            &will_a,
            WillStatus::Active,
            WillError::WillNotBothActive,
        );
        assert_status(
            &env,
            &will_b,
            WillStatus::Active,
            WillError::WillNotBothActive,
        );

        if will_a.token != will_b.token {
            panic_with_error!(&env, WillError::PrimaryTokenMismatch);
        }

        // A merge cannot carry hashed beneficiaries across. `merge_beneficiaries`
        // only merges *visible* beneficiaries, so the consumed will's
        // commitments and their committed percentages would be dropped while
        // its balance moved to the survivor: those beneficiaries would lose
        // their claim with no error, and the survivor's existing percentages
        // would apply to the larger combined balance (#380).
        //
        // Rather than silently re-basing a commitment's percentage against a
        // balance the beneficiary never agreed to, the merge is rejected while
        // either will still has an unrevealed hashed beneficiary. The owner
        // must let every commitment reveal and claim first (or cancel the will)
        // and then merge. Hashed entries that have already been claimed are
        // inert — their share has been paid out and only the flag remains — so
        // they do not block a merge.
        if unclaimed_hashed_bps(&will_a.hashed_beneficiaries) > 0
            || unclaimed_hashed_bps(&will_b.hashed_beneficiaries) > 0
        {
            panic_with_error!(&env, WillError::MergeWithHashedBeneficiaries);
        }

        // Merge beneficiaries with proportional recalculation
        let merged_beneficiaries = merge_beneficiaries(&env, &will_a, &will_b);

        if merged_beneficiaries.len() > MAX_BENEFICIARIES {
            log!(
                &env,
                "merged will would have {} beneficiaries; MAX_BENEFICIARIES is {}",
                merged_beneficiaries.len(),
                MAX_BENEFICIARIES
            );
            panic_with_error!(&env, WillError::MergeWouldExceedLimits);
        }

        // Merge guardians, matching on **address** only. Comparing whole
        // `Guardian` structs treated the same address as two entries whenever
        // the two wills recorded a different weight or consent for it, so a
        // guardian listed in both wills was appended twice — breaking the
        // no-duplicate-guardian rule `assert_valid_guardians` enforces on
        // every creation path and double-counting that guardian's weight
        // toward quorum (#379).
        //
        // When both wills name the same address, the entries are combined by
        // this documented rule:
        //
        // - **weight**: the greater of the two. Each will's threshold was
        //   validated against that will's own weights, so dropping either
        //   one could leave the surviving will's `guardian_threshold`
        //   unreachable; the larger weight keeps the surviving threshold
        //   satisfiable and is still a single entry, so the guardian's
        //   weight is counted exactly once.
        // - **consent**: the more advanced of the two, ranked `Accepted` >
        //   `Pending` > `Rejected`. A guardian who accepted the role on
        //   either will has consented; a guardian who is `Rejected` on both
        //   stays `Rejected`, which is terminal for them and keeps them out
        //   of the vote.
        let mut merged_guardians: Vec<Guardian> = Vec::new(&env);
        for guardian in will_a.guardians.iter() {
            merged_guardians.push_back(guardian);
        }
        for guardian in will_b.guardians.iter() {
            let mut found = false;
            for i in 0..merged_guardians.len() {
                let existing = merged_guardians.get_unchecked(i);
                if existing.address == guardian.address {
                    found = true;
                    let merged = Guardian {
                        address: existing.address.clone(),
                        weight: existing.weight.max(guardian.weight),
                        consent: more_advanced_consent(existing.consent, guardian.consent),
                    };
                    merged_guardians.set(i, merged);
                    break;
                }
            }
            if !found {
                merged_guardians.push_back(guardian);
            }
        }

        if merged_guardians.len() > MAX_GUARDIANS {
            panic_with_error!(&env, WillError::MergeWouldExceedLimits);
        }

        // Merge parameters: use minimum check-in period, maximum grace period
        let merged_checkin_period = if will_a.checkin_period_days < will_b.checkin_period_days {
            will_a.checkin_period_days
        } else {
            will_b.checkin_period_days
        };

        let merged_grace_period = if will_a.grace_period_days > will_b.grace_period_days {
            will_a.grace_period_days
        } else {
            will_b.grace_period_days
        };

        // Combine balances (both the multi-token map and the legacy
        // primary-token mirror).
        let combined_balance = will_a.balance + will_b.balance;
        let mut combined_balances = will_a.balances.clone();
        for (token_addr, amount) in will_b.balances.iter() {
            let prev = combined_balances.get(token_addr.clone()).unwrap_or(0);
            combined_balances.set(token_addr, prev + amount);
        }

        // Clear persistent GuardianVote/GuardianCancelVote entries for both
        // wills' *pre-merge* guardian lists before the in-memory counters are
        // zeroed below — otherwise the vote rows are orphaned in storage
        // forever and could be miscounted if a guardian address is reused.
        storage::reset_guardian_votes(&env, &will_a);
        storage::reset_guardian_cancel_votes(&env, &will_a);
        storage::reset_guardian_votes(&env, &will_b);
        storage::reset_guardian_cancel_votes(&env, &will_b);

        // Update will_a with merged state
        will_a.beneficiaries = merged_beneficiaries;
        will_a.guardians = merged_guardians;
        will_a.checkin_period_days = merged_checkin_period;
        will_a.grace_period_days = merged_grace_period;
        will_a.balances = combined_balances;
        will_a.balance = combined_balance;
        will_a.guardian_votes = 0;
        will_a.guardian_cancel_votes = 0;

        // Remove old beneficiary indexes for will_b
        for beneficiary in will_b.beneficiaries.iter() {
            storage::remove_beneficiary_index(&env, &beneficiary.address, will_id_b);
        }

        // Mark will_b as cancelled with zero balance
        will_b.balances = Map::new(&env);
        will_b.balance = 0;
        will_b.status = WillStatus::Cancelled;
        will_b.guardian_votes = 0;
        will_b.guardian_cancel_votes = 0;

        // Decrement active will count since will_b is now cancelled
        storage::decrement_active_will_count(&env);

        // Drop will_b from the owner index now that it is a terminal,
        // zeroed-out placeholder — otherwise get_wills_by_owner keeps
        // surfacing it alongside the surviving will_a indefinitely.
        storage::remove_owner_index(&env, &owner, will_id_b);

        // Save both wills
        storage::save_will(&env, &will_a);
        storage::save_will(&env, &will_b);

        // Record the status change on the consumed will (#381). The consumed
        // will really does move `Active` → `Cancelled` here, so its audit
        // trail must say so: without this entry `get_will_history` showed only
        // the will's `create` transition, making a merge indistinguishable
        // from a will that had simply been left alone, and hiding the reason
        // the balance moved to another will. `cancel_will`, `trigger_will`,
        // `release_inheritance` and the guardian paths all record theirs.
        record_transition(
            &env,
            will_id_b,
            WillStatus::Active,
            WillStatus::Cancelled,
            &owner,
            symbol_short!("merge"),
        );

        // The survivor keeps its `Active` status, so this is an `Active` →
        // `Active` entry of the same kind `create_will` and `split_will`
        // record. A merge rewrites the survivor's balance, beneficiaries,
        // guardians and periods, and those mutations are what the trail exists
        // to describe — without an entry, the surviving will's history would
        // jump straight from `create` to its eventual release with no hint
        // that a second will's funds and beneficiaries were folded into it.
        record_transition(
            &env,
            will_id_a,
            WillStatus::Active,
            WillStatus::Active,
            &owner,
            symbol_short!("merge"),
        );

        // Update beneficiary indexes for will_a
        for beneficiary in will_a.beneficiaries.iter() {
            storage::index_by_beneficiary(&env, &beneficiary.address, will_id_a);
        }

        events::wills_merged(
            &env,
            will_id_a,
            will_id_b,
            &owner,
            combined_balance,
            &will_a.beneficiaries,
        );
    }

    /// Returns the audit trail for `will_id`, recording every status
    /// transition since creation.
    ///
    /// # Bounded length
    ///
    /// The retained trail holds at most [`storage::MAX_HISTORY_ENTRIES`]
    /// transitions. Once a will exceeds that many — a long-lived will cycling
    /// `Active` → `Triggered` → `Active` through repeated emergency check-ins —
    /// the **oldest** entry is dropped to make room, so this always returns the
    /// most recent transitions, oldest-first. The cap keeps the persistent
    /// `WillHistory` entry inside Soroban's per-entry size limit and keeps this
    /// read bounded (#392).
    ///
    /// For the full, untrimmed history, follow the off-chain event log: every
    /// state-mutating entry point publishes an event, and that log is never
    /// trimmed. Callers who want to walk the retained trail in bounded slices
    /// should prefer [`WillContract::get_will_history_page`].
    pub fn get_will_history(env: Env, will_id: u64) -> Vec<WillStatusTransition> {
        storage::get_history(&env, will_id)
    }

    /// Returns a bounded page of `will_id`'s audit trail, oldest-first.
    ///
    /// The paged counterpart to [`WillContract::get_will_history`], for callers
    /// that would rather not pull the whole retained trail in one call. The
    /// trail is itself capped at [`storage::MAX_HISTORY_ENTRIES`] transitions
    /// (#392); this bounds the *per-call* cost on top of that.
    ///
    /// # Parameters
    /// - `will_id`: the will whose trail to read.
    /// - `cursor`: optional zero-based offset into the trail. Pass `None` or `0`
    ///   for the first page.
    /// - `limit`: maximum number of transitions to return. Capped at
    ///   [`storage::MAX_PAGE_SIZE`].
    ///
    /// # Pagination
    /// 1. Call with `cursor=None, limit=N`.
    /// 2. If the page has `N` entries, call again with `cursor = offset + N`.
    /// 3. Repeat until a page comes back shorter than `N`.
    ///
    /// The cursor is positional, not keyed. It is stable for the duration of a
    /// walk as long as no transition is appended past the cursor, but a
    /// concurrent write that trips the history cap trims the front of the
    /// trail and shifts every earlier offset — restart the walk in that case.
    pub fn get_will_history_page(
        env: Env,
        will_id: u64,
        cursor: Option<u32>,
        limit: u32,
    ) -> Vec<WillStatusTransition> {
        storage::paginate_history(&env, will_id, cursor, limit)
    }

    /// Archives a Released or Cancelled will, removing it from active
    /// storage and indexes so it no longer appears in owner/beneficiary
    /// queries. The archived will data will eventually be garbage-collected
    /// by Soroban's state archival system.
    ///
    /// **Callable by anyone**: no `require_auth` is enforced. Once a will
    /// reaches a terminal state (`Released` or `Cancelled`) any party may
    /// call this function to reclaim on-chain storage and reduce ledger-rent
    /// costs. The design is intentional — a will's final asset distributions
    /// are already complete before this point — but it creates an observable
    /// race condition described below.
    ///
    /// # What archival removes
    ///
    /// Beyond the will entry and the owner/beneficiary/Triggered indexes,
    /// archival also drops the will's on-chain `WillHistory` entry and every
    /// `GuardianVote` / `GuardianCancelVote` entry belonging to its guardians
    /// (#393). Those keys are only ever read to describe a *live* will, so
    /// leaving them behind would strand ledger state — paid for out of the
    /// protocol's rent — for entries no query can resolve. See
    /// [`storage::archive_will`] for the full reasoning.
    ///
    /// **Clients must not treat [`WillContract::get_will_history`] as a
    /// post-archival recovery path** — it returns an empty trail for an archived
    /// will. Use the off-chain event log, which is never trimmed and is the
    /// durable audit record.
    ///
    /// # Race condition: permissionless archival and `WillNotFound` ambiguity
    ///
    /// Because any account can call `archive_will` at any time after a will
    /// is released, a client that reads a will's status and then queries it
    /// again a moment later may observe the will disappear between the two
    /// calls. Specifically:
    ///
    /// 1. Client A reads will `42` and sees `WillStatus::Released`.
    /// 2. Account B (anyone) calls `archive_will(42)`.
    /// 3. Client A calls `get_will(42)` — it now panics with
    ///    [`WillError::WillNotFound`].
    ///
    /// This is compounded by the limitation documented in
    /// [`storage::load_will`] (issue #166): Soroban's persistent-storage API
    /// cannot distinguish a key that **never existed** from a key that was
    /// **explicitly archived by this function** or one that was
    /// **TTL-archived by the network** after its storage lease expired.
    /// All three cases surface as the identical [`WillError::WillNotFound`]
    /// panic to the caller.
    ///
    /// ## Recommended client-side handling
    ///
    /// Clients should treat `WillNotFound` on a `will_id` that was previously
    /// known to exist (or that appears in an off-chain index) as one of three
    /// possible states, in order of likelihood:
    ///
    /// 1. **Explicitly archived** — the will completed its lifecycle, funds
    ///    were distributed, and a third party (or the owner) called
    ///    `archive_will`. This is the normal post-release state and requires
    ///    no recovery. The final state is recoverable from the off-chain
    ///    event log, which archival does not touch.
    /// 2. **Network TTL expiry** — the will's persistent entry lapsed.
    ///    Terminal wills stop renewing their TTL (see `storage::save_will`),
    ///    so Released/Cancelled wills gradually expire. The entry can be
    ///    restored by a network-level state-restore transaction; until then
    ///    the contract cannot serve it.
    /// 3. **Never created** — the id was never allocated. Clients can rule
    ///    this out by confirming the id is below the current `NextWillId`
    ///    counter or by checking an off-chain event log.
    ///
    /// A dedicated `WillArchived` error code that would let callers
    /// distinguish case 1 from cases 2 and 3 is deferred: the current
    /// soroban-sdk version does not expose an archived-entry probe, so a
    /// single [`WillError::WillNotFound`] is the only signal available today.
    /// Clients MUST NOT treat `WillNotFound` as proof that a will was never
    /// created or that funds were never distributed.
    ///
    /// Publishes an `archived` event carrying the owner, the archival
    /// timestamp and the reason (the terminal status the will was archived
    /// from), so indexers can explain why the will disappears from queries.
    /// See [`events::will_archived`].
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if no will exists with this id (see
    ///   the ambiguity note above — this error is also returned for wills
    ///   that have already been archived).
    /// - [`WillError::WillNotSettled`] if the will is not `Released` or `Cancelled`.
    pub fn archive_will(env: Env, will_id: u64) {
        let will = load_will(&env, will_id);
        if will.status != WillStatus::Released && will.status != WillStatus::Cancelled {
            panic_with_error!(&env, WillError::WillNotSettled);
        }

        let reason = if will.status == WillStatus::Released {
            symbol_short!("released")
        } else {
            symbol_short!("cancelled")
        };
        storage::archive_will(&env, &will);

        events::will_archived(
            &env,
            will_id,
            &will.owner,
            env.ledger().timestamp(),
            reason,
        );
    }

    // -----------------------------------------------------------------------
    // Issue #45 — split_will
    // -----------------------------------------------------------------------

    /// Carves a subset of beneficiaries and balance out of an existing will
    /// into a new, fully independent child will.
    ///
    /// The original will's balance is reduced by `tokens` and any beneficiaries
    /// present in `beneficiaries_to_split` are removed from it; the new will
    /// receives those beneficiaries with percentages renormalised to 100, and
    /// it starts `Active` with the same check-in period, grace period,
    /// co-owners, and threshold as the original.
    ///
    /// The child inherits the source's guardians and threshold, but every
    /// guardian's consent is reset to [`GuardianConsent::Pending`] (addresses
    /// and vote weights are preserved), exactly like [`create_will`]: a
    /// guardian must be asked about the child will before they can vote on it,
    /// so consent recorded on the source does not carry over (#375).
    ///
    /// # Parameters
    /// - `will_id`: the source will to split from.
    /// - `owner`: must be the primary owner of the source will.
    /// - `beneficiaries_to_split`: subset of beneficiaries to move to the new will.
    ///   Every address must already be a beneficiary of the source will, must
    ///   not be repeated, and the list may hold at most
    ///   [`MAX_BENEFICIARIES`] entries. The `Allocation` on each entry is
    ///   **ignored**: the child's allocation is the source will's entry for
    ///   that address, renormalised to sum to 10,000 bps across the child list.
    ///   The entry exists only to name the addresses to move.
    /// - `tokens`: `(token_address, amount)` pairs to move from the source
    ///   will's balances into the child will, mirroring `create_will`'s
    ///   multi-token API. Each `amount` must be > 0 and no greater than what
    ///   the source will currently holds of that token; duplicate token
    ///   addresses are summed. Every token the split-out beneficiaries need
    ///   access to must be listed here — a token left out of `tokens` stays
    ///   on the source will and is not moved to the child.
    ///
    /// # Returns
    /// The id of the newly created child will.
    ///
    /// The child's audit trail is seeded with a `create` transition exactly like
    /// [`create_will`], so `get_will_history` on the child starts with the
    /// same entry regardless of which creation path produced it. The source
    /// will's own history is untouched: a split is not a status change on it.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] / [`WillError::WillNotActive`]
    /// - [`WillError::InvalidTokenCount`] if `tokens` is empty or exceeds
    ///   `MAX_TOKENS`.
    /// - [`WillError::ZeroAmount`] if any token amount is not positive.
    /// - [`WillError::InsufficientBalance`] if a requested token amount
    ///   exceeds what the source will holds of that token.
    /// - [`WillError::BeneficiaryNotFound`] if an address in
    ///   `beneficiaries_to_split` is not a beneficiary of the source will.
    /// - [`WillError::DuplicateBeneficiary`] if an address appears more than
    ///   once in `beneficiaries_to_split`.
    /// - [`WillError::InvalidSplit`] if `beneficiaries_to_split` is empty or would
    ///   leave the source will with no beneficiaries.
    /// - [`WillError::FixedAmountExceedsBalance`] if either the remaining or
    ///   split beneficiary list has `Allocation::FixedAmount` entries that no
    ///   longer fit the resulting balance.
    pub fn split_will(
        env: Env,
        will_id: u64,
        owner: Address,
        beneficiaries_to_split: Vec<Beneficiary>,
        tokens: Vec<(Address, i128)>,
    ) -> u64 {
        owner.require_auth();
        let mut source = load_owned(&env, will_id, &owner);
        assert_status(&env, &source, WillStatus::Active, WillError::WillNotActive);

        if tokens.is_empty() || tokens.len() > MAX_TOKENS {
            panic_with_error!(&env, WillError::InvalidTokenCount);
        }
        if beneficiaries_to_split.is_empty() {
            panic_with_error!(&env, WillError::InvalidSplit);
        }
        split_uniqueness_check::assert_split_addresses_unique(&env, &beneficiaries_to_split);

        // Accumulate requested amounts per token (duplicates are additive),
        // then verify each against what the source will actually holds.
        let mut child_balances: Map<Address, i128> = Map::new(&env);
        for (token_addr, amt) in tokens.iter() {
            if amt <= 0 {
                panic_with_error!(&env, WillError::ZeroAmount);
            }
            let prev = child_balances.get(token_addr.clone()).unwrap_or(0);
            child_balances.set(token_addr, prev + amt);
        }
        for (token_addr, amt) in child_balances.iter() {
            let held = source.balances.get(token_addr.clone()).unwrap_or(0);
            if amt > held {
                panic_with_error!(&env, WillError::InsufficientBalance);
            }
        }

        // Build the child's beneficiary list from the SOURCE will's entries.
        //
        // `beneficiaries_to_split` is a request to move addresses, not to
        // invent them: the comment above this block used to claim it verified
        // the addresses exist on the source, but it only filtered the source
        // list, so any address and allocation the caller passed in became a
        // beneficiary of the child even when it was never on the source (#377).
        // The allocation used for the child is therefore the source entry's,
        // not the caller's; percentages are renormalised afterwards.
        if beneficiaries_to_split.len() > MAX_BENEFICIARIES {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }

        // A repeated address would silently collapse in the filter below and
        // leave the source and child disagreeing about how many beneficiaries
        // moved, so reject it up front.
        for (i, s) in beneficiaries_to_split.iter().enumerate() {
            for other in beneficiaries_to_split.iter().skip(i + 1) {
                if s.address == other.address {
                    panic_with_error!(&env, WillError::DuplicateBeneficiary);
                }
            }
            if !names_address(&source.beneficiaries, &s.address) {
                panic_with_error!(&env, WillError::BeneficiaryNotFound);
            }
        }

        let mut remaining_beneficiaries: Vec<Beneficiary> = Vec::new(&env);
        let mut source_split: Vec<Beneficiary> = Vec::new(&env);
        for b in source.beneficiaries.iter() {
            let mut being_split = false;
            for s in beneficiaries_to_split.iter() {
                if s.address == b.address {
                    being_split = true;
                    // Take the allocation from the source entry, not the
                    // caller-supplied one.
                    source_split.push_back(b.clone());
                    break;
                }
            }
            if !being_split {
                remaining_beneficiaries.push_back(b.clone());
            }
        }

        // The source will must keep at least one beneficiary.
        if remaining_beneficiaries.is_empty() {
            panic_with_error!(&env, WillError::InvalidSplit);
        }

        // Renormalise each side's `Allocation::Percentage` entries so they sum
        // to 10,000 bps again; `FixedAmount` entries pass through unchanged.
        let normalised_remaining = renormalize_percentages(&env, &remaining_beneficiaries);
        let normalised_split = renormalize_percentages(&env, &source_split);

        // Move every requested token amount out of the source's balances and
        // into the child's. `token`/`balance` mirror the primary (first)
        // token in `tokens`, same as `balances`, which remains the
        // authoritative multi-token ledger.
        for (token_addr, amt) in child_balances.iter() {
            let held = source.balances.get(token_addr.clone()).unwrap_or(0);
            source.balances.set(token_addr, held - amt);
        }
        source.balance = source.balances.get(source.token.clone()).unwrap_or(0);

        let (primary_token, _) = tokens.get_unchecked(0);
        let primary_amount = child_balances.get(primary_token.clone()).unwrap_or(0);
        let child_token_count = child_balances.len();

        // Re-validate `FixedAmount` beneficiaries against each side's new
        // balance before committing anything (#239): a split funded with
        // less than the original fixed-amount commitments must fail loudly
        // here rather than silently under-paying at distribute() time.
        assert_valid_allocations(
            &env,
            &normalised_remaining,
            primary_token_balance(&source.balances, &source.token),
        );
        assert_valid_allocations(
            &env,
            &normalised_split,
            primary_token_balance(&child_balances, &primary_token),
        );

        // Remove split-off beneficiaries from the source index and add them to
        // the child's index.
        for b in beneficiaries_to_split.iter() {
            storage::remove_beneficiary_index(&env, &b.address, will_id);
        }

        source.beneficiaries = normalised_remaining;
        storage::save_will(&env, &source);

        // Create the new child will.
        let new_id = storage::next_will_id(&env);
        let now = env.ledger().timestamp();

        for b in normalised_split.iter() {
            storage::index_by_beneficiary(&env, &b.address, new_id);
        }

        let child = Will {
            id: new_id,
            owner: source.owner.clone(),
            balances: child_balances,
            token: primary_token,
            is_native: false,
            balance: primary_amount,
            beneficiaries: normalised_split.clone(),
            hashed_beneficiaries: Vec::new(&env),
            checkin_period_days: source.checkin_period_days,
            grace_period_days: source.grace_period_days,
            last_checkin: now,
            trigger_time: None,
            confirmation_deadline: None,
            status: WillStatus::Active,
            guardians: reset_guardian_consent(&env, &source.guardians),
            guardian_vote_weight: 0,
            guardian_votes: 0,
            guardian_cancel_vote_weight: 0,
            guardian_cancel_votes: 0,
            guardian_threshold: source.guardian_threshold,
            guardian_list_updated_at: now,
            schema_version: CURRENT_SCHEMA_VERSION,
            keeper_bounty_bps: 0,
            delegate: None,
        };
        storage::save_will(&env, &child);
        storage::index_by_owner(&env, &source.owner, new_id);
        // The child is a new live will. Its balance moved out of the source,
        // so the locked totals are unchanged.
        storage::increment_active_will_count(&env);
        storage::increment_active_will_count(&env);

        // Seed the child's audit trail with the same `create` transition
        // `create_will` and `batch_create_wills` record, so `get_will_history`
        // starts with one entry for the child too (#376). The source keeps its
        // own history; the split is not a status change on the source.
        record_transition(
            &env,
            new_id,
            WillStatus::Active,
            WillStatus::Active,
            &owner,
            symbol_short!("create"),
        );

        events::will_split(&env, will_id, new_id, &owner, primary_amount);
        events::will_created(
            &env,
            new_id,
            &owner,
            child_token_count,
            &normalised_split,
            now + source.checkin_period_days * SECONDS_PER_DAY,
        );

        new_id
    }

    // -----------------------------------------------------------------------
    // Issue #46 — reveal_and_claim
    // -----------------------------------------------------------------------

    /// Registers a hashed beneficiary on an existing active will.
    ///
    /// Only the owner (or co-owner set meeting the threshold) may add hashed
    /// beneficiaries. The combined percentages of `beneficiaries` and
    /// `hashed_beneficiaries` must still sum to 100.
    ///
    /// # Parameters
    /// - `will_id`: the will to add the hashed beneficiary to.
    /// - `owner`: must be the primary owner.
    /// - `commitment`: 32-byte SHA-256 hash of the pre-image
    ///   `beneficiary.to_xdr() || salt` (see [`Self::reveal_and_claim`]).
    /// - `percentage`: share of the will's balance for this beneficiary.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] / [`WillError::WillNotActive`]
    /// - [`WillError::InvalidPercentages`] if total percentages would exceed 100.
    /// - [`WillError::InvalidPreimage`] if `commitment` is not a 32-byte digest.
    /// - `commitment`: SHA-256 hash of the pre-image `address_bytes || salt_bytes`.
    ///   Must be exactly 32 bytes — see the validation rules below.
    /// - `percentage`: share of the will's balance for this beneficiary, in
    ///   basis points. Must be greater than zero.
    ///
    /// # Validation
    ///
    /// A commitment is only useful if some pre-image can ever hash to it, and
    /// only claimable if exactly one slot matches it, so this entry point
    /// rejects three shapes that would otherwise strand a reserved share
    /// forever (#371):
    ///
    /// - A `commitment` that is not exactly 32 bytes cannot be a SHA-256
    ///   digest, so no pre-image can ever match it
    ///   ([`WillError::InvalidCommitmentLength`]).
    /// - A `commitment` already present on this will is rejected
    ///   ([`WillError::DuplicateCommitment`]): [`reveal_and_claim`] always
    ///   matches the *first* slot, so the second would be unclaimable.
    /// - A `percentage` of 0 reserves no funds but still occupies a slot and
    ///   dilutes every other hashed beneficiary's share of the withheld pool
    ///   ([`WillError::InvalidPercentages`]), mirroring how
    ///   `assert_valid_allocations` rejects a zero percentage for a visible
    ///   beneficiary.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] / [`WillError::WillNotActive`]
    /// - [`WillError::InvalidCommitmentLength`] if `commitment` is not exactly 32 bytes.
    /// - [`WillError::DuplicateCommitment`] if `commitment` is already registered on this will.
    /// - [`WillError::InvalidPercentages`] if `percentage` is zero, or if total
    ///   percentages would exceed 100.
    pub fn add_hashed_beneficiary(
        env: Env,
        will_id: u64,
        owner: Address,
        commitment: Bytes,
        percentage: u32,
    ) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        // A commitment that is not a SHA-256 digest can never match a hashed
        // pre-image, so the slot would be unclaimable (#452).
        if commitment.len() != 32 {
            panic_with_error!(&env, WillError::InvalidPreimage);
        }
        // Validate the new slot before mutating the will, so a rejected call
        // leaves no partial state behind (#371).
        assert_valid_hashed_beneficiary(&env, &will.hashed_beneficiaries, &commitment, percentage);

        will.hashed_beneficiaries.push_back(HashedBeneficiary {
            commitment: commitment.clone(),
            percentage,
            claimed: false,
        });

        // Validate combined percentages.
        assert_valid_percentages(&env, &will.beneficiaries, &will.hashed_beneficiaries);

        storage::save_will(&env, &will);

        events::hashed_beneficiary_added(&env, will_id, &owner, &commitment, percentage);
    }

    /// Verifies a pre-image against a stored commitment hash and, if correct,
    /// immediately transfers that beneficiary's share to the revealed address.
    ///
    /// The pre-image must be `claimant.to_xdr() || salt`: the XDR encoding of
    /// the beneficiary `Address` followed by a random salt chosen at
    /// registration time. The contract checks that the pre-image starts with
    /// the XDR of `claimant`, so the commitment is bound to one payout address.
    ///
    /// # Replay protection (#440)
    ///
    /// - Each hashed slot has a `claimed` flag that is set and persisted in
    ///   the same invocation that pays out; a second call with the same
    ///   pre-image fails with [`WillError::AlreadyClaimed`], so a replayed
    ///   transaction cannot pay out twice.
    /// - Because the pre-image is bound to `claimant` and `claimant` must
    ///   authorize the call, an attacker who observes the pre-image in a
    ///   pending or past transaction cannot resubmit it with their own
    ///   address as the recipient ([`WillError::InvalidPreimage`]).
    /// - Soroban authorization entries carry their own nonce and expiration
    ///   ledger, so the signed invocation itself cannot be replayed.
    /// ## Pre-image layout
    ///
    /// The pre-image is exactly [`PREIMAGE_LENGTH`] bytes:
    ///
    /// ```text
    /// bytes  0..32  address fingerprint of the beneficiary (see below)
    /// bytes 32..64  a random 32-byte salt chosen by the beneficiary
    /// ```
    ///
    /// The **address fingerprint** is
    /// `sha256(xdr(beneficiary_address))[0..32]` — the first
    /// [`PREIMAGE_ADDRESS_LENGTH`] bytes of the SHA-256 digest of the
    /// beneficiary address's XDR encoding. `reveal_and_claim` recomputes it
    /// from `claimant` and requires the two to be equal (#369).
    ///
    /// Earlier documentation described these first 32 bytes as "the raw bytes
    /// of the beneficiary `Address`". That was never implementable as written:
    /// a Soroban address does not serialise to 32 bytes (`Address::to_xdr`
    /// yields a tagged XDR encoding — 36 bytes for a contract account — and a
    /// different length for an account or muxed account), so no conforming
    /// pre-image could ever be decoded back into an address. A fixed 32-byte
    /// fingerprint of the address is the same length the pre-image layout has
    /// always reserved, is computable by the beneficiary at registration
    /// time, and is not invertible: to replay someone else's pre-image an
    /// attacker would have to find an address whose SHA-256 digest collides
    /// with theirs.
    ///
    /// The total length is checked first; anything else is rejected with
    /// [`WillError::InvalidPreimageLength`] (#370).
    ///
    /// ## Why the address half is bound to `claimant`
    ///
    /// A pre-image is not a secret once it is used: it appears verbatim in
    /// the transaction arguments, in simulation results, and in the mempool
    /// while the claim is pending. Before #369 the contract only checked that
    /// `sha256(preimage)` matched a stored commitment, and then paid
    /// whichever `claimant` the caller supplied. Any third party who observed
    /// the pre-image — a keeper bot, a block explorer, a mempool watcher —
    /// could replay it with their *own* address and take the reserved share
    /// before the real beneficiary ever got a transaction confirmed. Requiring
    /// `decode(preimage[0..32]) == claimant` makes the reveal useless to
    /// anyone but the address it commits to.
    ///
    /// This entrypoint works once the will is `Released`: `distribute()`
    /// withholds every unclaimed hashed beneficiary's combined percentage
    /// from what it pays the visible beneficiaries (#181) and leaves it in
    /// the will's balances, so a claim before release would have no funds
    /// behind it. A claimant's share is their percentage as a fraction of
    /// that combined withheld pool, which drains to exactly zero once every
    /// hashed beneficiary has claimed, in any order.
    ///
    /// # Parameters
    /// - `will_id`: the will to claim from.
    /// - `claimant`: the address that will receive the funds. Must authorise,
    ///   and must be the address encoded in the first 32 bytes of `preimage`.
    /// - `preimage`: raw bytes of exactly [`PREIMAGE_LENGTH`] bytes whose
    ///   SHA-256 must match a stored commitment.
    ///
    /// # Panics
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodNotExpired`] if the grace period has not elapsed.
    /// - [`WillError::InvalidPreimage`] if no matching commitment is found, or
    ///   the pre-image is not prefixed with the XDR encoding of `claimant`.
    /// - [`WillError::WillNotReleased`] if the will is not `Released`.
    /// - [`WillError::InvalidPreimageLength`] if `preimage` is not exactly
    ///   [`PREIMAGE_LENGTH`] bytes. Checked before hashing, so the empty, short
    ///   or over-long pre-image fails with this error rather than the generic
    ///   [`WillError::InvalidPreimage`] (#370).
    /// - [`WillError::PreimageAddressMismatch`] if the first 32 bytes of
    ///   `preimage` are not a valid `Address`, or decode to an address other
    ///   than `claimant` (#369). Checked before hashing, so a stolen pre-image
    ///   is rejected without ever reaching the commitment lookup.
    /// - [`WillError::InvalidPreimage`] if a correctly-sized pre-image matches no
    ///   stored commitment.
    /// - [`WillError::AlreadyClaimed`] if that slot was already claimed.
    pub fn reveal_and_claim(env: Env, will_id: u64, claimant: Address, preimage: Bytes) {
        claimant.require_auth();
        let mut will = load_will(&env, will_id);
        // A hashed beneficiary's reserved share is only carved out of the
        // will's balances once `distribute()` runs (see #181), so claiming
        // is only meaningful -- and only has funds behind it -- once the
        // will has actually been released.
        assert_status(
            &env,
            &will,
            WillStatus::Released,
            WillError::WillNotReleased,
        );

        if env.ledger().timestamp() < grace_deadline(&will) {
            panic_with_error!(&env, WillError::GracePeriodNotExpired);
        }

        // Reject a wrong-length pre-image before doing anything with it (#370).
        // The documented pre-image layout is 32 address bytes plus a 32-byte
        // salt, so any other length can never be a valid reveal; catching it
        // here also avoids paying for a SHA-256 over attacker-controlled bytes
        // and keeps the failure distinguishable from a genuine mismatch.
        if preimage.len() != PREIMAGE_LENGTH {
            panic_with_error!(&env, WillError::InvalidPreimageLength);
        }

        // Bind the pre-image to `claimant` before it is ever used (#369).
        //
        // The pre-image is public as soon as it is broadcast, so matching the
        // commitment alone would let any third party who saw it replay it
        // with their own `claimant` and take the reserved share. Requiring the
        // pre-image's address half to be the fingerprint of `claimant` makes
        // the reveal worthless to anyone but the address it commits to.
        //
        // This runs *before* the commitment hash so a stolen pre-image is
        // rejected without paying for a SHA-256 over attacker-controlled
        // bytes, and so the failure is reported as a clear binding error
        // rather than as a confusing generic mismatch.
        if !preimage_is_bound_to(&env, &claimant, &preimage) {
            panic_with_error!(&env, WillError::PreimageAddressMismatch);
        }

        // Hash the supplied pre-image with SHA-256.
        let digest = env.crypto().sha256(&preimage);
        let digest_bytes = Bytes::from_array(&env, &digest.to_array());

        // Find the matching hashed beneficiary slot.
        let mut found_idx: Option<u32> = None;
        for (i, hb) in will.hashed_beneficiaries.iter().enumerate() {
            if constant_time_bytes_eq(&hb.commitment, &digest_bytes) {
                found_idx = Some(i as u32);
                break;
            }
        }

        let idx = match found_idx {
            Some(i) => i,
            None => panic_with_error!(&env, WillError::InvalidPreimage),
        };

        // `will` was just loaded fresh from persistent storage above, so the
        // in-memory `claimed` flag is already authoritative — no separate
        // persistent lookup is needed.
        let hb = will.hashed_beneficiaries.get(idx).unwrap();
        if hb.claimed {
            panic_with_error!(&env, WillError::AlreadyClaimed);
        }

        // --- COMPUTE: each token's share from the current (pre-mutation) balances,
        // and the post-claim balances map, in a single pass ---
        // Mirrors distribute()'s multi-token payout so a hashed beneficiary on
        // a will holding more than one token is paid its share of every locked
        // token, not just the primary-token mirror.
        //
        // `will.balances` at this point holds only the pool `distribute()`
        // withheld for every still-unclaimed hashed beneficiary combined
        // (see #181), not each token's original total -- so a claimant's
        // fair share is their percentage as a fraction of the combined
        // unclaimed pool, not of 10,000. This drains to exactly zero once
        // every hashed beneficiary has claimed, regardless of claim order.
        let unclaimed_bps = unclaimed_hashed_bps(&will.hashed_beneficiaries) as i128;
        let mut transfer_plan: Vec<(Address, i128)> = Vec::new(&env);
        let mut updated_balances: Map<Address, i128> = Map::new(&env);
        let mut primary_share: i128 = 0;
        for (token_addr, total) in will.balances.iter() {
            let share = if total == 0 || hb.percentage == 0 {
                0
            } else {
                total * (hb.percentage as i128) / unclaimed_bps
            };
            if share > 0 {
                transfer_plan.push_back((token_addr.clone(), share));
                // Every token actually paid out here must leave the
                // protocol-wide "total locked" count, not just the primary
                // token (#497) -- otherwise a secondary token's locked total
                // would stay inflated forever after a hashed-beneficiary claim.
                storage::adjust_locked_value(&env, &token_addr, -share);
            }
            if token_addr == will.token {
                primary_share = share;
            }
            updated_balances.set(token_addr, total - share);
        }

        // --- EFFECTS: mutate and persist all state before any external call ---
        will.balances = updated_balances;
        // `will.balance` mirrors `will.balances[will.token]` for backward
        // compatibility; keep it in sync so other readers of the legacy field
        // don't drift from the authoritative multi-token map.
        will.balance = will.balances.get(will.token.clone()).unwrap_or(0);

        // Update the in-memory Vec entry.
        let mut updated_hb: Vec<HashedBeneficiary> = Vec::new(&env);
        for (i, entry) in will.hashed_beneficiaries.iter().enumerate() {
            if i as u32 == idx {
                updated_hb.push_back(HashedBeneficiary {
                    commitment: entry.commitment.clone(),
                    percentage: entry.percentage,
                    claimed: true,
                });
            } else {
                updated_hb.push_back(entry.clone());
            }
        }
        will.hashed_beneficiaries = updated_hb;
        storage::save_will(&env, &will);

        // --- INTERACTIONS: external token transfers execute after state is settled ---
        let contract_address = env.current_contract_address();
        for (token_addr, share) in transfer_plan.iter() {
            if share > 0 {
                token::Client::new(&env, &token_addr).transfer(
                    &contract_address,
                    &claimant,
                    &share,
                );
            }
        }

        events::hashed_claimed(&env, will_id, &claimant, primary_share);
    }
}

// ── Private helpers ─────────────────────────────────────────────────────────

/// Returns the timestamp at which a `Triggered` will's grace period ends
/// (`trigger_time + grace_period_days`).
///
/// A `Triggered` will always has a `trigger_time`; a missing one is treated
/// as "not triggered" rather than defaulting to `0`, which would make the
/// deadline already elapsed and let funds be released immediately (#442).
fn grace_period_end(env: &Env, will: &Will) -> u64 {
    let trigger_time = match will.trigger_time {
        Some(t) => t,
        None => panic_with_error!(env, WillError::WillNotTriggered),
    };
    match will
        .grace_period_days
        .checked_mul(SECONDS_PER_DAY)
        .and_then(|secs| trigger_time.checked_add(secs))
    {
        Some(deadline) => deadline,
        None => panic_with_error!(env, WillError::InvalidPeriod),
    }
}

/// Compares two `ProtocolStats` by value, treating a missing token as a zero
/// total and ignoring token order.
fn stats_match(a: &ProtocolStats, b: &ProtocolStats) -> bool {
    if a.active_will_count != b.active_will_count {
        return false;
    }
    let total_of = |stats: &ProtocolStats, token: &Address| -> i128 {
        for entry in stats.total_locked_by_token.iter() {
            if entry.token == *token {
                return entry.total_locked;
            }
        }
        0
    };
    for entry in a.total_locked_by_token.iter() {
        if total_of(b, &entry.token) != entry.total_locked {
            return false;
        }
    }
    for entry in b.total_locked_by_token.iter() {
        if total_of(a, &entry.token) != entry.total_locked {
            return false;
        }
    }
    true
}

/// Loads a will by id, panicking with [`WillError::WillNotFound`] if it does not exist.
fn load_will(env: &Env, will_id: u64) -> Will {
    match storage::load_will(env, will_id) {
        Ok(will) => will,
        Err(e) => panic_with_error!(env, e),
    }
}

/// Asserts a beneficiary list is non-empty and holds at most
/// [`MAX_BENEFICIARIES`] entries, panicking with
/// [`WillError::TooManyBeneficiaries`] otherwise.
///
/// Contract errors carry only a numeric code, so the actual count and the
/// limit are also written to the diagnostic log, which surfaces in simulation
/// and in the test host.
fn assert_beneficiary_count(env: &Env, beneficiaries: &Vec<Beneficiary>) {
    let count = beneficiaries.len();
    if count == 0 {
        log!(env, "a will needs at least 1 beneficiary");
        panic_with_error!(env, WillError::TooManyBeneficiaries);
    }
    if count > MAX_BENEFICIARIES {
        log!(
            env,
            "{} beneficiaries supplied; MAX_BENEFICIARIES is {}",
            count,
            MAX_BENEFICIARIES
        );
        panic_with_error!(env, WillError::TooManyBeneficiaries);
    }
}

/// Loads a will by id and asserts `owner` is its primary owner.
fn load_owned(env: &Env, will_id: u64, owner: &Address) -> Will {
    let will = load_will(env, will_id);
    if &will.owner != owner {
        panic_with_error!(env, WillError::NotOwner);
    }
    will
}

/// Asserts a will is in the `expected` status, panicking with `err` otherwise.
fn assert_status(env: &Env, will: &Will, expected: WillStatus, err: WillError) {
    if will.status != expected {
        panic_with_error!(env, err);
    }
}

/// Asserts a will is not in a terminal status, panicking with
/// [`WillError::WillNotActive`] otherwise.
///
/// `Released`, `Cancelled` and `Settled` wills are final: their guardian
/// rosters can no longer change, so `accept_guardian_role` /
/// `reject_guardian_role` must refuse them rather than pay for a storage write
/// on an entry nothing can act on any more (#374).
fn assert_consent_changeable(env: &Env, will: &Will) {
    match will.status {
        WillStatus::Released | WillStatus::Cancelled | WillStatus::Settled => {
            panic_with_error!(env, WillError::WillNotActive)
        }
        WillStatus::PendingConfirmation | WillStatus::Active | WillStatus::Triggered => {}
    }
}

/// Returns `guardian`'s vote weight on `will` (0 if it is not a guardian).
fn g_weight(will: &Will, guardian: &Address) -> u32 {
    will.guardians
        .iter()
        .find(|g| &g.address == guardian)
        .map(|g| g.weight)
        .unwrap_or(0)
}

/// Asserts `caller` authorized this call and is either the will's owner or
/// its designated delegate, panicking with `NotOwner` otherwise.
fn assert_owner_or_delegate(env: &Env, will: &Will, caller: &Address) {
    let is_delegate = will.delegate.as_ref().map(|d| d == caller).unwrap_or(false);
    if caller != &will.owner && !is_delegate {
        panic_with_error!(env, WillError::NotOwner);
    }
}

/// Returns whether `beneficiaries` names `address`.
///
/// Operates on an in-memory list, so callers can decide reverse-index
/// membership without touching storage.
fn names_address(beneficiaries: &Vec<Beneficiary>, address: &Address) -> bool {
    beneficiaries
        .iter()
        .any(|beneficiary| &beneficiary.address == address)
}

/// Asserts a beneficiary list's allocations are internally consistent and
/// affordable against `primary_balance`, the will's balance in its primary
/// token ([`Will::token`]):
///
/// - No address may appear more than once (the beneficiary index only stores
///   one entry per address, so a repeat would silently drop one of the
///   allocations rather than actually splitting the share).
/// - Every `Allocation::Percentage` must be non-zero, and all percentage
///   shares together must sum to exactly 10,000 basis points (100 % of
///   whatever remains once fixed amounts are set aside) — this guarantees
///   every token balance is fully distributed with no dust left behind.
/// - Every `Allocation::FixedAmount` must be positive, and the sum of every
///   fixed amount on the will must never exceed `primary_balance` —
///   otherwise `distribute` could not pay every fixed beneficiary in full.
///   Fixed amounts are denominated in the primary token only, so this is
///   deliberately *not* the sum across every token the will holds (#384):
///   comparing against a sum of unrelated tokens (with unrelated decimals)
///   would approve a will that `distribute` cannot pay out in full.
/// - A will made up entirely of `FixedAmount` beneficiaries (no percentage
///   beneficiaries at all) is allowed to account for less than the whole
///   balance; the unallocated remainder is refunded to the owner at
///   distribute time rather than left stranded in the contract (#383).
fn assert_valid_allocations(env: &Env, beneficiaries: &Vec<Beneficiary>, primary_balance: i128) {
    let mut percentage_total: u32 = 0;
    let mut fixed_total: i128 = 0;
    let mut has_percentage = false;

    for i in 0..beneficiaries.len() {
        let beneficiary = beneficiaries.get_unchecked(i);
        match beneficiary.allocation {
            Allocation::Percentage(bp) => {
                if bp == 0 {
                    panic_with_error!(env, WillError::InvalidPercentages);
                }
                total_checked_add(&mut percentage_total, bp, env);
                has_percentage = true;
            }
            Allocation::FixedAmount(amount) => {
                if amount <= 0 {
                    panic_with_error!(env, WillError::InvalidPercentages);
                }
                fixed_total = fixed_total.saturating_add(amount);
            }
        }
        for j in (i + 1)..beneficiaries.len() {
            if beneficiary.address == beneficiaries.get_unchecked(j).address {
                panic_with_error!(env, WillError::DuplicateBeneficiary);
            }
        }
    }

    if fixed_total > primary_balance {
        panic_with_error!(env, WillError::FixedAmountExceedsBalance);
    }
    if has_percentage && percentage_total != 10_000 {
        panic_with_error!(env, WillError::InvalidPercentages);
    }
    // A FixedAmount-only list is deliberately allowed to leave a portion of
    // `primary_balance` unaccounted for (fixed_total < primary_balance): that
    // headroom is exactly what a later `add_hashed_beneficiary` call
    // reserves for a not-yet-disclosed beneficiary (#181/#186). If no hashed
    // beneficiary is ever added, `distribute()` still returns whatever the
    // fixed amounts did not claim to the owner instead of stranding it in
    // the contract (#383). Any secondary token's whole balance is likewise
    // refunded, since fixed amounts are denominated in the primary token
    // only (#384).
}

/// Rejects a guardian-list replacement while guardian-cancel votes are still
/// recorded against the current list (#488).
///
/// Every list-replacing entry point — `update_guardians`,
/// `update_guardians_weighted` and the guardian branch of
/// `update_will_settings` — wipes both vote namespaces through
/// `storage::reset_guardian_votes` / `storage::reset_guardian_cancel_votes`.
/// Those entry points only run while the will is `Active`, and every
/// `Triggered -> Active` transition (`emergency_checkin`, a cancel quorum)
/// already zeroes the cancel counters, so this state is not expected to
/// arise. If it ever does, silently erasing an accumulating cancel would let
/// a fresh trigger bypass it, so the update is refused instead and the cancel
/// votes are left intact.
fn assert_no_guardian_cancel_in_flight(env: &Env, will: &Will) {
    if will.guardian_cancel_votes > 0 || will.guardian_cancel_vote_weight > 0 {
        panic_with_error!(env, WillError::GuardianCancelInProgress);
    }
}

/// Byte length of a SHA-256 digest, i.e. of a `HashedBeneficiary` commitment.
///
/// A commitment of any other length cannot be the output of
/// `env.crypto().sha256`, so no pre-image could ever match it and the reserved
/// share would be stuck forever (#371).
const SHA256_DIGEST_LEN: u32 = 32;

/// Validates a candidate hashed-beneficiary slot against the slots already on
/// the will, before it is appended by [`WillContract::add_hashed_beneficiary`].
///
/// Enforces the three rules that keep every reserved share claimable (#371):
/// the commitment is exactly [`SHA256_DIGEST_LEN`] bytes, it is not already
/// present on the will, and the percentage is non-zero. This mirrors what
/// [`assert_valid_allocations`] does for visible beneficiaries — reject zero
/// percentages and duplicate addresses — extended to the commitment instead of
/// an address.
///
/// The percentage *total* is checked separately by
/// [`assert_valid_percentages`], which needs the visible beneficiaries too and
/// runs once the new slot is on the will.
fn assert_valid_hashed_beneficiary(
    env: &Env,
    existing: &Vec<HashedBeneficiary>,
    commitment: &Bytes,
    percentage: u32,
) {
    if commitment.len() != SHA256_DIGEST_LEN {
        panic_with_error!(env, WillError::InvalidCommitmentLength);
    }
    // `reveal_and_claim` stops at the first matching slot and then reports
    // `AlreadyClaimed`, so a second slot with the same commitment could never
    // be reached by anyone.
    for hb in existing.iter() {
        if hb.commitment == *commitment {
            panic_with_error!(env, WillError::DuplicateCommitment);
        }
    }
    // A zero percentage reserves nothing yet still occupies a slot, and
    // `unclaimed_hashed_bps` counts it in the denominator of every other
    // hashed beneficiary's share — so it silently dilutes them.
    if percentage == 0 {
        panic_with_error!(env, WillError::InvalidPercentages);
    }
}

/// Final pre-transfer guard for `release_inheritance` (#490, #491).
///
/// Soroban executes a contract invocation atomically and single-threaded: no
/// other transaction can change this will's storage between the status check
/// at the top of `release_inheritance` and the transfers in `distribute`, so
/// the will is effectively locked for the whole call. This re-check is defense
/// in depth against code paths *inside* the call: it reloads the will from
/// storage and requires both the stored and the in-memory copy to still be
/// `Triggered`, with a recorded `trigger_time`.
///
/// It also re-asserts the list caps (#491). Every entry point that sets
/// beneficiaries or tokens already enforces them, but a will written under an
/// older layout (see `migration.rs`) is re-checked here, so `distribute`
/// never performs more than [`MAX_RELEASE_PAYOUTS`] beneficiary transfers.
fn assert_release_preconditions(env: &Env, will_id: u64, will: &Will) {
    let stored = load_will(env, will_id);
    if will.status != WillStatus::Triggered
        || stored.status != WillStatus::Triggered
        || stored.trigger_time.is_none()
    {
        panic_with_error!(env, WillError::WillNotTriggered);
    }
    if will.beneficiaries.len() > MAX_BENEFICIARIES {
        panic_with_error!(env, WillError::TooManyBeneficiaries);
    }
    if will.balances.len() > MAX_TOKENS {
        panic_with_error!(env, WillError::InvalidTokenCount);
    }
}

/// Rescales every `Allocation::Percentage` entry in `beneficiaries` so they
/// sum to exactly 10,000 bps again, proportionally to their current shares
/// (the last percentage entry absorbs any rounding remainder).
/// `Allocation::FixedAmount` entries pass through unchanged.
pub(crate) fn renormalize_percentages(
    env: &Env,
    beneficiaries: &Vec<Beneficiary>,
) -> Vec<Beneficiary> {
    let mut percentage_total: u32 = 0;
    let mut percentage_count: u32 = 0;
    for b in beneficiaries.iter() {
        if let Allocation::Percentage(bp) = b.allocation {
            percentage_total = percentage_total.saturating_add(bp);
            percentage_count += 1;
        }
    }

    let mut result: Vec<Beneficiary> = Vec::new(env);
    let mut percentage_index: u32 = 0;
    let mut running: u32 = 0;
    for b in beneficiaries.iter() {
        match b.allocation {
            Allocation::Percentage(bp) => {
                percentage_index += 1;
                let new_bp = if percentage_index == percentage_count {
                    10_000u32.saturating_sub(running)
                } else if percentage_total > 0 {
                    ((bp as u64) * 10_000 / percentage_total as u64) as u32
                } else {
                    0
                };
                running += new_bp;
                result.push_back(Beneficiary {
                    address: b.address.clone(),
                    allocation: Allocation::Percentage(new_bp),
                });
            }
            Allocation::FixedAmount(_) => {
                result.push_back(b.clone());
            }
        }
    }
    result
}

/// Validates that the combined `Allocation::Percentage` shares of
/// `beneficiaries` plus every hashed beneficiary's `percentage` never exceed
/// 10,000 basis points (100%) in total.
fn assert_valid_percentages(
    env: &Env,
    beneficiaries: &Vec<Beneficiary>,
    hashed_beneficiaries: &Vec<HashedBeneficiary>,
) {
    let mut total: u32 = 0;
    for b in beneficiaries.iter() {
        if let Allocation::Percentage(bp) = b.allocation {
            total_checked_add(&mut total, bp, env);
        }
    }
    for hb in hashed_beneficiaries.iter() {
        total_checked_add(&mut total, hb.percentage, env);
    }
    if total > 10_000 {
        panic_with_error!(env, WillError::InvalidPercentages);
    }
}

/// Returns the [`PREIMAGE_ADDRESS_LENGTH`]-byte address fingerprint that a
/// hashed beneficiary's pre-image must carry.
///
/// It is `sha256(xdr(address))[0..32]`: the first
/// [`PREIMAGE_ADDRESS_LENGTH`] bytes of the SHA-256 digest of the address's
/// XDR encoding. See `reveal_and_claim`'s "Pre-image layout" section for why a
/// fingerprint is used rather than the address bytes themselves (#369).
fn preimage_address_binding(env: &Env, address: &Address) -> Bytes {
    let digest = env.crypto().sha256(&address.clone().to_xdr(env));
    Bytes::from_array(env, &digest.to_array())
}

/// Constant-time equality for two `Bytes` values (#495).
///
/// `Bytes`'s own `PartialEq` compares byte-by-byte and returns as soon as it
/// finds a mismatch, so how long the comparison takes leaks how many leading
/// bytes matched. Used for `reveal_and_claim`'s commitment check, where the
/// comparison is standing in for "is this the right secret" -- the standard
/// recommendation for any such check is to never let its running time depend
/// on *where* two values first differ, even when (as here) the practical
/// exploitability is debatable, since SHA-256's avalanche effect means a
/// matching prefix is not evidence of a related input.
///
/// Different-length inputs return `false` immediately: every call site in
/// this contract compares two fixed, already-length-validated digests, so
/// the length check itself is never a timing channel in practice, and
/// comparing byte-by-byte past the shorter input's end isn't meaningful
/// anyway.
fn constant_time_bytes_eq(a: &Bytes, b: &Bytes) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a.get_unchecked(i) ^ b.get_unchecked(i);
    }
    diff == 0
}

/// Returns whether `preimage`'s address half is the fingerprint of `address`.
///
/// The comparison is length-safe: the caller has already checked that
/// `preimage` is exactly [`PREIMAGE_LENGTH`] bytes, so both halves are
/// [`PREIMAGE_ADDRESS_LENGTH`] bytes long and a mismatch is a genuine
/// difference rather than a short read. Constant-time comparison is not
/// required here — both values are public, derived from data the caller
/// already supplied.
fn preimage_is_bound_to(env: &Env, address: &Address, preimage: &Bytes) -> bool {
    preimage.slice(0..PREIMAGE_ADDRESS_LENGTH) == preimage_address_binding(env, address)
}

/// Sums the `percentage` of every hashed beneficiary that has not yet
/// claimed their share (issue #181/#186). This is the fraction of each
/// token's balance that `distribute()` must withhold from the visible
/// beneficiaries and leave in the will's `balances` map for later claiming
/// via `reveal_and_claim`.
fn unclaimed_hashed_bps(hashed_beneficiaries: &Vec<HashedBeneficiary>) -> u32 {
    let mut total: u32 = 0;
    for hb in hashed_beneficiaries.iter() {
        if !hb.claimed {
            total = total.saturating_add(hb.percentage);
        }
    }
    total
}

/// Adds `value` into `total`, panicking with `InvalidPercentages` on overflow
/// instead of aborting — a `u32` overflow here would otherwise be reachable
/// with adversarial basis-point inputs.
fn total_checked_add(total: &mut u32, value: u32, env: &Env) {
    *total = match total.checked_add(value) {
        Some(sum) => sum,
        None => panic_with_error!(env, WillError::InvalidPercentages),
    };
}

/// Sums every locked amount in a will's `balances` map.
///
/// Used by `merge_beneficiaries` to weigh each will's beneficiaries against
/// the **combined value across every token it holds**, so a will locking a
/// secondary token is no longer valued at its legacy primary-token mirror
/// alone (#382). The sum is saturating so a pathological multi-token balance
/// cannot wrap around into a negative total.
fn total_balance(balances: &Map<Address, i128>) -> i128 {
    let mut total: i128 = 0;
    for (_, amount) in balances.iter() {
        total = total.saturating_add(amount);
    }
    total
}

/// Returns `balances`' entry for `primary_token`, or 0 when the will holds
/// none of it. This — not [`total_balance`] — is the value
/// `Allocation::FixedAmount` entries are denominated in and validated
/// against: a fixed amount is a claim on the will's primary token, and
/// adding up units of unrelated tokens (with unrelated decimals) would let a
/// will be approved that `distribute` cannot pay out in full (#384).
fn primary_token_balance(balances: &Map<Address, i128>, primary_token: &Address) -> i128 {
    balances.get(primary_token.clone()).unwrap_or(0)
}

/// Asserts a guardian list is no longer than [`MAX_GUARDIANS`] and contains no
/// repeated address. Also validates that the owner is not in the guardian list.
///
/// Duplicates matter because [`WillContract::guardian_trigger`] counts each
/// address at most once. A list such as `[g, g]` looks like a working 2-of-2
/// quorum but can only ever reach a single vote, silently leaving the will with
/// a guardian override that can never fire.
///
/// The owner cannot be a guardian since guardians are meant to act when the
/// owner is incapacitated or known to be dead.
fn assert_valid_guardians(env: &Env, owner: &Address, guardians: &Vec<Address>) {
    if guardians.len() > MAX_GUARDIANS {
        panic_with_error!(env, WillError::TooManyBeneficiaries);
    }
    for i in 0..guardians.len() {
        let guardian = guardians.get_unchecked(i);
        if &guardian == owner {
            panic_with_error!(env, WillError::OwnerCannotBeGuardian);
        }
        for j in (i + 1)..guardians.len() {
            if guardian == guardians.get_unchecked(j) {
                panic_with_error!(env, WillError::DuplicateGuardian);
            }
        }
    }
}

/// Filters `owner`'s wills by `status` and cuts one page from the matches.
///
/// Every indexed will is inspected so `total_count` is exact; the owner index
/// is capped at `storage::MAX_WILLS_PER_INDEX`, which bounds the cost.
fn wills_by_owner_and_status_page(
    env: &Env,
    owner: &Address,
    status: WillStatus,
    cursor: Option<u64>,
    limit: u32,
) -> WillPage {
    let page_size = limit.min(storage::MAX_PAGE_SIZE);
    let cursor_val = cursor.unwrap_or(0);
    let ids = storage::get_owner_wills(env, owner);

    let mut wills = Vec::new(env);
    let mut total_count: u32 = 0;
    let mut has_more = false;
    for id in ids.iter() {
        let will = match storage::load_will(env, id) {
            Ok(w) => w,
            Err(e) => panic_with_error!(env, e),
        };
        if will.status != status {
            continue;
        }
        total_count += 1;
        if id <= cursor_val {
            continue;
        }
        if wills.len() < page_size {
            wills.push_back(will);
        } else {
            has_more = true;
        }
    }

    let next_cursor = if has_more {
        wills.last().map(|w| w.id)
    } else {
        None
    };

    WillPage {
        wills,
        total_count,
        next_cursor,
    }
}

/// Returns the Unix timestamp at which `will`'s grace period ends.
///
/// The grace period starts at the missed check-in deadline
/// (`last_checkin + checkin_period_days`), which `trigger_will` records in
/// `trigger_time` (#457). Every grace-period check — `emergency_checkin`,
/// `release_inheritance`, `reveal_and_claim`, `get_time_until_deadline` —
/// goes through this helper so they can never disagree. The fallback for a
/// missing `trigger_time` uses the same anchor rather than `0`, which would
/// otherwise make the grace period appear long expired.
fn grace_deadline(will: &Will) -> u64 {
    let start = will
        .trigger_time
        .unwrap_or(will.last_checkin + will.checkin_period_days * SECONDS_PER_DAY);
    start + will.grace_period_days * SECONDS_PER_DAY
}

/// Counts the live votes on `will` and returns `(vote_count, vote_weight)`.
/// A vote is live only if it was cast by a guardian on the *current* list who
/// has accepted the role and whose vote has not expired.
///
/// `has_voted` selects the namespace: [`storage::has_guardian_voted`] for
/// release votes or [`storage::has_guardian_cancel_voted`] for cancel votes.
///
/// Recomputing the tally on every vote, rather than keeping a running total,
/// enforces the upper bound from issue #453. A guardian who has been removed,
/// has rejected the role or whose vote has expired can never count toward the
/// threshold, and the count can never exceed the number of guardians on the
/// will.
fn tally_guardian_votes(
    env: &Env,
    will: &Will,
    now: u64,
    has_voted: fn(&Env, u64, &Address, u64, u64) -> bool,
) -> (u32, u32) {
    let mut votes: u32 = 0;
    let mut weight: u32 = 0;
    for g in will.guardians.iter() {
        if g.consent == GuardianConsent::Accepted
            && has_voted(env, will.id, &g.address, now, will.grace_period_days)
        {
            votes += 1;
            weight = weight.saturating_add(g.weight);
        }
    }
    (votes, weight)
}

/// Asserts both periods are at least one day and at most [`MAX_PERIOD_DAYS`].
///
/// The upper bound keeps `days * SECONDS_PER_DAY` well inside `u64`. The lower
/// bound rules out a zero-day period, which would make a will triggerable (or
/// releasable) in the very ledger it was created in, defeating the check-in
/// mechanism entirely.
fn assert_valid_periods(env: &Env, checkin_period_days: u64, grace_period_days: u64) {
    let valid = 1..=MAX_PERIOD_DAYS;
    if !valid.contains(&checkin_period_days) || !valid.contains(&grace_period_days) {
        panic_with_error!(env, WillError::InvalidPeriod);
    }
}

/// Distributes all token balances across `will.beneficiaries` proportionally
/// to their basis-point shares, transfers the shares out of the contract,
/// clears the balances map, marks the will `Released`, and publishes the
/// `InheritanceReleased` event.
///
/// # Rounding Behavior
///
/// Each token's distribution is calculated as: `share = balance * (basis_points / 10_000)`.
/// Integer division truncates toward zero, which may result in zero shares for
/// beneficiaries with very small calculated amounts. For example, distributing 9 units
/// equally among 10 beneficiaries (900 basis points each) gives each person 0.9 units,
/// which truncates to 0.
///
/// To ensure no dust is left behind, any rounding remainder is paid to the final
/// beneficiary in the list. This guarantees the full balance of every token is
/// always distributed across beneficiaries.
///
/// **Note:** Callers should ensure that the will's balance is sufficient to give
/// each beneficiary at least 1 unit of their share. Extremely small balances relative
/// to beneficiary counts can result in most recipients getting zero after rounding.
/// Consider validating a minimum will amount at creation time (see issue #37).
/// Splits `will.balance` across `will.beneficiaries` proportionally to their
/// percentages, transfers the shares out of the contract, marks the will
/// `Released`, and publishes the `InheritanceReleased` event with a full
/// For each token in `will.balances`, splits the balance across
/// `will.beneficiaries` proportionally to their basis-point shares, transfers
/// the shares out of the contract, clears the balances map, marks the will
/// `Released`, and publishes the `InheritanceReleased` event. Any rounding
/// remainder from integer division is paid to the final beneficiary so the
/// full balance of every token is always distributed with no dust left behind.
///
/// Any keeper bounty is deducted from the first token's balance before the
/// beneficiary split, and for every token the planned payouts (bounty plus
/// shares) must sum to exactly that token's balance, otherwise the release
/// aborts with [`WillError::DistributionMismatch`] (#456). A will whose
/// beneficiaries are all `FixedAmount` pays any leftover to the final
/// beneficiary. `distribute` refuses to run for a will that is no longer
/// `Active` or `Triggered`, so a will can never be paid out twice (#458).
///
/// Follows checks-effects-interactions ordering: all per-beneficiary share
/// amounts are computed from the pre-mutation balances, then all state is
/// committed (status, balances, indexes), and only then are the external
/// token transfers executed.
/// Calculates `floor(total * basis_points / 10_000)` without ever forming
/// the potentially overflowing `total * basis_points` intermediate. The
/// workspace release profile enables overflow checks, but this decomposition
/// also makes the calculation safe independently of that compiler setting.
pub(crate) fn proportional_share(total: i128, basis_points: u32) -> i128 {
    const BASIS_POINTS_TOTAL: i128 = 10_000;

    let whole = total / BASIS_POINTS_TOTAL;
    let remainder = total % BASIS_POINTS_TOTAL;
    whole * basis_points as i128 + remainder * basis_points as i128 / BASIS_POINTS_TOTAL
}

/// Pays out every token balance the will holds, marks the will `Released`,
/// and publishes the `InheritanceReleased` event.
///
/// For each token in `will.balances` the balance is split as follows:
///
/// 1. The keeper bounty (`will.keeper_bounty_bps`), taken out of a single
///    token, when a non-owner keeper released the will (#297). The token is
///    chosen once, up front, as the first entry of `will.balances` whose
///    bounty share rounds above zero; that same token is both reduced to make
///    room and used to pay the keeper (#378). If no token's share rounds
///    above zero, no bounty is paid and no beneficiary share is reduced.
/// 2. The reserve for hashed beneficiaries that have not yet called
///    `reveal_and_claim` (#181/#186). It is withheld from the visible
///    beneficiaries and left in `will.balances` for later claiming.
/// 3. `Allocation::FixedAmount` beneficiaries, which are denominated in the
///    will's *primary* token (`will.token`) only — a `FixedAmount(100)`
///    beneficiary on a two-token will receives 100 units of the primary
///    token in total, not 100 units of every token it holds (#384). Every
///    other token's whole balance is therefore available to step 4.
/// 4. `Allocation::Percentage` beneficiaries, which split whatever remains of
///    each token by basis point. The last percentage beneficiary absorbs the
///    integer-division remainder, so no dust is left behind (#190).
///
/// # Rounding
///
/// Each token's distribution is calculated as
/// `share = balance * (basis_points / 10_000)`. Integer division truncates
/// toward zero, which may result in zero shares for beneficiaries with very
/// small calculated amounts: distributing 9 units equally among 10
/// beneficiaries (900 bp each) gives each 0.9 units, which truncates to 0.
/// The remainder is paid to the final percentage beneficiary, which
/// guarantees the full balance of every token is always distributed.
///
/// **Note:** Callers should ensure that the will's balance is sufficient to
/// give each beneficiary at least 1 unit of their share. Extremely small
/// balances relative to beneficiary counts can result in most recipients
/// getting zero after rounding. Consider validating a minimum will amount at
/// creation time (see issue #37).
///
/// # Leftover value
///
/// If a will has **no** percentage beneficiaries (a `FixedAmount`-only will,
/// which `assert_valid_allocations` deliberately allows to be
/// under-allocated) the value left over after the fixed amounts have been
/// paid has no beneficiary left to receive it. Rather than leaving those
/// tokens stranded in the contract with no accounting and no withdrawal path
/// once the will is `Released`, the remainder of every token is refunded to
/// the will's owner and reported via [`events::leftover_refunded`] (#383).
/// The refund is a no-op for wills that do have percentage beneficiaries,
/// since the last one already absorbs the whole remainder.
///
/// Follows checks-effects-interactions ordering: all per-beneficiary share
/// amounts (and the owner refund) are computed from the pre-mutation
/// balances, then all state is committed (status, balances, indexes), and
/// only then are the external token transfers executed.
fn distribute(env: &Env, will: &mut Will, keeper: &Option<Address>) {
    // Defense in depth against double release (#458): every public caller
    // already asserts `Active`/`Triggered`, but a will that has left those
    // states must never be paid out again, whichever path reaches here.
    if will.status != WillStatus::Active && will.status != WillStatus::Triggered {
        panic_with_error!(env, WillError::WillNotTriggered);
    }

    let contract_address = env.current_contract_address();
    let count = will.beneficiaries.len();
    let token_count = will.balances.len();

    // --- COMPUTE: calculate every share from the current (pre-mutation) balances ---
    // Calculate keeper bounty if applicable (not paid to owner, only to other keepers)
    let mut bounty_amount: i128 = 0;
    let mut bounty_computed = false;
    // A keeper bounty is due only to a caller other than the owner.
    let should_pay_bounty = keeper
        .as_ref()
        .map(|k| k != &will.owner && will.keeper_bounty_bps > 0)
        .unwrap_or(false);

    // Pick the single token the bounty is computed from *and* paid out of,
    // before any per-token math runs (#378). Previously the bounty was
    // computed inside the per-token loop under a `bounty_amount == 0` guard
    // while the payout happened in the first entry of `transfer_plan`: on a
    // multi-token will whose first token's bounty rounded to zero, the amount
    // was computed against a *later* token but transferred with the *first*
    // token's client — and that first token's balance had never been reduced
    // to make room, so the keeper was paid out of beneficiaries' funds or the
    // whole release aborted. Choosing the token up front and keying both the
    // deduction and the payment off the same address makes the two impossible
    // to desynchronise.
    //
    // Rounding: the bounty is `floor(balance * keeper_bounty_bps / 10_000)`
    // through [`proportional_share`], so a token too small for the share to
    // round above zero contributes nothing and is skipped. If *every* token
    // rounds to zero, no bounty is paid at all and no beneficiary share is
    // reduced — the tokens are distributed in full.
    let mut bounty_token: Option<(Address, i128)> = None;
    if should_pay_bounty {
        for (token_addr, total) in will.balances.iter() {
            if total == 0 {
                continue;
            }
            let amount = proportional_share(total, will.keeper_bounty_bps);
            if amount > 0 {
                bounty_token = Some((token_addr, amount));
                break;
            }
        }
    }

    // Build a Vec of (token_addr, Vec<(beneficiary_addr, share)>) so we can
    // commit all state before any external call fires.
    let mut transfer_plan: Vec<(Address, Vec<(Address, i128)>)> = Vec::new(env);

    // Per-token amounts refunded to the owner because no beneficiary was left
    // to receive them (#383). See the "Leftover value" section of this
    // function's docs.
    let mut refund_plan: Vec<(Address, i128)> = Vec::new(env);

    // Counted once up front rather than re-scanned per token: the count is the
    // same for every token, and the percentage loop below needs it to know
    // which beneficiary absorbs the rounding remainder.
    let mut percentage_count: u32 = 0;
    for beneficiary in will.beneficiaries.iter() {
        if let Allocation::Percentage(_) = beneficiary.allocation {
            percentage_count += 1;
        }
    }

    // Any beneficiary added via `add_hashed_beneficiary` who has not yet
    // called `reveal_and_claim` has a standing claim on this fraction of
    // every token's balance. It must be withheld here rather than paid out
    // to the visible beneficiaries (#181), and left in `will.balances` for
    // `reveal_and_claim` to draw from afterward.
    let hashed_bps = unclaimed_hashed_bps(&will.hashed_beneficiaries);
    let mut hashed_reserves: Map<Address, i128> = Map::new(env);

    for (token_addr, total) in will.balances.iter() {
        if total == 0 {
            continue;
        }

        // Calculate bounty from first token's balance if applicable. The
        // bounty is carved out of this token's balance *before* beneficiary
        // shares are computed; paying it on top of a full distribution would
        // spend funds belonging to other wills held by this contract (#458).
        let mut token_bounty: i128 = 0;
        if should_pay_bounty && !bounty_computed {
            bounty_computed = true;
            bounty_amount = proportional_share(total, will.keeper_bounty_bps);
            token_bounty = bounty_amount;
        }

        // Deduct the keeper bounty only from the token it was computed from.
        // Comparing the address (rather than relying on iteration position)
        // keeps the deduction and the payout tied to the same token (#378).
        let mut available = total;
        if let Some((bounty_addr, bounty_amount)) = &bounty_token {
            if bounty_addr == &token_addr {
                available = (total - bounty_amount).max(0);
            }
        }

        if hashed_bps > 0 {
            let hashed_reserve = proportional_share(available, hashed_bps);
            available -= hashed_reserve;
            hashed_reserves.set(token_addr.clone(), hashed_reserve);
        }

        let mut shares: Vec<(Address, i128)> = Vec::new(env);

        // Fixed amounts are denominated in the will's primary token only
        // (#384): a `FixedAmount(100)` on a two-token will is 100 units of
        // `will.token` in total, not 100 units of every token the will holds.
        // Any other token's whole balance is available to the percentage
        // split below.
        //
        // The amounts are capped at what is actually available so a
        // misconfigured/under-funded will never aborts the whole
        // distribution.
        let mut remaining = available;
        if token_addr == will.token {
            for beneficiary in will.beneficiaries.iter() {
                if let Allocation::FixedAmount(amt) = beneficiary.allocation {
                    let share = amt.min(remaining).max(0);
                    remaining -= share;
                    shares.push_back((beneficiary.address.clone(), share));
                }
            }
        }

        // Whatever remains is split among percentage-based beneficiaries,
        // proportionally to their basis points; the final one absorbs the
        // rounding remainder so no dust is left behind.
        let mut percentage_index: u32 = 0;
        let mut percentage_remaining = remaining;
        for beneficiary in will.beneficiaries.iter() {
            if let Allocation::Percentage(bp) = beneficiary.allocation {
                percentage_index += 1;
                let share = if percentage_index == percentage_count {
                    percentage_remaining
                } else {
                    let portion = proportional_share(remaining, bp);
                    percentage_remaining -= portion;
                    portion
                };
                shares.push_back((beneficiary.address.clone(), share));
            }
        }

        // A will with only `FixedAmount` beneficiaries has no percentage
        // beneficiary to absorb the leftover (e.g. after a `top_up`). Give it
        // to the final beneficiary instead of leaving it stranded in the
        // contract once the balances map is cleared (#456).
        if percentage_count == 0 && remaining > 0 && !shares.is_empty() {
            let last = shares.len() - 1;
            let (addr, share) = shares.get(last).unwrap();
            shares.set(last, (addr, share + remaining));
        }

        // Conservation check (#456): the planned payouts for this token must
        // account for exactly its balance. Anything else means funds would be
        // silently lost in, or over-drawn from, the contract.
        let mut planned: i128 = token_bounty;
        for (_, share) in shares.iter() {
            if share < 0 {
                panic_with_error!(env, WillError::DistributionMismatch);
            }
            planned = match planned.checked_add(share) {
                Some(v) => v,
                None => panic_with_error!(env, WillError::DistributionMismatch),
            };
        }
        if planned != total {
            panic_with_error!(env, WillError::DistributionMismatch);
        }

        // With no percentage beneficiaries there is nobody left to absorb what
        // the fixed amounts did not claim. Refunding it to the owner keeps the
        // tokens from being stranded in the contract with no accounting and no
        // withdrawal path once the will is `Released` (#383).
        if percentage_count == 0 && remaining > 0 {
            refund_plan.push_back((token_addr.clone(), remaining));
        }

        transfer_plan.push_back((token_addr, shares));
    }

    // --- EFFECTS: mutate and persist all state before any external call ---
    storage::decrement_active_will_count(env);
    storage::adjust_locked_for_balances(env, &will.balances, -1);

    // Any tokens reserved for still-unclaimed hashed beneficiaries stay in
    // `will.balances` for `reveal_and_claim`; everything else is cleared as
    // it has now either been paid out or never held anything to reserve.
    will.balances = hashed_reserves;
    will.balance = will.balances.get(will.token.clone()).unwrap_or(0);
    will.status = WillStatus::Released;

    // Prune stale index entries (#71): remove the released will from the
    // owner index and from every beneficiary's reverse index.
    storage::remove_owner_index(env, &will.owner, will.id);
    for beneficiary in will.beneficiaries.iter() {
        storage::remove_beneficiary_index(env, &beneficiary.address, will.id);
    }

    storage::unindex_triggered_will(env, will.id);
    storage::save_will(env, will);

    // --- INTERACTIONS: external token transfers execute after state is settled ---
    //
    // Every payout below goes through `try_transfer`, not the panicking
    // `transfer` (#459). A SEP-41 token can refuse a transfer for reasons
    // entirely outside this contract's control -- a frozen or unauthorized
    // recipient, a paused token, a missing trustline for a classic asset
    // wrapped as a token contract -- and `distribute` runs once, for every
    // beneficiary and token on the will, in a single atomic call. If any one
    // of those transfers panicked, Soroban's all-or-nothing transaction
    // semantics would roll back the *entire* call, including every other
    // transfer that already succeeded earlier in the same loop -- and because
    // this contract's own state (`will.status = Released`, `will.balances`
    // cleared) was already committed above, on retry the same failing
    // transfer would be reached again, forever. One permanently-unreachable
    // recipient would therefore block every other beneficiary's inheritance
    // indefinitely, not just their own.
    //
    // A failed transfer's amount is instead recorded via
    // `storage::set_failed_payout` and left for anyone to retry later through
    // `retry_failed_payout`, while every other transfer in this same call
    // still goes through normally. The retry path never re-reads or
    // re-validates the will -- it only needs the recorded amount -- so it
    // keeps working even after this will is archived.
    for (token_addr, amount) in refund_plan.iter() {
        if amount > 0
            && pay_or_record_failure(
                env,
                &token_addr,
                &contract_address,
                &will.owner,
                amount,
                will.id,
            )
        {
            events::leftover_refunded(env, will.id, &token_addr, &will.owner, amount);
        }
    }

    for (token_addr, shares) in transfer_plan.iter() {
        for (beneficiary_addr, share) in shares.iter() {
            if share > 0 {
                pay_or_record_failure(
                    env,
                    &token_addr,
                    &contract_address,
                    &beneficiary_addr,
                    share,
                    will.id,
                );
            }
        }

        // Pay the keeper bounty out of the very token it was computed from,
        // whose balance was reduced to make room for it above (#378).
        if let (Some((bounty_addr, bounty_amount)), Some(keeper_addr)) = (&bounty_token, keeper) {
            if bounty_addr == &token_addr
                && *bounty_amount > 0
                && pay_or_record_failure(
                    env,
                    &token_addr,
                    &contract_address,
                    keeper_addr,
                    *bounty_amount,
                    will.id,
                )
            {
                events::keeper_bounty_paid(env, will.id, keeper_addr, *bounty_amount);
            }
        }
    }

    events::inheritance_released(env, will.id, token_count, count);
}

/// Attempts to transfer `amount` of `token_addr` from `from` to `to`,
/// returning whether it succeeded so the caller can publish its own
/// success-specific event (`leftover_refunded`, `keeper_bounty_paid`, or
/// nothing for an ordinary beneficiary share) exactly as it did before #459.
/// On failure, records the amount via `storage::set_failed_payout` and
/// publishes `events::payout_failed` instead of panicking, so one
/// recipient's transfer failure cannot block any other transfer in the same
/// `distribute` call.
fn pay_or_record_failure(
    env: &Env,
    token_addr: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    will_id: u64,
) -> bool {
    let result = token::Client::new(env, token_addr).try_transfer(from, to, &amount);
    if result.is_ok() {
        return true;
    }
    storage::set_failed_payout(env, will_id, token_addr, to, amount);
    events::payout_failed(env, will_id, token_addr, to, amount);
    false
}

/// Combines the two `GuardianConsent` states recorded for one guardian address
/// across two source wills into the state the merged will carries (#379).
///
/// The merge takes the *more advanced* state, ranked `Accepted` > `Pending` >
/// `Rejected`: a guardian who accepted the role on either will has consented
/// to it and may vote on the merged will, while a guardian who declined on
/// both stays `Rejected` — terminal for them, keeping them out of the vote
/// until the owner re-appoints them through `update_guardians`.
fn more_advanced_consent(a: GuardianConsent, b: GuardianConsent) -> GuardianConsent {
    match (a, b) {
        (GuardianConsent::Rejected, GuardianConsent::Rejected) => GuardianConsent::Rejected,
        (GuardianConsent::Rejected, _) | (_, GuardianConsent::Rejected) => GuardianConsent::Pending,
        (GuardianConsent::Accepted, _) | (_, GuardianConsent::Accepted) => {
            GuardianConsent::Accepted
        }
        _ => GuardianConsent::Pending,
    }
}

/// Copies a will's guardian list onto a derived will, resetting every
/// guardian's consent to [`GuardianConsent::Pending`] while preserving
/// addresses and vote weights.
///
/// `clone_will` and `split_will` hand the owner a brand-new will the guardian
/// never agreed to, so consent recorded on the source must not carry over
/// (#375) — a guardian has to be asked about the child before they can vote
/// on it, exactly as on a freshly created will.
fn reset_guardian_consent(env: &Env, guardians: &Vec<Guardian>) -> Vec<Guardian> {
    let mut reset: Vec<Guardian> = Vec::new(env);
    for g in guardians.iter() {
        reset.push_back(Guardian {
            address: g.address.clone(),
            weight: g.weight,
            consent: GuardianConsent::Pending,
        });
    }
    reset
}

/// Combines two `Allocation`s recorded for the same beneficiary across two
/// source wills into the single allocation the merged will carries.
///
/// Two fixed amounts are summed; a fixed amount always wins over a percentage
/// (the beneficiary was promised an exact sum, which a percentage split
/// cannot express); and two percentages collapse to `existing`, whose basis
/// points are only advisory here — `merge_beneficiaries` recomputes them from
/// the merged shares anyway.
fn merge_allocation(existing: &Allocation, incoming: &Allocation) -> Allocation {
    match (existing, incoming) {
        (Allocation::FixedAmount(amt_a), Allocation::FixedAmount(amt_b)) => {
            Allocation::FixedAmount(amt_a.saturating_add(*amt_b))
        }
        (Allocation::FixedAmount(amt), _) => Allocation::FixedAmount(*amt),
        (_, Allocation::FixedAmount(amt)) => Allocation::FixedAmount(*amt),
        (Allocation::Percentage(_), Allocation::Percentage(_)) => existing.clone(),
    }
}

/// Merges beneficiaries from two wills, recalculating percentages proportionally
/// based on the combined balance. If a beneficiary appears in both wills, their
/// percentages are summed before recalculation. Preserves `FixedAmount` allocation
/// types where applicable.
///
/// Shares are weighed by each will's **combined value across every token it
/// holds** ([`total_balance`]), not by the legacy primary-token mirror
/// `Will::balance`, so tokens other than the primary one are no longer ignored
/// when the merged percentages are derived (#382). Share arithmetic goes
/// through [`proportional_share`], which never forms the overflowing
/// `balance * basis_points` intermediate.
fn merge_beneficiaries(env: &Env, will_a: &Will, will_b: &Will) -> Vec<Beneficiary> {
    let value_a = total_balance(&will_a.balances);
    let value_b = total_balance(&will_b.balances);
    let total_value = value_a.saturating_add(value_b);
    let mut beneficiary_shares: Vec<(Address, i128)> = Vec::new(env);
    let mut beneficiary_allocations: Vec<(Address, Allocation)> = Vec::new(env);

    for (beneficiaries, will_value) in [
        (&will_a.beneficiaries, value_a),
        (&will_b.beneficiaries, value_b),
    ] {
        for beneficiary in beneficiaries.iter() {
            let share = match beneficiary.allocation {
                Allocation::Percentage(bp) => proportional_share(will_value, bp),
                Allocation::FixedAmount(amt) => amt,
            };
            // Accumulate in place: find this beneficiary's existing entry and
            // add to it, rather than rebuilding both accumulator Vecs from
            // scratch on every iteration (which allocated a fresh Vec per
            // beneficiary and left a dead `_allocation` clone behind, #382).
            let mut found = false;
            for i in 0..beneficiary_shares.len() {
                let (addr, existing_share) = beneficiary_shares.get(i).unwrap();
                if addr == beneficiary.address {
                    let merged = existing_share.saturating_add(share);
                    // Track original allocation type: prefer FixedAmount if
                    // either will has it.
                    let existing_alloc = beneficiary_allocations.get(i).unwrap().1;
                    beneficiary_shares.set(i, (addr.clone(), merged));
                    beneficiary_allocations.set(
                        i,
                        (
                            addr,
                            merge_allocation(&existing_alloc, &beneficiary.allocation),
                        ),
                    );
                    found = true;
                    break;
                }
            }
            if !found {
                beneficiary_shares.push_back((beneficiary.address.clone(), share));
                beneficiary_allocations
                    .push_back((beneficiary.address.clone(), beneficiary.allocation.clone()));
            }
        }
    }

    // Recalculate basis-point percentages from combined shares.
    // Ensure no beneficiary with a non-zero share is silently dropped due to rounding.
    // Preserve FixedAmount allocations where applicable.
    let mut merged_beneficiaries: Vec<Beneficiary> = Vec::new(env);
    let mut total_bp: u32 = 0;
    let count = beneficiary_shares.len();

    for (i, (addr, share)) in beneficiary_shares.iter().enumerate() {
        // `beneficiary_allocations` is maintained in lockstep with
        // `beneficiary_shares` (same address, same index), so the original
        // allocation type is a direct index lookup rather than a linear scan
        // per beneficiary.
        let original_allocation = beneficiary_allocations
            .get(i as u32)
            .map(|(_, alloc)| alloc);

        let allocation = match original_allocation {
            Some(Allocation::FixedAmount(amt)) => Allocation::FixedAmount(amt),
            _ => {
                // Convert to percentage for non-fixed-amount beneficiaries.
                // Widened to u128 so the `share * 10_000` intermediate can
                // never overflow, mirroring `proportional_share`'s guarantee
                // in the other direction (#382).
                let bp = if total_value > 0 {
                    ((share as u128 * 10_000u128) / total_value as u128) as u32
                } else {
                    0
                };

                // Include all beneficiaries: those with bp > 0, or those with share > 0 but bp = 0
                // (they get 1 bp to prevent silent dropping), or the last one (for remainder).
                if bp > 0 || (share > 0 && bp == 0) || (i as u32) == count - 1 {
                    let final_bp = if bp > 0 {
                        bp
                    } else if share > 0 {
                        1
                    } else {
                        0
                    };
                    if final_bp > 0 {
                        total_bp += final_bp;
                    }
                    Allocation::Percentage(final_bp)
                } else {
                    continue;
                }
            }
        };

        merged_beneficiaries.push_back(Beneficiary {
            address: addr,
            allocation,
        });
    }

    // Handle rounding: assign remainder to the last percentage-based beneficiary
    // to reach exactly 10,000 bp. FixedAmount beneficiaries keep their exact amounts.
    if total_bp < 10_000 && !merged_beneficiaries.is_empty() {
        let remainder = 10_000 - total_bp;
        // Find the last percentage-based beneficiary to assign the remainder
        for i in (0..merged_beneficiaries.len()).rev() {
            let beneficiary = merged_beneficiaries.get(i).unwrap();
            if let Allocation::Percentage(bp) = beneficiary.allocation {
                merged_beneficiaries.set(
                    i,
                    Beneficiary {
                        address: beneficiary.address,
                        allocation: Allocation::Percentage(bp + remainder),
                    },
                );
                break;
            }
        }
    } else if total_bp > 10_000 && !merged_beneficiaries.is_empty() {
        // If we exceeded 10_000 due to giving everyone at least 1 bp, reduce the last percentage beneficiary
        let excess = total_bp - 10_000;
        for i in (0..merged_beneficiaries.len()).rev() {
            let beneficiary = merged_beneficiaries.get(i).unwrap();
            if let Allocation::Percentage(bp) = beneficiary.allocation {
                let new_bp = if bp > excess { bp - excess } else { 1 };
                merged_beneficiaries.set(
                    i,
                    Beneficiary {
                        address: beneficiary.address,
                        allocation: Allocation::Percentage(new_bp),
                    },
                );
                break;
            }
        }
    }

    merged_beneficiaries
}

/// Records a status transition in `will_id`'s on-chain audit trail.
fn record_transition(
    env: &Env,
    will_id: u64,
    from_status: WillStatus,
    to_status: WillStatus,
    actor: &Address,
    action: soroban_sdk::Symbol,
) {
    let transition = WillStatusTransition {
        will_id,
        from_status,
        to_status,
        timestamp: env.ledger().timestamp(),
        actor: actor.clone(),
        action,
        // Overwritten by `append_history` with the will's next sequence
        // value (#500); the placeholder here is never observed by a caller.
        seq: 0,
    };
    storage::append_history(env, will_id, &transition);
}
