use soroban_sdk::{contracttype, Address, Bytes, Map, Symbol, Vec};

/// How a beneficiary's share of a will is calculated.
///
/// - `Percentage(bp)` is a share expressed in basis points (1 bp = 0.01 %) of
///   whatever balance remains *after* every `FixedAmount` beneficiary on the
///   same will has been paid, applied to **every** token the will holds. All
///   `Percentage` shares on a will must sum to exactly 10,000 (100 % of the
///   remainder).
/// - `FixedAmount(amount)` entitles the beneficiary to exactly `amount` of
///   the will's **primary token** ([`Will::token`], the first entry of the
///   `tokens` list passed to `create_will`), paid before any percentage-based
///   split is computed. The sum of all `FixedAmount` entries on a will can
///   never exceed that token's balance (enforced by
///   `assert_valid_allocations`).
///
/// # Fixed amounts and multi-token wills
///
/// A `FixedAmount` is a claim on one specific token, not on "the will's
/// value". A will holding two tokens and a `FixedAmount(100)` beneficiary
/// pays that beneficiary 100 units of the primary token **in total** — the
/// secondary token's balance is not drawn on to satisfy it, and is instead
/// split among the percentage beneficiaries or, if there are none, refunded
/// to the owner at release time (issue #384, #383).
///
/// A single will may mix both kinds: e.g. one beneficiary with a fixed
/// amount and the rest splitting the remainder by percentage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Allocation {
    Percentage(u32),
    FixedAmount(i128),
}

/// A single beneficiary entry: an address and how much of the will's balance
/// it is entitled to receive when the inheritance is released.
///
/// See [`Allocation`] for how `allocation` is interpreted and validated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Beneficiary {
    pub address: Address,
    pub allocation: Allocation,
}

/// Consent status for a named guardian.
///
/// A guardian must explicitly accept before they can cast a `guardian_trigger`
/// vote. The owner may also reject a guardian's acceptance.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardianConsent {
    /// Guardian has been named but has not yet responded.
    Pending = 0,
    /// Guardian has accepted the role and may vote.
    Accepted = 1,
    /// Guardian has declined the role.
    Rejected = 2,
}

/// A guardian definition used by the weighted-guardian API.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianSpec {
    pub address: Address,
    pub weight: u32,
}

/// A guardian entry: an address paired with a vote weight and consent status.
///
/// Guardians with higher weights count for more when reaching quorum.
/// If all guardians have weight 1, the threshold is a simple majority count.
/// A guardian must accept their role via `accept_guardian_role` before they
/// can vote in `guardian_trigger`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Guardian {
    pub address: Address,
    pub weight: u32,
    pub consent: GuardianConsent,
}

/// A privacy-preserving beneficiary entry (issue #46).
///
/// Instead of a raw address the owner stores a SHA-256 commitment hash of the
/// pre-image `<address_bytes> || <salt_bytes>`. At claim time the beneficiary
/// calls `reveal_and_claim` with the pre-image; the contract verifies the hash
/// matches and pays out to the revealed address.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashedBeneficiary {
    /// SHA-256 hash of the pre-image (address bytes concatenated with salt).
    pub commitment: Bytes,
    /// Percentage of the will's balance this beneficiary receives.
    pub percentage: u32,
    /// Whether this hashed beneficiary has already been claimed.
    pub claimed: bool,
}

/// Lifecycle state of a will.
///
/// ```text
/// create_will --(confirmation delay)--> PendingConfirmation
/// create_will --(no delay)------------> Active
///     |                                    |
///     |--(confirm_will)--> Active <--------+
///     |
///     |   Active --(missed check-in / trigger_will)--> Triggered
///     |      |                                            |
///     |      |--(cancel_will)--> Cancelled               |--(grace period expires)--> Released
///     |      |                                            |    |
///     |      |--(emergency_checkin)--> Active            |    |--(close_will)--> Settled
///     |      |--(guardian_cancel_trigger)--> Active       |    |
///     |      |--(guardian_trigger quorum)--> Released ----+    |
///     |                                                          |
///     +--(cancel_will)--> Cancelled <--(cancel_will, PendingConfirmation)
///
/// PendingConfirmation --(cancel_will)--> Cancelled
/// ```
///
/// Every arrow above is a real transition performed by the named entry point;
/// `check_in` and the other settings updates do not change `status`.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WillStatus {
    /// The will has just been created and is waiting for the owner to confirm
    /// it within the confirmation window (issue #43). No check-in clock runs
    /// while a will is in this state.
    PendingConfirmation,
    /// The will is funded and the owner is checking in on schedule.
    Active,
    /// The owner missed a check-in deadline; the grace period is running.
    Triggered,
    /// The grace period expired (or guardians reached quorum) and funds were
    /// distributed to beneficiaries (lump sum or final vested claim).
    Released,
    /// The owner cancelled the will and withdrew the remaining balance.
    Cancelled,
    /// A Released will that has been explicitly closed/archived by the owner.
    Settled,
}

