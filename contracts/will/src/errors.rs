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
///
/// # Documentation convention
///
/// Each variant below documents, in order:
/// 1. **Cause** — the condition that raises the error.
/// 2. **Recovery** — what the caller should do to proceed.
/// 3. **Example** — a short scenario showing the failure.
///
/// # Example: matching on an error code
///
/// ```ignore
/// match client.try_release_inheritance(&will_id) {
///     Err(Ok(WillError::GracePeriodNotExpired)) => {
///         // Wait until the grace period elapses, then retry.
///     }
///     Err(Ok(WillError::NotOwner)) => {
///         // Call from the owner's address.
///     }
///     other => other,
/// }
/// ```
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum WillError {
    /// No will exists for the given identifier.
    ///
    /// **Cause:** the `will_id` was never created, or was removed.
    /// **Recovery:** verify the id via `get_will`/`get_wills` before retrying.
    /// **Example:** calling `release_inheritance(999)` when no will `999` exists.
    WillNotFound = 1,
    /// The caller is not the owner of the will.
    ///
    /// **Cause:** the transaction was signed by an address other than the will's
    /// owner.
    /// **Recovery:** resubmit the call from the owner's address.
    /// **Example:** a beneficiary calls `update_beneficiaries`.
    NotOwner = 2,
    /// The requested action requires the will to be `Active`. Also returned by
    /// `update_beneficiaries` while the will is `Triggered`: the beneficiary
    /// list is frozen during a grace period; call `emergency_checkin` first to
    /// make it editable again (#732).
    ///
    /// **Cause:** the will is in a state (e.g. `Triggered`, `Released`,
    /// `Cancelled`) that does not permit the action.
    /// **Recovery:** check `get_will` status; if `Triggered`, call
    /// `emergency_checkin` to return to `Active` before retrying.
    /// **Example:** calling `update_beneficiaries` on a `Triggered` will.
    WillNotActive = 3,
    /// The requested action requires the will to be `Triggered`.
    ///
    /// **Cause:** the action (e.g. `release_inheritance`) was attempted while
    /// the will is not `Triggered`.
    /// **Recovery:** trigger the will first (via `trigger_will` or guardian
    /// quorum) and wait for the grace period.
    /// **Example:** calling `release_inheritance` on an `Active` will.
    WillNotTriggered = 4,
    /// `release_inheritance` was called before the grace period elapsed.
    ///
    /// **Cause:** the grace period after triggering has not yet passed.
    /// **Recovery:** wait until the grace period deadline, then retry.
    /// **Example:** releasing immediately after `trigger_will`.
    GracePeriodNotExpired = 5,
    /// `emergency_checkin` (or `guardian_cancel_trigger`) was called after the
    /// grace period already elapsed.
    ///
    /// **Cause:** the grace period has already expired, so the will can no
    /// longer be cancelled by check-in.
    /// **Recovery:** the will is releasable; proceed with
    /// `release_inheritance` instead.
    /// **Example:** calling `emergency_checkin` after the grace period ended.
    GracePeriodExpired = 6,
    /// Beneficiary percentages did not sum to exactly 10,000.
    ///
    /// **Cause:** the `Percentage` allocations do not total 10,000 basis
    /// points (100%).
    /// **Recovery:** adjust the percentages so they sum to exactly 10,000.
    /// **Example:** two beneficiaries at 5,000 and 4,000 (total 9,000).
    InvalidPercentages = 7,
    /// The guardian has already voted to trigger this will.
    ///
    /// **Cause:** the same guardian called `guardian_trigger` twice.
    /// **Recovery:** no action needed; the vote is already counted. Wait for
    /// other guardians to reach quorum.
    /// **Example:** a guardian re-submitting `guardian_trigger`.
    AlreadyVoted = 8,
    /// The caller is not a designated guardian of this will.
    ///
    /// **Cause:** the caller's address is not in the will's guardian list.
    /// **Recovery:** call from a designated guardian address.
    /// **Example:** a random address calls `guardian_trigger`.
    NotGuardian = 9,
    /// `trigger_will` was called before the check-in deadline passed.
    ///
    /// **Cause:** the owner's check-in deadline has not yet elapsed.
    /// **Recovery:** wait until the check-in deadline, then retry.
    /// **Example:** triggering a will whose owner just checked in.
    CheckinNotDue = 10,
    /// An amount of zero (or less) was supplied where a positive amount is required.
    ///
    /// **Cause:** a zero amount was passed to an operation requiring a positive
    /// value.
    /// **Recovery:** supply a strictly positive amount.
    /// **Example:** `create_will` with a `FixedAmount` allocation of `0`.
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
    ///
    /// **Recovery:** trim the offending list to within its cap (and keep at
    /// least one beneficiary).
    /// **Example:** `create_will` with 11 beneficiaries.
    TooManyBeneficiaries = 12,
    /// The requested action requires the will to be `Released` or `Cancelled`.
    ///
    /// **Cause:** the action was attempted on a will that has not settled.
    /// **Recovery:** wait until the will is released or cancelled.
    /// **Example:** querying settlement-only data on an `Active` will.
    WillNotSettled = 13,
    /// The requested action requires the will to be `Released`.
    ///
    /// **Cause:** the action was attempted on a will that is not `Released`.
    /// **Recovery:** release the will first, then retry.
    /// **Example:** calling a released-only action on a `Triggered` will.
    WillNotReleased = 23,
    /// Cannot merge: both wills must be owned by the same address.
    ///
    /// **Cause:** the two wills passed to `merge_wills` have different owners.
    /// **Recovery:** merge only wills owned by the same address.
    /// **Example:** merging will A (owner X) with will B (owner Y).
    NotSameOwner = 24,
    /// Cannot merge: one or both wills are not in Active status.
    ///
    /// **Cause:** at least one of the wills being merged is not `Active`.
    /// **Recovery:** merge only `Active` wills.
    /// **Example:** merging a will that is already `Triggered`.
    WillNotBothActive = 14,
    /// Cannot merge: same will id provided for both wills.
    ///
    /// **Cause:** `merge_wills` was called with identical ids.
    /// **Recovery:** supply two distinct will ids.
    /// **Example:** `merge_wills(5, 5)`.
    SameWillId = 15,
    /// Cannot merge: merging would result in too many beneficiaries or guardians.
    ///
    /// **Cause:** the combined beneficiary or guardian list would exceed its
    /// cap.
    /// **Recovery:** reduce the lists before merging.
    /// **Example:** merging two wills with 6 beneficiaries each (12 > 10).
    MergeWouldExceedLimits = 16,
    /// A check-in or grace period was zero, or long enough that the resulting
    /// deadline could not be represented as a ledger timestamp.
    ///
    /// **Cause:** the supplied period is `0` or overflows the timestamp range.
    /// **Recovery:** supply a positive, representable period.
    /// **Example:** `create_will` with a check-in period of `0`.
    InvalidPeriod = 25,
    /// The same address was supplied more than once in a guardian list.
    ///
    /// **Cause:** a guardian address appears twice in the list.
    /// **Recovery:** de-duplicate the guardian list.
    /// **Example:** `update_guardians([G, G])`.
    DuplicateGuardian = 26,
    /// The guardian-list cooldown has not yet elapsed; guardian_trigger is
    /// blocked until the cooldown period passes after the last guardian-list
    /// change.
    ///
    /// **Cause:** a guardian-list change happened recently and the cooldown is
    /// still active.
    /// **Recovery:** wait for the cooldown to elapse, then retry.
    /// **Example:** `guardian_trigger` right after `update_guardians`.
    GuardianCooldownActive = 27,
    /// The owner cannot designate themselves as a guardian of their own will.
    ///
    /// **Cause:** the owner's address appears in the guardian list.
    /// **Recovery:** remove the owner from the guardian list.
    /// **Example:** `update_guardians([owner, G])`.
    OwnerCannotBeGuardian = 17,
    /// A beneficiary is not found in the will's beneficiary list.
    ///
    /// **Cause:** the referenced beneficiary address is not present.
    /// **Recovery:** use an address that is in the beneficiary list.
    /// **Example:** `renounce_beneficiary` for an unknown address.
    BeneficiaryNotFound = 18,
    /// Keeper bounty basis points exceed the maximum allowed (100 bps/1%).
    ///
    /// **Cause:** the configured keeper bounty is above 100 bps.
    /// **Recovery:** set the bounty to at most 100 bps.
    /// **Example:** `update_will_settings` with a bounty of 200 bps.
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
    ///
    /// **Recovery:** set a threshold within `1..=reachable maximum`, or grow the
    /// guardian list/weights so the stored threshold is reachable.
    /// **Example:** a 2-guardian list with `guardian_threshold = 3`.
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
    ///
    /// **Recovery:** reduce the fixed amounts so they fit within the primary
    /// token balance.
    /// **Example:** a 100-token balance with fixed allocations totalling 150.
    FixedAmountExceedsBalance = 21,
    /// A supplied token address does not respond to a read-only `decimals()`
    /// probe, indicating it is not a valid SEP-41 token.
    ///
    /// **Cause:** the token address is not a valid SEP-41 token.
    /// **Recovery:** supply a valid SEP-41 token address.
    /// **Example:** passing a non-token contract address in `tokens`.
    InvalidToken = 28,
    /// The same beneficiary address was supplied more than once.
    ///
    /// **Cause:** a beneficiary address appears twice in the list.
    /// **Recovery:** de-duplicate the beneficiary list.
    /// **Example:** `create_will` with the same beneficiary twice.
    DuplicateBeneficiary = 29,
    /// `confirm_will` was called on a will that is not `PendingConfirmation`.
    ///
    /// **Cause:** the will is not awaiting confirmation.
    /// **Recovery:** only call `confirm_will` on a `PendingConfirmation` will.
    /// **Example:** confirming an already-`Active` will.
    WillNotConfirmed = 30,
    /// `confirm_will` was called after the confirmation deadline elapsed.
    ///
    /// **Cause:** the confirmation window has closed.
    /// **Recovery:** the will can no longer be confirmed; create a new will.
    /// **Example:** confirming after the confirmation deadline.
    ConfirmationWindowExpired = 31,
    /// `get_wills` was called with more ids than `MAX_GET_WILLS_IDS`.
    ///
    /// **Cause:** the id list exceeds the batch cap.
    /// **Recovery:** split the query into smaller batches.
    /// **Example:** `get_wills` with more than `MAX_GET_WILLS_IDS` ids.
    TooManyIds = 32,
    /// `split_will` was asked to move more of a token than the will
    /// currently holds of it.
    ///
    /// **Cause:** the split amount exceeds the will's balance of that token.
    /// **Recovery:** reduce the split amount to within the balance.
    /// **Example:** splitting 150 tokens from a will holding 100.
    InsufficientBalance = 33,
    /// `split_will` was called with an empty beneficiary-to-split list, or a
    /// split that would leave the source or new will with an invalid state.
    ///
    /// **Cause:** the split spec is empty or would produce an invalid will.
    /// **Recovery:** supply a non-empty, valid split spec.
    /// **Example:** `split_will` with an empty beneficiary list.
    InvalidSplit = 34,
    /// `reveal_and_claim` was called with a pre-image that does not match any
    /// stored `Ha

/* … truncated 10122 chars — edit only what you need near the top … */
