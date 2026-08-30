use soroban_sdk::contracterror;

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
    /// The requested action requires the will to be `Active`.
    WillNotActive = 3,
    /// The requested action requires the will to be `Triggered`.
    WillNotTriggered = 4,
    /// `release_inheritance` was called before the grace period elapsed.
    GracePeriodNotExpired = 5,
    /// `emergency_checkin` was called after the grace period already elapsed.
    GracePeriodExpired = 6,
    /// Beneficiary percentages did not sum to exactly 10,000.
    InvalidPercentages = 7,
    /// A beneficiary percentage is not in the valid range (1..=100).
    InvalidPercentage = 22,
    /// The guardian has already voted to trigger this will.
    AlreadyVoted = 8,
    /// The caller is not a designated guardian of this will.
    NotGuardian = 9,
    /// `trigger_will` was called before the check-in deadline passed.
    CheckinNotDue = 10,
    /// An amount of zero (or less) was supplied where a positive amount is required.
    ZeroAmount = 11,
    /// Too many beneficiaries (or guardians) were supplied.
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
    /// Guardian threshold is out of range (must be 1..=guardians.len()).
    InvalidGuardianThreshold = 20,
    /// The sum of every `Allocation::FixedAmount` beneficiary on a will
    /// exceeds the will's balance, or (for a will with no percentage-based
    /// beneficiaries at all) does not exactly account for the whole balance.
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
}
