use soroban_sdk::{contracttype, Address, Bytes, Map, Symbol, Vec};

/// How a beneficiary's share of a will is calculated.
///
/// - `Percentage(bp)` is a share expressed in basis points (1 bp = 0.01 %) of
///   whatever balance remains *after* every `FixedAmount` beneficiary on the
///   same will has been paid. All `Percentage` shares on a will must sum to
///   exactly 10,000 (100 % of the remainder).
/// - `FixedAmount(amount)` entitles the beneficiary to exactly `amount` of
///   the will's token, paid before any percentage-based split is computed.
///   The sum of all `FixedAmount` entries on a will can never exceed the
///   will's balance (enforced by `assert_valid_allocations`).
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
    Pending,
    /// Guardian has accepted the role and may vote.
    Accepted,
    /// Guardian has declined the role.
    Rejected,
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

/// Consent status of a designated guardian.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardianConsent {
    Pending = 0,
    Accepted = 1,
    Rejected = 2,
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
/// Active --(missed check-in)--> Triggered --(grace period expires)--> Released --(close_will)--> Settled
///   |                               |
///   |--(cancel_will)--> Cancelled   |--(emergency_checkin)--> Active
///   |--(partial_release)--> Active  (balance reduced, subset paid)
/// ```
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
    /// The beneficiaries and their basis-point shares. Always sums to 10,000.
    pub beneficiaries: Vec<Beneficiary>,
    /// Privacy-preserving beneficiaries registered by commitment hash (issue #46).
    /// Their percentages count towards the 100-sum together with `beneficiaries`.
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
    /// Release triggers when this reaches `guardian_threshold`.
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
    /// Number of distinct guardian votes required to force an early release.
    /// Must be between 1 and `guardians.len()`.
    pub guardian_threshold: u32,
    /// Unix timestamp (seconds) of the last guardian-list change.
    /// `guardian_trigger` is only effective after a cooldown period has
    /// elapsed since this timestamp, preventing a compromised owner from
    /// swapping guardians right before a malicious action.
    pub guardian_list_updated_at: u64,
    /// Schema version for this will. Used to track which contract version
    /// wrote this state and enable forward/backward compatible migrations.
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