/// Aggregate protocol statistics that can be queried directly on-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenLockedBalance {
    /// The token contract address.
    pub token: Address,
    /// The total amount of this token that is currently locked in active wills.
    pub total_locked: i128,
}

/// Aggregate protocol statistics that can be queried directly on-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolStats {
    /// The number of wills that are still in a non-terminal state.
    pub active_will_count: u64,
    /// Locked balances by token for all currently active wills.
    pub total_locked_by_token: Vec<TokenLockedBalance>,
}

/// Aggregate statistics for the wills owned by a single address (issue #447).
///
/// Returned by `get_owner_stats` so clients can show ownership totals
/// alongside a paginated `get_wills_by_owner` listing without walking every
/// page themselves.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerStats {
    /// Total number of wills indexed under this owner, in any status.
    pub total_wills: u32,
    /// Number of those wills that are still non-terminal
    /// (`PendingConfirmation`, `Active` or `Triggered`).
    pub active_wills: u32,
    /// Locked balances by token, summed across the owner's non-terminal wills.
    pub total_locked_by_token: Vec<TokenLockedBalance>,
}



/// A beneficiary's claimable share in a pull-based distribution.
///
/// Stored in persistent storage keyed by `(will_id, beneficiary_address)`.
/// When the will enters `Released` status with `pull_distribution = true`,
/// `distribute` computes each beneficiary's share and stores a `ClaimableShare`
/// with `total` set to the share amount and `claimed` set to `0`. When the
/// beneficiary calls `claim_share`, `claimed` is set to `total` and the tokens
/// are transferred out of the contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimableShare {
    /// The total amount the beneficiary is entitled to claim.
    pub total: i128,
    /// The amount already claimed (0 or `total`).
    pub claimed: i128,
}

