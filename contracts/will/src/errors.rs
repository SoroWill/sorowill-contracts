use soroban_sdk::{contracterror, contracttype};

/// High-level classification of a [`WillError`] (#709).
///
/// Allows callers, indexers, and client SDKs to distinguish expected
/// caller/validation errors from internal or system-level failures.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    /// Caller, input, authorization, or will-state precondition failure.
    User,
    /// Internal invariant, storage/schema, or resource-limit failure.
    System,
}

/// Errors returned by the SoroWill contract.
///
/// Every error variant is surfaced to callers as a `#[contracterror]` so that
/// SDK and client code can match on a stable numeric code instead of parsing
/// panic messages.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum WillError {
    /// No will exists for the given identifier.
    WillNotFound = 1,
    /// The caller is not the owner of the will.
    NotOwner = 2,
    /// The requested action requires the will to be `Active`. Also returned by
    /// `update_beneficiaries` while the will is `Triggered`: the beneficiary
    /// list is frozen during a grace period; call `emergency_checkin` first to
    /// make it editable again (#732).
    WillNotActive = 3,
    /// The requested action requires the will to be `Triggered`.
    WillNotTriggered = 4,
    /// `release_inheritance` was called before the grace period elapsed.
    GracePeriodNotExpired = 5,
    /// `emergency_checkin` (or `guardian_cancel_trigger`) was called after the
    /// grace period already elapsed.
    GracePeriodExpired = 6,
    /// Beneficiary percentages did not sum to exactly 10,000.
    InvalidPercentages = 7,
    /// The guardian has already voted to trigger this will.
    AlreadyVoted = 8,
    /// The caller is not a designated guardian of this will.
    NotGuardian = 9,
    /// `trigger_will` was called before the check-in deadline passed.
    CheckinNotDue = 10,
    /// An amount of zero (or less) was supplied where a positive amount is required.
    ZeroAmount = 11,
    /// Too many beneficiaries (or guardians/tokens) were supplied, or the list
    /// was empty. A will may name at most `MAX_BENEFICIARIES` (10)
    /// beneficiaries; see the README FAQ for why the cap exists.
    /// A list-length cap was exceeded. Raised by:
    /// - a `beneficiaries` list that is empty or longer than
    ///   `MAX_BENEFICIARIES` — `create_will`, `update_beneficiaries`,
    ///   `update_will_settings` and each `batch_create_wills` spec;
    /// - a `renounce_beneficiary` call that would leave the will with no
    ///   beneficiaries at all: the last remaining beneficiary cannot renounce,
    ///   because a will with an empty list would have nobody for
    ///   `release_inheritance` to pay (#361);
    /// - a `guardians` list longer than `MAX_GUARDIANS` — from
    ///   `assert_valid_guardians`, reached by every entry point that installs
    ///   or replaces a guardian list;
    /// - a `batch_create_wills` spec list that is empty or longer than
    ///   `BATCH_MAX`.
    ///
    /// Token-list bounds are deliberately *not* reported here: an empty or
    /// over-long `tokens` list raises [`WillError::InvalidTokenCount`] (#390).
    TooManyBeneficiaries = 12,
    /// The requested action requires the will to be `Released` or `Cancelled`.
    WillNotSettled = 13,
    /// The requested action requires the will to be `Released`.
    WillNotReleased = 23,
    /// Cannot merge: both wills must be owned by the same address.
    NotSameOwner = 24,
    /// Cannot merge: one or both wills are not in Active status.
    WillNotBothActive = 14,
    /// Cannot merge: same will id provided for both wills.
    SameWillId = 15,
    /// Cannot merge: merging would result in too many beneficiaries or guardians.
    MergeWouldExceedLimits = 16,
    /// A check-in or grace period was zero, or long enough that the resulting
    /// deadline could not be represented as a ledger timestamp.
    InvalidPeriod = 25,
    /// The same address was supplied more than once in a guardian list.
    DuplicateGuardian = 26,
    /// The guardian-list cooldown has not yet elapsed; guardian_trigger is
    /// blocked until the cooldown period passes after the last guardian-list
    /// change.
    GuardianCooldownActive = 27,
    /// The owner cannot designate themselves as a guardian of their own will.
    OwnerCannotBeGuardian = 17,
    /// A beneficiary is not found in the will's beneficiary list.
    BeneficiaryNotFound = 18,
    /// Keeper bounty basis points exceed the maximum allowed (100 bps/1%).
    KeeperBountyExceedsMax = 19,
    /// `guardian_threshold` lies outside the range the will's guardian list can
    /// actually reach.
    ///
    /// Quorum in `guardian_trigger` / `guardian_cancel_trigger` is compared
    /// against the **accumulated vote weight** of the consenting guardians, not
    /// against a head count, so the reachable maximum depends on how the list
    /// was installed:
    /// - unweighted lists — `create_will`, `update_guardians` and
    ///   `update_will_settings` give every guardian weight 1, so the valid
    ///   range is `1..=guardians.len()`;
    /// - weighted lists — `update_guardians_weighted` validates against
    ///   `1..=sum(guardian weights)`.
    ///
    /// A threshold of 0, or one above the reachable maximum, is rejected there.
    /// `update_guardians_weighted` also raises it for any `GuardianSpec` whose
    /// `weight` is `0`: zero weights are rejected, not normalised to `1` (#489).
    /// The same error is also raised when *shrinking* a non-empty guardian list
    /// would leave the will's already-stored `guardian_threshold`
    /// permanently unreachable, and when `update_guardians_weighted` is called
    /// without a threshold on a list whose total weight is below the stored
    /// one. An empty guardian list disables the mechanism, so no threshold is
    /// checked in that case.
    InvalidGuardianThreshold = 20,
    /// The sum of every `Allocation::FixedAmount` entry on a will exceeds the
    /// will's **primary-token** balance (`Will::token`, the first entry of the
    /// `tokens` list). Secondary tokens are not drawn on to satisfy a fixed
    /// amount, so they never contribute to this check (#384).
    ///
    /// `assert_valid_allocations` rejects only that over-commitment. A will
    /// whose `FixedAmount` entries do *not* exactly account for the whole
    /// primary-token balance is deliberately allowed: the unallocated headroom
    /// is what a later `add_hashed_beneficiary` call reserves for a
    /// not-yet-disclosed beneficiary (#181/#186), and if none is ever added the
    /// leftover is refunded to the owner at release time
    /// (`events::leftover_refunded`, #383) rather than stranded in the
    /// contract. Every `Percentage` share, by contrast, must still sum to
    /// exactly 10,000 basis points, or `InvalidPercentages` is raised.
    FixedAmountExceedsBalance = 21,
    /// A supplied token address does not respond to a read-only `decimals()`
    /// probe, indicating it is not a valid SEP-41 token.
    InvalidToken = 28,
    /// The same beneficiary address was supplied more than once.
    DuplicateBeneficiary = 29,
    /// `confirm_will` was called on a will that is not `PendingConfirmation`.
    WillNotConfirmed = 30,
    /// `confirm_will` was called after the confirmation deadline elapsed.
    ConfirmationWindowExpired = 31,
    /// `get_wills` was called with more ids than `MAX_GET_WILLS_IDS`.
    TooManyIds = 32,
    /// `split_will` was asked to move more of a token than the will
    /// currently holds of it.
    InsufficientBalance = 33,
    /// `split_will` was called with an empty beneficiary-to-split list, or a
    /// split that would leave the source or new will with an invalid state.
    InvalidSplit = 34,
    /// `reveal_and_claim` was called with a pre-image that does not match any
    /// stored `HashedBeneficiary` commitment on the will.
    InvalidPreimage = 35,
    /// `reveal_and_claim` was called for a hashed beneficiary slot that has
    /// already been claimed.
    AlreadyClaimed = 36,
    /// An owner or beneficiary index list is already at
    /// `MAX_WILLS_PER_INDEX` and cannot accept another will id.
    TooManyWills = 37,
    /// A guardian has not accepted their role and cannot vote.
    GuardianNotConsented = 38,
    /// `distribute` computed per-beneficiary shares that do not add up to
    /// the will's balance for some token. Aborts the release rather than
    /// silently leaving (or over-paying) funds in the contract (#456).
    DistributionMismatch = 39,
    /// A stored will could not be decoded as the current `Will` layout or any
    /// known legacy layout (see `migration.rs`).
    ///
    /// #751 — previously also `= 39`, colliding with `DistributionMismatch`:
    /// a hard compile error (duplicate discriminant), found while fixing
    /// unrelated issues in this same file and corrected alongside them since
    /// nothing else in this repo could compile until it was.
    UnsupportedSchemaVersion = 51,
    /// Cannot merge: the two wills' primary tokens differ, so summing their
    /// legacy `balance` fields would be nonsensical.
    ///
    /// #751 — previously also `= 39`; see `UnsupportedSchemaVersion` above.
    PrimaryTokenMismatch = 52,
    /// The same token address was supplied more than once in a `tokens`
    /// list. `create_will` documents each token address as unique, and a
    /// duplicated entry would make the legacy `balance` mirror disagree with
    /// the accumulated `balances` map (#350).
    DuplicateToken = 40,
    /// A `batch_check_in` call supplied more will ids than
    /// `batch_check_in_limit::MAX_BATCH_CHECK_IN` (50).
    BatchTooLarge = 41,
    /// The `tokens` list supplied to `create_will`, `clone_will`, `split_will`
    /// or `batch_create_wills` was empty, or held more than `MAX_TOKENS`
    /// entries (#390). Also raised by `top_up` when the token being topped up
    /// is not already in the will's `balances` map and that map already holds
    /// `MAX_TOKENS` distinct tokens, since each new entry is another storage
    /// read on every release or refund (#358); topping up a token the will
    /// already holds stays allowed at the cap.
    InvalidTokenCount = 42,
    /// `reveal_and_claim` was called with a pre-image that is not exactly
    /// `SHA256_DIGEST_LEN` bytes. A shorter or longer pre-image could never
    /// hash to a stored 32-byte commitment, so it is rejected before the
    /// digest is computed (#370).
    InvalidPreimageLength = 43,
    /// A hashed-beneficiary commitment was not exactly `SHA256_DIGEST_LEN`
    /// bytes and therefore could never be matched by a pre-image (#371).
    InvalidCommitmentLength = 44,
    /// The same commitment hash is already registered on this will, making
    /// the second slot unreachable by `reveal_and_claim` (#371).
    DuplicateCommitment = 45,
    /// `reveal_and_claim` was called with a pre-image whose first 32 bytes are
    /// not a valid Soroban address, or that decode to an address other than
    /// `claimant`. The pre-image is public (it sits in the transaction
    /// arguments, in simulation results, and in the mempool), so without this
    /// binding any third party who observes it could replay it with their own
    /// address as `claimant` and take the reserved share before the real
    /// beneficiary does (#369).
    PreimageAddressMismatch = 46,
    /// `merge_wills` was called while either will still carried hashed
    /// beneficiaries that had not revealed. `merge_beneficiaries` only merges
    /// *visible* beneficiaries, so the consumed will's commitments and their
    /// committed percentages would be dropped while its balance moved to the
    /// survivor — silently stranding those beneficiaries' claim (#380).
    MergeWithHashedBeneficiaries = 47,
    /// A `batch_check_in` `will_ids` list named the same will more than once,
    /// so one will would otherwise be processed — and emit a redundant
    /// `check_in` event — up to `MAX_BATCH_CHECK_IN` times in a single call,
    /// making the reported count meaningless (#355).
    DuplicateWillId = 48,
    /// `accept_guardian_role` was called by a guardian who had already
    /// `Rejected` the role. `Rejected` is terminal for a guardian entry: they
    /// can only be asked again if the owner re-appoints them through
    /// `update_guardians` / `update_guardians_weighted`, which resets the list
    /// to `Pending` (#374).
    InvalidConsentTransition = 49,
    /// `retry_failed_payout` was called for a `(will_id, token, recipient)`
    /// tuple with no recorded failed payout — either it was never recorded,
    /// it was already retried successfully, or the arguments do not match any
    /// payout `distribute` attempted (#459).
    NoFailedPayout = 50,
    /// `update_guardians`, `update_guardians_weighted` or the guardian branch
    /// of `update_will_settings` was called while guardian-cancel votes were
    /// still recorded against the current guardian list. Replacing the list
    /// would wipe those votes and let a fresh trigger bypass a cancel that was
    /// accumulating against it, so the update is refused (#488).
    GuardianCancelInProgress = 53,
    /// A schema migration produced a will that violates a structural
    /// invariant (the version did not advance by exactly one step, a
    /// `Triggered` will has no `trigger_time`, a vote tally exceeds the
    /// guardian list, or the list caps / percentage total are exceeded), so
    /// the migrated state is rejected instead of being persisted (#596).
    MigrationInvariantViolation = 54,
    /// `distribute` was asked to pay out more transfers than the
    /// `MAX_RELEASE_PAYOUTS` budget (beneficiaries x tokens) allows, so the
    /// release is rejected up front instead of running out of Soroban
    /// transaction resources partway through the payout loop (#597).
    DistributionTooLarge = 55,
    /// A pagination cursor string was malformed and could not be decoded into
    /// a valid cursor. Cursors are opaque, contract-produced values; a caller
    /// that supplies a truncated, corrupted or otherwise ill-formed string is
    /// rejected with this error instead of being parsed, so a bad cursor can
    /// never reach the pagination logic and cause a panic (#655).
    InvalidCursor = 56,
    /// `merge_wills` was called with a will that is already `Released` (or
    /// otherwise not `Active`). Merging a released will into an active one
    /// would let the distribution logic process the same beneficiary twice,
    /// so both wills must be `Active` before a merge is accepted (#656).
    MergeWithReleasedWill = 57,
    /// A beneficiary (or hashed-beneficiary claimant) address is invalid for
    /// receiving payouts — for example, the contract's own address, which would
    /// lock distributed funds inside the contract with no withdrawal path (#711).
    InvalidBeneficiaryAddress = 58,
    /// Guardian weights exceed the tally range.
    GuardianWeightOverflow = 59,
    /// A release is already in progress for this will.
    ReleaseInProgress = 60,
    /// The amount would exceed the maximum will balance.
    BalanceExceedsMax = 61,
    /// The will has already released its inheritance.
    AlreadyReleased = 62,
    /// The guardian cooldown cannot be reduced.
    GuardianCooldownCannotDecrease = 63,
    /// The beneficiary preimage salt is all zeros.
    WeakPreimageSalt = 64,
    /// The owner cannot name themselves as a beneficiary.
    OwnerCannotBeBeneficiary = 65,
    /// The ledger timestamp precedes the last check-in.
    InvalidTimestamp = 66,
    /// The token contract rejected a required transfer.
    TokenTransferFailed = 67,
}

