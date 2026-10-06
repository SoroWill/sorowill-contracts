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
    WillNotFound = 1,
    NotOwner = 2,
    WillNotActive = 3,
    WillNotTriggered = 4,
    GracePeriodNotExpired = 5,
    GracePeriodExpired = 6,
    InvalidPercentages = 7,
    AlreadyVoted = 8,
    NotGuardian = 9,
    CheckinNotDue = 10,
    ZeroAmount = 11,
    TooManyBeneficiaries = 12,
    WillNotSettled = 13,
    WillNotReleased = 23,
    NotSameOwner = 24,
    WillNotBothActive = 14,
    SameWillId = 15,
    MergeWouldExceedLimits = 16,
    InvalidPeriod = 25,
    DuplicateGuardian = 26,
    GuardianCooldownActive = 27,
    OwnerCannotBeGuardian = 17,
    BeneficiaryNotFound = 18,
    KeeperBountyExceedsMax = 19,
    InvalidGuardianThreshold = 20,
    FixedAmountExceedsBalance = 21,
    InvalidToken = 28,
    DuplicateBeneficiary = 29,
    WillNotConfirmed = 30,
    ConfirmationWindowExpired = 31,
    TooManyIds = 32,
    InsufficientBalance = 33,
    InvalidSplit = 34,
    InvalidPreimage = 35,
    AlreadyClaimed = 36,
    TooManyWills = 37,
    GuardianNotConsented = 38,
    DistributionMismatch = 39,
    UnsupportedSchemaVersion = 51,
    PrimaryTokenMismatch = 52,
    DuplicateToken = 40,
    BatchTooLarge = 41,
    InvalidTokenCount = 42,
    InvalidPreimageLength = 43,
    InvalidCommitmentLength = 44,
    DuplicateCommitment = 45,
    PreimageAddressMismatch = 46,
    MergeWithHashedBeneficiaries = 47,
    DuplicateWillId = 48,
    InvalidConsentTransition = 49,
    NoFailedPayout = 50,
    GuardianCancelInProgress = 51,
}