/// The full on-chain state of a single will.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Will {
    /// Unique, monotonically increasing identifier for this will.
    pub id: u64,
    /// The primary owner address. Used for backwards-compatible single-owner
    /// flows and as the refund destination on cancellation.
    pub owner: Address,
    /// Map of token contract address → amount currently locked in the will,
    /// in each token's base units. A will may hold any number of distinct
    /// SEP-41 compliant tokens simultaneously.
    pub balances: Map<Address, i128>,
    /// The token contract (e.g. a USDC Stellar Asset Contract) held by the will.
    pub token: Address,
    /// Whether the held asset is native XLM (as opposed to a token contract).
    /// When `true`, transfers use `env.transfer()` instead of the token client.
    pub is_native: bool,
    /// The amount of `token` currently locked in the will, in the token's base units.
    /// A legacy mirror of `balances[token]` kept for backward compatibility;
    /// every writer that touches the primary token's balance must update both
    /// fields together until this mirror is fully removed.
    pub balance: i128,
    /// The visible beneficiaries of the will and how each is allocated.
    ///
    /// Entries may mix two allocation kinds (see [`Allocation`]), so this
    /// list does **not** always describe a split of 10,000 basis points.
    /// `assert_valid_allocations` enforces, on every write:
    /// - every `Allocation::Percentage(bp)` is non-zero, and the `Percentage`
    ///   entries together sum to exactly 10,000 basis points. This check only
    ///   applies when at least one `Percentage` entry is present — a
    ///   `FixedAmount`-only list has no percentage total to check;
    /// - every `Allocation::FixedAmount(amount)` is positive, and their sum
    ///   does not exceed the will's **primary-token** balance
    ///   ([`Will::token`]). A sum *below* that balance is allowed, since the
    ///   headroom is reserved for a later `add_hashed_beneficiary` and is
    ///   otherwise refunded to the owner at release;
    /// - no address appears twice ([`crate::WillError::DuplicateBeneficiary`]).
    ///
    /// So: percentages must sum to 10,000 when present, while fixed amounts
    /// are bounded above by the primary-token balance and need not account for
    /// the whole of it.
    pub beneficiaries: Vec<Beneficiary>,
    /// Privacy-preserving beneficiaries registered by commitment hash (issue #46).
    ///
    /// Each `HashedBeneficiary::percentage` is expressed in the same basis
    /// points (1 bp = 0.01 %, 10,000 = 100 %) as `Allocation::Percentage`, not
    /// in whole percent. They are withheld from the visible beneficiaries'
    /// share of each token and are claimable later via `reveal_and_claim` once
    /// the pre-image is revealed.
    ///
    /// `assert_valid_percentages` requires the visible `Percentage` entries
    /// plus every hashed percentage to total at most 10,000 basis points — the
    /// hashed entries take priority, and the remainder is what the visible
    /// percentage split divides. Unlike `beneficiaries`, a will with no
    /// percentage beneficiaries at all is legal, and its leftover primary-token
    /// balance is refunded to the owner on release.
    pub hashed_beneficiaries: Vec<HashedBeneficiary>,
    /// How many days the owner may go without checking in before the will
    /// can be triggered.
    pub checkin_period_days: u64,
    /// How many days after being triggered the owner has to prove they are
    /// alive (via `emergency_checkin`) before inheritance can be released.
    pub grace_period_days: u64,
    /// Unix timestamp (seconds) of the owner's last check-in.
    pub last_checkin: u64,
    /// Unix timestamp (seconds) at which the will was triggered, if any.
    pub trigger_time: Option<u64>,
    /// Unix timestamp (seconds) by which the owner must call `confirm_will`
    /// to move from `PendingConfirmation` to `Active` (issue #43).
    /// `None` once the will is confirmed or if no delay was requested.
    pub confirmation_deadline: Option<u64>,
    /// Current lifecycle state of the will.
    pub status: WillStatus,
    /// Optional guardians (up to 3) who may force an early release
    /// via a weight-based quorum using `guardian_trigger`.
    pub guardians: Vec<Guardian>,
    /// Accumulated weight of guardian votes cast in the current cycle.
    /// Quorum is reached when this reaches (or exceeds) `guardian_threshold`;
    /// see [`Will::guardian_threshold`] for the comparison and
    /// [`Will::guardian_votes`] for the corresponding head count.
    pub guardian_vote_weight: u32,
    /// Number of distinct guardians who have voted to trigger the current
    /// guardian-release cycle.
    pub guardian_votes: u32,
    /// Accumulated weight of guardian votes cast toward cancelling the current
    /// trigger. Reaches quorum at `guardian_threshold`, returning the will to
    /// `Active` and resetting the check-in deadline.
    pub guardian_cancel_vote_weight: u32,
    /// Number of distinct guardians who have voted to cancel the current trigger.
    pub guardian_cancel_votes: u32,
    /// The weight that a guardian quorum must accumulate to take effect.
    ///
    /// This is compared against the **accumulated vote weight**
    /// ([`Will::guardian_vote_weight`], and
    /// [`Will::guardian_cancel_vote_weight`] for the cancel path), not against
    /// [`Will::guardian_votes`]. With every guardian weighted 1 the two are
    /// equivalent, but a weighted guardian list installed via
    /// `update_guardians_weighted` lets a single guardian of weight 3 satisfy a
    /// threshold of 3 on its own.
    ///
    /// It is validated in `1..=total_weight`, where `total_weight` is
    /// `guardians.len()` for the unweighted lists that `create_will`,
    /// `update_guardians` and `update_will_settings` produce, and the sum of
    /// the supplied weights for a weighted list. Shrinking a non-empty guardian
    /// list below the stored threshold is rejected, since the quorum would
    /// become unreachable. An empty guardian list disables the mechanism and no
    /// threshold is checked.
    pub guardian_threshold: u32,
    /// Unix timestamp (seconds) of the last guardian-list change.
    /// `guardian_trigger` is only effective after a cooldown period has
    /// elapsed since this timestamp, preventing a compromised owner from
    /// swapping guardians right before a malicious action.
    pub guardian_list_updated_at: u64,
    /// Schema version for this will. Used to track which contract version
    /// wrote this state and enable forward/backward compatible migrations.
    /// See `migration.rs` and the "Storage schema versioning" section of
    /// CONTRIBUTING.md for how new versions are introduced.
    pub schema_version: u32,
    /// Optional keeper bounty in basis points (e.g., 10 = 0.1%).
    /// When set, a portion of the inheritance is paid to callers who trigger
    /// or release the will (not to the owner). Defaults to 0.
    pub keeper_bounty_bps: u32,
    /// Optional delegate address that may call `check_in` on the owner's
    /// behalf. `None` means only the owner can check in.
    pub delegate: Option<Address>,
}

/// Reasons a guardian can provide when casting a trigger vote.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardianVoteReason {
    /// The owner is confirmed deceased.
    Deceased = 0,
    /// The owner is incapacitated and unable to check in.
    Incapacitated = 1,
    /// The owner cannot be reached or located.
    Unreachable = 2,
    /// Any other reason not covered by the above.
    Other = 3,
}

/// A single entry in a will's on-chain audit trail, recording one status
/// transition. The full history of a will can be reconstructed by reading
/// all entries in insertion order.
#[contracttype]
#[derive(Clone, Debug)]
pub struct WillStatusTransition {
    /// The will this transition belongs to.
    pub will_id: u64,
    /// The status before the transition.
    pub from_status: WillStatus,
    /// The status after the transition.
    pub to_status: WillStatus,
    /// Unix timestamp (seconds) when the transition occurred.
    pub timestamp: u64,
    /// The address that initiated the transition, or the contract address
    /// for transitions triggered by anyone (e.g. `trigger_will`,
    /// `release_inheritance`).
    pub actor: Address,
    /// A short label describing what caused the transition
    /// (e.g. "create", "checkin", "trigger", "release").
    pub action: Symbol,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guardian_vote_reason_discriminants() {
        assert_eq!(GuardianVoteReason::Deceased as u32, 0);
        assert_eq!(GuardianVoteReason::Incapacitated as u32, 1);
        assert_eq!(GuardianVoteReason::Unreachable as u32, 2);
        assert_eq!(GuardianVoteReason::Other as u32, 3);
    }
}
