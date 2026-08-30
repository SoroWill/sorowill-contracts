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
//! with fresh ones.
//!
//! Two distribution modes are supported:
//! - **Push mode** (default): `distribute` transfers tokens directly to each
//!   beneficiary in a single call.
//! - **Pull mode**: `distribute` stores each beneficiary's share as a
//!   claimable amount. Beneficiaries call [`WillContract::claim_share`]
//!   independently to withdraw their share.
//!
//! Grace periods may optionally be split into multiple tiers, each releasing
//! a configurable percentage of the balance at a different time offset.

mod errors;
mod events;
mod storage;
mod types;

/// Resource-cost profile for every public entry point. Measurement rather
/// than assertion — see the module docs for how to read the numbers.
#[cfg(test)]
mod profile;
/// Reusable harness that drives entry points with arbitrary input and asserts
/// the contract's invariants. Shared by the `proptest` suite in
/// [`fuzz_test`] and by the `cargo-fuzz` targets under `fuzz/`.
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzz_harness;

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

/// Regression test for issue #189: ensure `merge_wills` preserves `Allocation::FixedAmount`
/// beneficiaries' fixed-amount semantics instead of converting to `Allocation::Percentage`.
#[cfg(test)]
mod merge_fixed_amount_test;

/// Tests for the `update_guardians` / `update_will_settings` threshold-safety
/// check: a new guardian list that would leave `guardian_threshold` unreachable
/// must be rejected with `InvalidGuardianThreshold`.
#[cfg(test)]
mod update_guardians_threshold_test;


use soroban_sdk::{
    contract, contractimpl, panic_with_error, symbol_short, token, Address, Bytes, Env, Map, Vec,
};

pub use errors::WillError;
pub use storage::GuardianVoteRecord;
pub use types::{
    Allocation, Beneficiary, Guardian, GuardianConsent, GuardianVoteReason, HashedBeneficiary,
    ProtocolStats, Will, WillStatus, WillStatusTransition,
};

/// Semantic version of the contract logic, encoded as
/// `major * 1_000_000 + minor * 1_000 + patch`.
///
/// Bump this constant in every PR that changes observable contract behaviour
/// so that SDKs and apps can detect version mismatches at runtime via
/// [`WillContract::get_contract_version`].
///
/// Current baseline: **1.0.0** → `1_000_000`.
pub const CONTRACT_VERSION: u32 = 1_000_000;

/// Number of seconds in a day, used to convert the day-denominated periods
/// stored on a `Will` into absolute ledger timestamps.
const SECONDS_PER_DAY: u64 = 86_400;

/// Maximum number of beneficiaries a single will may have.
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

soroban_sdk::contractmeta!(
    key = "Description",
    val = "Trustless on-chain inheritance and dead man's switch protocol for Stellar Soroban"
);
// Kept in sync with CONTRACT_VERSION's semver-decoded form by
// issue_272_test.rs; bump both together.
soroban_sdk::contractmeta!(
    key = "Version",
    val = "1.0.0"
);
soroban_sdk::contractmeta!(
    key = "Homepage",
    val = "https://github.com/SoroWill/sorowill-contracts"
);

#[contract]
pub struct WillContract;

