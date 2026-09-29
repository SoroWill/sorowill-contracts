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
    /// A list-length cap was exceeded. Raised by:
    /// - a `beneficiaries` list that is empty or longer than
    ///   `MAX_BENEFICIARIES` — `create_will`, `update_beneficiaries`,
    ///   `update_will_settings` and each `batch_create_wills` spec;
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
    /// Cannot merge: the two wills' primary tokens differ, so summing their
    /// legacy `balance` fields would be nonsensical.
    PrimaryTokenMismatch = 39,
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
    /// entries (#390).
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
}