impl WillError {
    /// Classifies this error as either a [`ErrorCategory::User`] (validation,
    /// authorization, or state precondition) or [`ErrorCategory::System`]
    /// (internal invariant, storage/schema, or resource-budget failure) (#709).
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::DistributionMismatch
            | Self::UnsupportedSchemaVersion
            | Self::MigrationInvariantViolation
            | Self::DistributionTooLarge
            | Self::GuardianWeightOverflow
            | Self::InvalidTimestamp
            | Self::TokenTransferFailed => ErrorCategory::System,

            Self::WillNotFound
            | Self::NotOwner
            | Self::WillNotActive
            | Self::WillNotTriggered
            | Self::GracePeriodNotExpired
            | Self::GracePeriodExpired
            | Self::InvalidPercentages
            | Self::AlreadyVoted
            | Self::NotGuardian
            | Self::CheckinNotDue
            | Self::ZeroAmount
            | Self::TooManyBeneficiaries
            | Self::WillNotSettled
            | Self::WillNotReleased
            | Self::NotSameOwner
            | Self::WillNotBothActive
            | Self::SameWillId
            | Self::MergeWouldExceedLimits
            | Self::InvalidPeriod
            | Self::DuplicateGuardian
            | Self::GuardianCooldownActive
            | Self::OwnerCannotBeGuardian
            | Self::BeneficiaryNotFound
            | Self::KeeperBountyExceedsMax
            | Self::InvalidGuardianThreshold
            | Self::FixedAmountExceedsBalance
            | Self::InvalidToken
            | Self::DuplicateBeneficiary
            | Self::WillNotConfirmed
            | Self::ConfirmationWindowExpired
            | Self::TooManyIds
            | Self::InsufficientBalance
            | Self::InvalidSplit
            | Self::InvalidPreimage
            | Self::AlreadyClaimed
            | Self::TooManyWills
            | Self::GuardianNotConsented
            | Self::PrimaryTokenMismatch
            | Self::DuplicateToken
            | Self::BatchTooLarge
            | Self::InvalidTokenCount
            | Self::InvalidPreimageLength
            | Self::InvalidCommitmentLength
            | Self::DuplicateCommitment
            | Self::PreimageAddressMismatch
            | Self::MergeWithHashedBeneficiaries
            | Self::DuplicateWillId
            | Self::InvalidConsentTransition
            | Self::NoFailedPayout
            | Self::GuardianCancelInProgress
            | Self::InvalidCursor
            | Self::MergeWithReleasedWill
            | Self::InvalidBeneficiaryAddress
            | Self::ReleaseInProgress
            | Self::BalanceExceedsMax
            | Self::AlreadyReleased
            | Self::GuardianCooldownCannotDecrease
            | Self::WeakPreimageSalt
            | Self::OwnerCannotBeBeneficiary => ErrorCategory::User,
        }
    }

    /// Returns `true` if this error represents a user/caller validation,
    /// authorization, or precondition failure.
    pub const fn is_user_error(&self) -> bool {
        matches!(self.category(), ErrorCategory::User)
    }

    /// Returns `true` if this error represents an internal invariant,
    /// storage/schema, or system-level failure.
    pub const fn is_system_error(&self) -> bool {
        matches!(self.category(), ErrorCategory::System)
    }
}