/// Current contract schema version. Must match storage::CURRENT_SCHEMA_VERSION.
const CURRENT_SCHEMA_VERSION: u32 = 1;

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
    /// - `beneficiaries`: 1 to `MAX_BENEFICIARIES` entries whose basis points
    ///   sum to exactly 10,000.
    /// - `checkin_period_days`: how many days the owner may go without checking
    ///   in; 1 to `MAX_PERIOD_DAYS`.
    /// - `grace_period_days`: how many days after being triggered the owner has
    ///   to prove they are alive; 1 to `MAX_PERIOD_DAYS`.
    /// - `guardians`: 0 to `MAX_GUARDIANS` distinct addresses that may jointly
    ///   force an early release.
    /// - `guardian_threshold`: number of guardian votes required to trigger.
    ///   Must be between 1 and `guardians.len()`. Ignored when `guardians` is empty.
    /// - `keeper_bounty_bps`: optional keeper bounty in basis points.
    ///
    /// # Returns
    /// The newly allocated will id.
    ///
    /// # Panics
    /// - [`WillError::ZeroAmount`] if any token amount is not positive.
    /// - [`WillError::TooManyBeneficiaries`] if the beneficiary/guardian/token lists are
    ///   empty or exceed their respective caps.
    /// - [`WillError::InvalidPercentages`] if beneficiary basis points do not sum to 10,000.
    /// - [`WillError::DuplicateBeneficiary`] if the same address is supplied twice.
    /// - [`WillError::DuplicateGuardian`] if the same guardian is supplied twice.
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
    /// // Create a will: lock 1 USDC, single beneficiary, 90-day check-in,
    /// // 7-day grace period, no guardians.
    /// let will_id = client.create_will(
    ///     &owner,
    ///     &vec![&env, (usdc_id.clone(), 1_000_000_i128)],
    ///     &vec![&env, Beneficiary { address: beneficiary.clone(), basis_points: 10_000 }],
    ///     &90,  // checkin_period_days
    ///     &7,   // grace_period_days
    ///     &vec![&env],  // no guardians
    ///     &1,           // guardian_threshold (ignored when no guardians)
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
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }
        if beneficiaries.is_empty() || beneficiaries.len() > MAX_BENEFICIARIES {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }
        assert_valid_guardians(&env, &owner, &guardians);
        assert_valid_periods(&env, checkin_period_days, grace_period_days);

        // Validate guardian threshold when guardians are present.
        if !guardians.is_empty() {
            let threshold_range = 1..=guardians.len();
            if !threshold_range.contains(&guardian_threshold) {
                panic_with_error!(&env, WillError::InvalidGuardianThreshold);
            }
        }

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
            // Accumulate in case the caller somehow duplicated the same token
            // address twice — treat it as an additive top-up rather than
            // silently overwriting.
            let prev = balances.get(token_addr.clone()).unwrap_or(0);
            balances.set(token_addr, prev + amount);
        }

        assert_valid_allocations(&env, &beneficiaries, total_balance(&balances));

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
        // multi-token source of truth.
        let (primary_token, primary_balance) = tokens.get_unchecked(0);

        let will = Will {
            id: will_id,
            owner: owner.clone(),
            balances,
            token: primary_token,
            is_native: false,
            balance: primary_balance,
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

        will.status = WillStatus::Active;
        will.last_checkin = now;
        will.confirmation_deadline = None;
        storage::save_will(&env, &will);

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
    pub fn batch_check_in(env: Env, will_ids: Vec<u64>, owner: Address) {
        owner.require_auth();
        let now = env.ledger().timestamp();
        let count = will_ids.len();

        for will_id in will_ids.iter() {
            let mut will = load_owned(&env, will_id, &owner);
            assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

            will.last_checkin = now;
            let next_deadline = now + will.checkin_period_days * SECONDS_PER_DAY;
            storage::save_will(&env, &will);

            events::check_in(&env, will_id, &owner, next_deadline);
        }

        events::batch_checkin(&env, &owner, &will_ids, count);
    }

    /// Starts the grace period for `will_id` once the check-in deadline has
    /// passed. Callable by anyone.
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

        will.status = WillStatus::Triggered;
        will.trigger_time = Some(now);
        let grace_period_ends = now + will.grace_period_days * SECONDS_PER_DAY;
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
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodExpired`] if the grace period has already elapsed.
    pub fn emergency_checkin(env: Env, will_id: u64, owner: Address) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(
            &env,
            &will,
            WillStatus::Triggered,
            WillError::WillNotTriggered,
        );

        let trigger_time = will.trigger_time.unwrap_or(0);
        let grace_deadline = trigger_time + will.grace_period_days * SECONDS_PER_DAY;
        let now = env.ledger().timestamp();
        if now > grace_deadline {
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
    /// In push mode (the default), tokens are transferred directly to each
    /// beneficiary. In pull mode (`pull_distribution = true`), shares are
    /// stored in claimable-shares storage and beneficiaries must call
    /// `claim_share` to withdraw.
    ///
    /// Splits are computed from `will.beneficiaries` as it stands at the
    /// moment this call executes, not as it stood when [`trigger_will`] ran —
    /// see [`renounce_beneficiary`]'s docs for the full interaction with an
    /// in-progress grace period.
    ///
    /// # Panics
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodNotExpired`] if the grace period has not elapsed yet.
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

        let trigger_time = will.trigger_time.unwrap_or(0);
        let grace_deadline = trigger_time + will.grace_period_days * SECONDS_PER_DAY;
        let now = env.ledger().timestamp();
        if now < grace_deadline {
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

        distribute(&env, &mut will, &caller);
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
        let refund = will.balance;
        let contract_address = env.current_contract_address();
        let token_count = will.balances.len();
        // Capture balances for transfer after state is committed.
        let balances_snapshot = will.balances.clone();

        // --- EFFECTS: mutate state and persist before any external calls ---
        storage::decrement_active_will_count(&env);
        storage::adjust_locked_value(&env, &will.token, -refund);

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
            WillStatus::Active,
            WillStatus::Cancelled,
            &owner,
            symbol_short!("cancel"),
        );

        // --- INTERACTIONS: external token transfers happen after state is settled ---
        for (token_addr, balance) in balances_snapshot.iter() {
            if balance > 0 {
                token::Client::new(&env, &token_addr).transfer(
                    &contract_address,
                    &owner,
                    &balance,
                );
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

        if beneficiaries.is_empty() || beneficiaries.len() > MAX_BENEFICIARIES {
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }
        assert_valid_allocations(&env, &beneficiaries, total_balance(&will.balances));

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
    /// beneficiaries.
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
    /// there is no separate confirmation step, no way to correlate a given
    /// renunciation to "the trigger cycle it applied to", and no snapshot to
    /// roll back to. Beneficiaries and owners who need the split to be
    /// stable once a will is `Triggered` must treat the trigger event itself
    /// as informational only and rely on the `beneficiary_renounced` event
    /// stream (correlated by `will_id` and timestamp against `will_triggered`)
    /// to reconstruct what happened during a given grace period.
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
    /// - [`WillError::InvalidPercentages`] if redistribution would result in invalid
    ///   percentages (shouldn't happen with a single beneficiary, but caught for safety).
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

        // A renounced `FixedAmount` share needs no redistribution: `distribute`
        // computes fixed payouts dynamically from the current beneficiary
        // list, so simply removing the entry leaves more of the balance for
        // percentage-based beneficiaries automatically. Only a renounced
        // `Percentage` share needs its basis points redistributed explicitly.
        let renounced_basis_points = match renounced_allocation {
            Some(Allocation::Percentage(bp)) => bp,
            _ => 0,
        };

        if renounced_basis_points > 0 && !new_beneficiaries.is_empty() {
            // Redistribute the renounced basis points proportionally across
            // the remaining percentage-based beneficiaries.
            let mut remaining_basis_points: u32 = 0;
            for b in new_beneficiaries.iter() {
                if let Allocation::Percentage(bp) = b.allocation {
                    remaining_basis_points = remaining_basis_points.saturating_add(bp);
                }
            }

            if remaining_basis_points > 0 {
                let mut updated_beneficiaries: Vec<Beneficiary> = Vec::new(&env);
                let mut total_redistributed: u32 = 0;
                let mut percentage_seen: u32 = 0;
                let mut percentage_total: u32 = 0;
                for b in new_beneficiaries.iter() {
                    if let Allocation::Percentage(_) = b.allocation {
                        percentage_total += 1;
                    }
                }

                for beneficiary_entry in new_beneficiaries.iter() {
                    match beneficiary_entry.allocation {
                        Allocation::Percentage(bp) => {
                            percentage_seen += 1;
                            let share_of_renounced = if percentage_seen == percentage_total {
                                // Last percentage beneficiary absorbs the rounding remainder.
                                renounced_basis_points - total_redistributed
                            } else {
                                let portion = (renounced_basis_points as u128 * bp as u128)
                                    / remaining_basis_points as u128;
                                total_redistributed =
                                    total_redistributed.saturating_add(portion as u32);
                                portion as u32
                            };
                            updated_beneficiaries.push_back(Beneficiary {
                                address: beneficiary_entry.address.clone(),
                                allocation: Allocation::Percentage(
                                    bp.saturating_add(share_of_renounced),
                                ),
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

                will.beneficiaries = updated_beneficiaries;
            } else {
                will.beneficiaries = new_beneficiaries;
            }
        } else {
            // Either the renounced share was a fixed amount (nothing to
            // redistribute), or this was the only beneficiary.
            will.beneficiaries = new_beneficiaries;
        }

        // Update indexes: remove the renouncing beneficiary from the reverse index
        storage::remove_beneficiary_index(&env, &beneficiary, will_id);

        // Validate the redistributed beneficiary allocations
        let will_balance = total_balance(&will.balances);
        assert_valid_allocations(&env, &will.beneficiaries, will_balance);

        // Get the owner for event emission (not changed)
        let owner = will.owner.clone();
        storage::save_will(&env, &will);

        events::beneficiary_renounced(&env, will_id, &beneficiary, &owner, &will.beneficiaries);
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
    ///   The owner must update the threshold via `update_guardian_threshold`
    ///   before or after shrinking the guardian list to an appropriate size.
    pub fn update_guardians(env: Env, will_id: u64, owner: Address, guardians: Vec<Address>) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        assert_valid_guardians(&env, &owner, &guardians);

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

        let now = env.ledger().timestamp();
        let next_deadline = now + will.checkin_period_days * SECONDS_PER_DAY;
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
            if new_beneficiaries.is_empty() || new_beneficiaries.len() > MAX_BENEFICIARIES {
                panic_with_error!(&env, WillError::TooManyBeneficiaries);
            }
            assert_valid_allocations(&env, &new_beneficiaries, total_balance(&will.balances));

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
            events::guardians_updated(&env, will_id, &owner);
        }
    }

    /// Adds `amount` of a specific `token` to an existing will's locked
    /// balance. Only possible while the will is `Active`. The token does not
    /// need to have been part of the original `create_will` call — new tokens
    /// can be added via `top_up`.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own `will_id`.
    /// - [`WillError::WillNotActive`] if the will is not `Active`.
    /// - [`WillError::ZeroAmount`] if `amount` is not positive.
    pub fn top_up(env: Env, will_id: u64, owner: Address, token: Address, amount: i128) {
        owner.require_auth();
        let mut will = load_owned(&env, will_id, &owner);
        assert_status(&env, &will, WillStatus::Active, WillError::WillNotActive);

        if amount <= 0 {
            panic_with_error!(&env, WillError::ZeroAmount);
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
        token::Client::new(&env, &token).transfer(
            &owner,
            &env.current_contract_address(),
            &amount,
        );

        events::top_up(&env, will_id, &owner, &token, amount, new_balance);
    }

    /// Returns the contract version as a `u32` encoded semver value:
    /// `major * 1_000_000 + minor * 1_000 + patch`.
    ///
    /// SDKs and apps can call this to detect version mismatches before
    /// submitting transactions that depend on specific contract behaviour.
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
    ///   (`trigger_time + grace_period_days`).
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
    pub fn get_guardian_vote_status(
        env: Env,
        will_id: u64,
        guardian: Address,
    ) -> Option<GuardianVoteRecord> {
        let will = load_will(&env, will_id);
        let record = storage::get_guardian_vote(&env, will_id, &guardian)?;
        let now = env.ledger().timestamp();
        let expiry_secs = will.grace_period_days * SECONDS_PER_DAY;
        if now - record.timestamp <= expiry_secs {
            Some(record)
        } else {
            None
        }
    }

    /// Returns aggregate protocol statistics for all wills currently tracked on-chain.
    pub fn get_protocol_stats(env: Env) -> ProtocolStats {
        storage::get_protocol_stats(&env)
    }

    /// Returns the list of will ids currently in `Triggered` status.
    ///
    /// This is the on-chain index that lets keeper bots and monitoring tools
    /// efficiently discover wills that are past their check-in deadline and
    /// within their grace period, without having to replay every
    /// `will_triggered` event off-chain.
    pub fn get_triggered_wills(env: Env) -> Vec<u64> {
        storage::get_triggered_wills(&env)
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

    /// Returns a page of wills owned by `owner` with the given `status`.
    ///
    /// Supports bounded pagination to avoid hitting Soroban resource limits
    /// for addresses with many wills.
    ///
    /// # Parameters
    /// - `owner`: the address to query wills for.
    /// - `status`: the will status to filter by.
    /// - `cursor`: optional will id to paginate after (exclusive). Pass `None`
    ///   or `0` for the first page.
    /// - `limit`: maximum number of wills to return. Capped at
    ///   [`storage::MAX_PAGE_SIZE`].
    pub fn get_wills_by_owner_and_status(

        env: Env,
        owner: Address,
        status: WillStatus,
        cursor: Option<u64>,
        limit: u32,
    ) -> Vec<Will> {
        let ids = storage::get_owner_wills(&env, &owner);
        let page = storage::paginate_ids(&env, &ids, cursor, limit);
        let mut wills = Vec::new(&env);
        for id in page.iter() {
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
    pub fn get_wills_by_beneficiary(env: Env, beneficiary: Address, cursor: Option<u64>, limit: u32) -> Vec<Will> {
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

        storage::set_guardian_voted(&env, will_id, &guardian, now, reason);
        will.guardian_vote_weight += weight;
        will.guardian_votes += 1;
        storage::save_will(&env, &will);

        events::guardian_voted(&env, will_id, &guardian, weight, will.guardian_vote_weight);

        if will.guardian_votes >= will.guardian_threshold {
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
    /// # Parameters
    /// - `will_id`: the will whose trigger should be cancelled.
    /// - `guardian`: the guardian casting the cancel vote; must authorize.
    ///
    /// # Panics
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::NotGuardian`] if `guardian` is not one of the will's guardians.
    /// - [`WillError::AlreadyVoted`] if `guardian` already cast a cancel vote in this cycle.
    /// - [`WillError::GuardianCooldownActive`] if the guardian-list cooldown has not elapsed.
    pub fn guardian_cancel_trigger(env: Env, will_id: u64, guardian: Address) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);
        assert_status(&env, &will, WillStatus::Triggered, WillError::WillNotTriggered);

        // Enforce guardian-list cooldown (same rule as guardian_trigger).
        let now = env.ledger().timestamp();
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
        will.guardian_cancel_vote_weight += weight;
        will.guardian_cancel_votes += 1;
        storage::save_will(&env, &will);

        events::guardian_cancel_voted(&env, will_id, &guardian, weight, will.guardian_cancel_vote_weight);

        if will.guardian_cancel_votes >= will.guardian_threshold {
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
    /// # Panics
    /// - [`WillError::WillNotFound`] if the will does not exist.
    /// - [`WillError::NotGuardian`] if `guardian` is not named on this will.
    pub fn accept_guardian_role(env: Env, will_id: u64, guardian: Address) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);

        let mut found = false;
        let mut updated_guardians: Vec<Guardian> = Vec::new(&env);
        for g in will.guardians.iter() {
            if g.address == guardian {
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
    }

    /// Allows a named guardian to reject their role on a will.
    ///
    /// A guardian can reject their role to prevent themselves from voting via
    /// [`guardian_trigger`]. Once rejected, the guardian cannot vote unless
    /// explicitly re-added to the guardian list.
    ///
    /// # Parameters
    /// - `will_id`: the will to reject guardianship for
    /// - `guardian`: the guardian address rejecting the role; must authorize
    ///
    /// # Panics
    /// - [`WillError::WillNotFound`] if the will does not exist.
    /// - [`WillError::NotGuardian`] if `guardian` is not named on this will.
    pub fn reject_guardian_role(env: Env, will_id: u64, guardian: Address) {
        guardian.require_auth();
        let mut will = load_will(&env, will_id);

        let mut found = false;
        let mut updated_guardians: Vec<Guardian> = Vec::new(&env);
        for g in will.guardians.iter() {
            if g.address == guardian {
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

        will.guardians = updated_guardians;
        storage::save_will(&env, &will);
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
    /// # Panics
    /// - [`WillError::WillNotFound`] if the source will does not exist.
    /// - [`WillError::WillNotActive`] if the source will is not `Active` or
    ///   `Triggered`.
    /// - [`WillError::ZeroAmount`] if any token amount is not positive.
    /// - [`WillError::TooManyBeneficiaries`] if the token list is empty or too large.
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
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
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
        // silently under-paying at distribute() time.
        assert_valid_allocations(&env, &source.beneficiaries, total_balance(&balances));

        let will_id = storage::next_will_id(&env);
        let now = env.ledger().timestamp();
        let token_count = balances.len();

        for beneficiary in source.beneficiaries.iter() {
            storage::index_by_beneficiary(&env, &beneficiary.address, will_id);
        }

        let (primary_token, primary_balance) = tokens.get_unchecked(0);

        let will = Will {
            id: will_id,
            owner: owner.clone(),
            balances,
            token: primary_token,
            is_native: false,
            balance: primary_balance,
            beneficiaries: source.beneficiaries.clone(),
            hashed_beneficiaries: Vec::new(&env),
            checkin_period_days: source.checkin_period_days,
            grace_period_days: source.grace_period_days,
            last_checkin: now,
            trigger_time: None,
            confirmation_deadline: None,
            status: WillStatus::Active,
            guardians: source.guardians.clone(),
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
    ///   [`BATCH_MAX`], or if any individual spec violates beneficiary/guardian/token caps.
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
                panic_with_error!(&env, WillError::TooManyBeneficiaries);
            }
            if beneficiaries.is_empty() || beneficiaries.len() > MAX_BENEFICIARIES {
                panic_with_error!(&env, WillError::TooManyBeneficiaries);
            }
            assert_valid_guardians(&env, &owner, &guardians);
            assert_valid_periods(&env, checkin_period_days, grace_period_days);
            if !guardians.is_empty() {
                let threshold_range = 1..=guardians.len();
                if !threshold_range.contains(&guardian_threshold) {
                    panic_with_error!(&env, WillError::InvalidGuardianThreshold);
                }
            }

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

            assert_valid_allocations(&env, &beneficiaries, total_balance(&balances));

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

            let (primary_token, primary_balance) = tokens.get_unchecked(0);

            let will = Will {
                id: will_id,
                owner: owner.clone(),
                balances,
                token: primary_token,
                is_native: false,
                balance: primary_balance,
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
    /// # Current behavior is a placeholder
    /// `CURRENT_SCHEMA_VERSION` is `1`, and every will created by this
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
    /// `CURRENT_SCHEMA_VERSION` and defines an actual v1 → v2 (or later)
    /// transformation here.
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
        will.schema_version = CURRENT_SCHEMA_VERSION;

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
    /// - Guardians from both wills are combined into a single list (up to MAX_GUARDIANS).
    /// - Check-in period: use the minimum (most conservative).
    /// - Grace period: use the maximum (most conservative).
    /// - The consumed will (will_id_b) is marked as Cancelled with zero balance.
    ///
    /// # Parameters
    /// - `owner`: the owner of both wills; must authorize this call.
    /// - `will_id_a`: the will that survives and receives the merged state.
    /// - `will_id_b`: the will that is consumed (marked Cancelled).
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] if `owner` does not own both wills.
    /// - [`WillError::WillNotBothActive`] if either will is not in `Active` status.
    /// - [`WillError::SameWillId`] if `will_id_a` equals `will_id_b`.
    /// - [`WillError::MergeWouldExceedLimits`] if merging would exceed MAX_BENEFICIARIES or MAX_GUARDIANS limits.
    /// - [`WillError::InvalidPercentages`] if recalculating percentages fails.
    pub fn merge_wills(
        env: Env,
        owner: Address,
        will_id_a: u64,
        will_id_b: u64,
    ) {
        owner.require_auth();

        if will_id_a == will_id_b {
            panic_with_error!(&env, WillError::SameWillId);
        }

        let mut will_a = load_owned(&env, will_id_a, &owner);
        let mut will_b = load_owned(&env, will_id_b, &owner);

        assert_status(&env, &will_a, WillStatus::Active, WillError::WillNotBothActive);
        assert_status(&env, &will_b, WillStatus::Active, WillError::WillNotBothActive);

        // Merge beneficiaries with proportional recalculation
        let merged_beneficiaries = merge_beneficiaries(&env, &will_a, &will_b);

        if merged_beneficiaries.len() > MAX_BENEFICIARIES {
            panic_with_error!(&env, WillError::MergeWouldExceedLimits);
        }

        // Merge guardians (unique)
        let mut merged_guardians = will_a.guardians.clone();
        for guardian in will_b.guardians.iter() {
            if !merged_guardians.contains(&guardian) {
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

    /// Returns the full audit trail for `will_id`, recording every status
    /// transition since creation.
    pub fn get_will_history(env: Env, will_id: u64) -> Vec<WillStatusTransition> {
        storage::get_history(&env, will_id)
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
    ///    no recovery. The final state is recoverable from the on-chain audit
    ///    trail via [`WillContract::get_will_history`] while that entry's own
    ///    TTL is still live.
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

        let archived_will = will.clone();
        storage::archive_will(&env, &will);

        events::will_archived(&env, will_id, &archived_will.owner);
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
    /// # Parameters
    /// - `will_id`: the source will to split from.
    /// - `owner`: must be the primary owner of the source will.
    /// - `beneficiaries_to_split`: subset of beneficiaries to move to the new will.
    ///   Their percentages will be renormalised to sum to 100 in the child will.
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
    /// # Panics
    /// - [`WillError::NotOwner`] / [`WillError::WillNotActive`]
    /// - [`WillError::TooManyBeneficiaries`] if `tokens` is empty or exceeds
    ///   `MAX_TOKENS`.
    /// - [`WillError::ZeroAmount`] if any token amount is not positive.
    /// - [`WillError::InsufficientBalance`] if a requested token amount
    ///   exceeds what the source will holds of that token.
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
            panic_with_error!(&env, WillError::TooManyBeneficiaries);
        }
        if beneficiaries_to_split.is_empty() {
            panic_with_error!(&env, WillError::InvalidSplit);
        }

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

        // Build a set of addresses being split out to verify they exist in the
        // source will and remove them from it.
        let mut remaining_beneficiaries: Vec<Beneficiary> = Vec::new(&env);
        for b in source.beneficiaries.iter() {
            let mut being_split = false;
            for s in beneficiaries_to_split.iter() {
                if s.address == b.address {
                    being_split = true;
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
        let normalised_split = renormalize_percentages(&env, &beneficiaries_to_split);

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
        assert_valid_allocations(&env, &normalised_remaining, total_balance(&source.balances));
        assert_valid_allocations(&env, &normalised_split, total_balance(&child_balances));

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
            guardians: source.guardians.clone(),
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
        storage::increment_active_will_count(&env);

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
    /// - `commitment`: SHA-256 hash of the pre-image `address_bytes || salt_bytes`.
    /// - `percentage`: share of the will's balance for this beneficiary.
    ///
    /// # Panics
    /// - [`WillError::NotOwner`] / [`WillError::WillNotActive`]
    /// - [`WillError::InvalidPercentages`] if total percentages would exceed 100.
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

        will.hashed_beneficiaries.push_back(HashedBeneficiary {
            commitment,
            percentage,
            claimed: false,
        });

        // Validate combined percentages.
        assert_valid_percentages(&env, &will.beneficiaries, &will.hashed_beneficiaries);

        storage::save_will(&env, &will);
    }

    /// Verifies a pre-image against a stored commitment hash and, if correct,
    /// immediately transfers that beneficiary's share to the revealed address.
    ///
    /// The pre-image must be 64 bytes: the first 32 bytes are the raw bytes of
    /// the beneficiary `Address` and the remaining 32 bytes are a random salt
    /// chosen by the beneficiary at registration time.
    ///
    /// This entrypoint works once the will is `Triggered` AND the grace period
    /// has elapsed (the same condition as `release_inheritance`). This keeps
    /// hashed-beneficiary payouts consistent with normal payouts.
    ///
    /// # Parameters
    /// - `will_id`: the will to claim from.
    /// - `claimant`: the address that will receive the funds; must authorise.
    /// - `preimage`: raw bytes whose SHA-256 must match a stored commitment.
    ///
    /// # Panics
    /// - [`WillError::WillNotTriggered`] if the will is not `Triggered`.
    /// - [`WillError::GracePeriodNotExpired`] if the grace period has not elapsed.
    /// - [`WillError::InvalidPreimage`] if no matching commitment is found.
    /// - [`WillError::AlreadyClaimed`] if that slot was already claimed.
    pub fn reveal_and_claim(env: Env, will_id: u64, claimant: Address, preimage: Bytes) {
        claimant.require_auth();
        let mut will = load_will(&env, will_id);
        assert_status(
            &env,
            &will,
            WillStatus::Triggered,
            WillError::WillNotTriggered,
        );

        let trigger_time = will.trigger_time.unwrap_or(0);
        let grace_deadline = trigger_time + will.grace_period_days * SECONDS_PER_DAY;
        if env.ledger().timestamp() < grace_deadline {
            panic_with_error!(&env, WillError::GracePeriodNotExpired);
        }

        // Hash the supplied pre-image with SHA-256.
        let digest = env.crypto().sha256(&preimage);
        let digest_bytes = Bytes::from_array(&env, &digest.to_array());

        // Find the matching hashed beneficiary slot.
        let mut found_idx: Option<u32> = None;
        for (i, hb) in will.hashed_beneficiaries.iter().enumerate() {
            if hb.commitment == digest_bytes {
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
        let mut transfer_plan: Vec<(Address, i128)> = Vec::new(&env);
        let mut updated_balances: Map<Address, i128> = Map::new(&env);
        let mut primary_share: i128 = 0;
        for (token_addr, total) in will.balances.iter() {
            let share = if total == 0 {
                0
            } else {
                total * (hb.percentage as i128) / 100
            };
            if share > 0 {
                transfer_plan.push_back((token_addr.clone(), share));
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

/// Loads a will by id, panicking with [`WillError::WillNotFound`] if it does not exist.
fn load_will(env: &Env, will_id: u64) -> Will {
    match storage::load_will(env, will_id) {
        Ok(will) => will,
        Err(e) => panic_with_error!(env, e),
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

/// Asserts `caller` authorized this call and is either the will's owner or
/// its designated delegate, panicking with `NotOwner` otherwise.
fn assert_owner_or_delegate(env: &Env, will: &Will, caller: &Address) {
    let is_delegate = will
        .delegate
        .as_ref()
        .map(|d| d == caller)
        .unwrap_or(false);
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
/// affordable against `will_balance`:
///
/// - No address may appear more than once (the beneficiary index only stores
///   one entry per address, so a repeat would silently drop one of the
///   allocations rather than actually splitting the share).
/// - Every `Allocation::Percentage` must be non-zero, and all percentage
///   shares together must sum to exactly 10,000 basis points (100 % of
///   whatever remains once fixed amounts are set aside) — this guarantees
///   every token balance is fully distributed with no dust left behind.
/// - Every `Allocation::FixedAmount` must be positive, and the sum of every
///   fixed amount on the will must never exceed `will_balance` — otherwise
///   `distribute` could not pay every fixed beneficiary in full.
/// - A will made up entirely of `FixedAmount` beneficiaries (no percentage
///   beneficiaries at all) must account for the *whole* balance, since
///   nobody is left to receive a "remainder" split.
fn assert_valid_allocations(env: &Env, beneficiaries: &Vec<Beneficiary>, will_balance: i128) {
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

    if fixed_total > will_balance {
        panic_with_error!(env, WillError::FixedAmountExceedsBalance);
    }
    if has_percentage {
        if percentage_total != 10_000 {
            panic_with_error!(env, WillError::InvalidPercentages);
        }
    } else if fixed_total != will_balance {
        panic_with_error!(env, WillError::FixedAmountExceedsBalance);
    }
}

/// Rescales every `Allocation::Percentage` entry in `beneficiaries` so they
/// sum to exactly 10,000 bps again, proportionally to their current shares
/// (the last percentage entry absorbs any rounding remainder).
/// `Allocation::FixedAmount` entries pass through unchanged.
fn renormalize_percentages(env: &Env, beneficiaries: &Vec<Beneficiary>) -> Vec<Beneficiary> {
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

/// Adds `value` into `total`, panicking with `InvalidPercentages` on overflow
/// instead of aborting — a `u32` overflow here would otherwise be reachable
/// with adversarial basis-point inputs.
fn total_checked_add(total: &mut u32, value: u32, env: &Env) {
    *total = match total.checked_add(value) {
        Some(sum) => sum,
        None => panic_with_error!(env, WillError::InvalidPercentages),
    };
}

/// Sums every token balance in `balances` into a single `i128`, saturating
/// rather than overflowing. Used to validate `Allocation::FixedAmount`
/// entries against "the will's balance" in the simplified single-balance
/// sense described in the `Allocation` docs: a fixed amount is available
/// against the combined value locked across all of a will's tokens.
fn total_balance(balances: &Map<Address, i128>) -> i128 {
    let mut total: i128 = 0;
    for (_, amount) in balances.iter() {
        total = total.saturating_add(amount);
    }
    total
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
/// Follows checks-effects-interactions ordering: all per-beneficiary share
/// amounts are computed from the pre-mutation balances, then all state is
/// committed (status, balances, indexes), and only then are the external
/// token transfers executed.
/// Calculates `floor(total * basis_points / 10_000)` without ever forming
/// the potentially overflowing `total * basis_points` intermediate. The
/// workspace release profile enables overflow checks, but this decomposition
/// also makes the calculation safe independently of that compiler setting.
fn proportional_share(total: i128, basis_points: u32) -> i128 {
    const BASIS_POINTS_TOTAL: i128 = 10_000;

    let whole = total / BASIS_POINTS_TOTAL;
    let remainder = total % BASIS_POINTS_TOTAL;
    whole * basis_points as i128
        + remainder * basis_points as i128 / BASIS_POINTS_TOTAL
}

fn distribute(env: &Env, will: &mut Will, keeper: &Option<Address>) {
    let contract_address = env.current_contract_address();
    let count = will.beneficiaries.len();
    let token_count = will.balances.len();

    // --- COMPUTE: calculate every share from the current (pre-mutation) balances ---
    // Calculate keeper bounty if applicable (not paid to owner, only to other keepers)
    let mut bounty_amount: i128 = 0;
    let should_pay_bounty = keeper
        .as_ref()
        .map(|k| k != &will.owner && will.keeper_bounty_bps > 0)
        .unwrap_or(false);

    // Build a Vec of (token_addr, Vec<(beneficiary_addr, share)>) so we can
    // commit all state before any external call fires.
    let mut transfer_plan: Vec<(Address, Vec<(Address, i128)>)> = Vec::new(env);

    for (token_addr, total) in will.balances.iter() {
        if total == 0 {
            continue;
        }

        // Calculate bounty from first token's balance if applicable
        if should_pay_bounty && bounty_amount == 0 {
            bounty_amount = proportional_share(total, will.keeper_bounty_bps);
        }

        let mut shares: Vec<(Address, i128)> = Vec::new(env);

        // Fixed-amount beneficiaries are paid first, capped at what is
        // actually available so a misconfigured/under-funded token never
        // aborts the whole distribution.
        let mut remaining = total;
        for beneficiary in will.beneficiaries.iter() {
            if let Allocation::FixedAmount(amt) = beneficiary.allocation {
                let share = amt.min(remaining).max(0);
                remaining -= share;
                shares.push_back((beneficiary.address.clone(), share));
            }
        }

        // Whatever remains is split among percentage-based beneficiaries,
        // proportionally to their basis points; the final one absorbs the
        // rounding remainder so no dust is left behind.
        let mut percentage_count: u32 = 0;
        for beneficiary in will.beneficiaries.iter() {
            if let Allocation::Percentage(_) = beneficiary.allocation {
                percentage_count += 1;
            }
        }
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

        transfer_plan.push_back((token_addr, shares));
    }

    // --- EFFECTS: mutate and persist all state before any external call ---
    storage::decrement_active_will_count(env);

    will.balance = 0;
    will.balances = Map::new(env);
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
    for (token_addr, shares) in transfer_plan.iter() {
        let token_client = token::Client::new(env, &token_addr);
        for (beneficiary_addr, share) in shares.iter() {
            if share > 0 {
                token_client.transfer(&contract_address, &beneficiary_addr, &share);
            }
        }

        // Pay keeper bounty from first token if applicable
        if should_pay_bounty && bounty_amount > 0 {
            if let Some(keeper_addr) = keeper {
                token_client.transfer(&contract_address, keeper_addr, &bounty_amount);
                events::keeper_bounty_paid(env, will.id, keeper_addr, bounty_amount);
            }
            bounty_amount = 0; // Only pay once
        }
    }

    events::inheritance_released(env, will.id, token_count, count);
}

/// Merges beneficiaries from two wills, recalculating percentages proportionally
/// based on the combined balance. If a beneficiary appears in both wills, their
/// percentages are summed before recalculation. Preserves `FixedAmount` allocation
/// types where applicable.
fn merge_beneficiaries(env: &Env, will_a: &Will, will_b: &Will) -> Vec<Beneficiary> {
    let total_balance = will_a.balance + will_b.balance;
    let mut beneficiary_shares: Vec<(Address, i128)> = Vec::new(env);
    let mut beneficiary_allocations: Vec<(Address, Allocation)> = Vec::new(env);

    for (beneficiaries, will_balance) in [
        (&will_a.beneficiaries, will_a.balance),
        (&will_b.beneficiaries, will_b.balance),
    ] {
        for beneficiary in beneficiaries.iter() {
            let share = match beneficiary.allocation {
                Allocation::Percentage(bp) => will_balance * (bp as i128) / 10_000,
                Allocation::FixedAmount(amt) => amt,
            };
            let mut found = false;
            let mut updated_shares: Vec<(Address, i128)> = Vec::new(env);
            let mut updated_allocations: Vec<(Address, Allocation)> = Vec::new(env);
            for (addr, existing_share) in beneficiary_shares.iter() {
                if addr == beneficiary.address {
                    updated_shares.push_back((addr, existing_share + share));
                    found = true;
                } else {
                    updated_shares.push_back((addr, existing_share));
                }
            }
            // Track original allocation type: prefer FixedAmount if either will has it
            for (addr, existing_alloc) in beneficiary_allocations.iter() {
                if addr == beneficiary.address {
                    // If either the existing or new allocation is FixedAmount, preserve it
                    let merged_alloc = match (existing_alloc, beneficiary.allocation) {
                        (Allocation::FixedAmount(amt_a), Allocation::FixedAmount(amt_b)) => {
                            Allocation::FixedAmount(amt_a + amt_b)
                        },
                        (Allocation::FixedAmount(amt), _) => Allocation::FixedAmount(amt),
                        (_, Allocation::FixedAmount(amt)) => Allocation::FixedAmount(amt),
                        (Allocation::Percentage(_), Allocation::Percentage(_)) => {
                            existing_alloc.clone()
                        },
                    };
                    updated_allocations.push_back((addr, merged_alloc));
                } else {
                    updated_allocations.push_back((addr, existing_alloc.clone()));
                }
            }
            if found {
                beneficiary_shares = updated_shares;
                beneficiary_allocations = updated_allocations;
            } else {
                beneficiary_shares.push_back((beneficiary.address.clone(), share));
                beneficiary_allocations.push_back((beneficiary.address.clone(), beneficiary.allocation));
            }
        }
    }

    // Recalculate basis-point percentages from combined shares.
    // Ensure no beneficiary with a non-zero share is silently dropped due to rounding.
    // Preserve FixedAmount allocations where applicable.
    let mut merged_beneficiaries: Vec<Beneficiary> = Vec::new(env);
    let mut total_bp: u32 = 0;
    let mut total_fixed: i128 = 0;
    let count = beneficiary_shares.len();

    for (i, (addr, share)) in beneficiary_shares.iter().enumerate() {
        // Find the original allocation type for this beneficiary
        let original_allocation = beneficiary_allocations.iter()
            .find(|(a, _)| a == addr)
            .map(|(_, alloc)| alloc);

        let allocation = match original_allocation {
            Some(Allocation::FixedAmount(amt)) => {
                total_fixed += amt;
                Allocation::FixedAmount(amt)
            },
            _ => {
                // Convert to percentage for non-fixed-amount beneficiaries
                let bp = if total_balance > 0 {
                    ((share * 10_000) / total_balance) as u32
                } else {
                    0
                };

                // Include all beneficiaries: those with bp > 0, or those with share > 0 but bp = 0
                // (they get 1 bp to prevent silent dropping), or the last one (for remainder).
                if bp > 0 || (share > 0 && bp == 0) || (i as u32) == count - 1 {
                    let final_bp = if bp > 0 { bp } else if share > 0 { 1 } else { 0 };
                    if final_bp > 0 {
                        total_bp += final_bp;
                    }
                    Allocation::Percentage(final_bp)
                } else {
                    continue;
                }
            },
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
    };
    storage::append_history(env, will_id, &transition);
}

