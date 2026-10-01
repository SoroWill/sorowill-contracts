# Contract event specification

Every state-changing entry point of the `will` contract publishes an event so
off-chain indexers can reconstruct a will's lifecycle without re-simulating
transactions. The source of truth is
[`contracts/will/src/events.rs`](../contracts/will/src/events.rs).

Each event has two topics: a short symbol naming the event and, unless noted,
the `will_id` (`u64`). The data column lists the tuple published as event
data, in order.

| Topic | Emitted by | Data |
| ----- | ---------- | ---- |
| `created` | `create_will`, `clone_will`, `batch_create_wills`, `split_will` | `(owner: Address, token_count: u32, beneficiaries: Vec<Beneficiary>, checkin_deadline: u64)` |
| `confirmed` | `confirm_will` | `owner: Address` |
| `checkin` | `check_in` | `(owner: Address, next_deadline: u64)` |
| `batchchk` | `batch_check_in` (second topic is `owner`) | `(will_ids: Vec<u64>, count: u32)` |
| `triggered` | `trigger_will` | `grace_period_ends: u64` |
| `emerg` | `emergency_checkin` | `(owner: Address, next_deadline: u64)` |
| `released` | `release_inheritance`, `guardian_trigger` (on quorum) | `(token_count: u32, beneficiaries_count: u32)` |
| `bounty` | `release_inheritance`, `guardian_trigger` (keeper bounty paid) | `(keeper: Address, amount: i128)` |
| `cancelled` | `cancel_will` | `(owner: Address, token_count: u32)` |
| `closed` | `close_will` | `owner: Address` |
| `archived` | `archive_will` | `(owner: Address, timestamp: u64, reason: Symbol)` |
| `benefup` | `update_beneficiaries` | `(owner: Address, beneficiary_count: u32, beneficiaries: Vec<Beneficiary>)` |
| `renounce` | `renounce_beneficiary` | `(beneficiary: Address, owner: Address, beneficiaries: Vec<Beneficiary>, trigger_time: Option<u64>)` — `trigger_time` is `Some(t)` when renounced during the grace period opened at `t`, `None` while `Active`. Renunciation is irreversible once emitted (#487). |
| `guardup` | `update_guardians` | `owner: Address` |
| `periodu` | `update_periods` | `(owner: Address, checkin_period_days: u64, grace_period_days: u64, next_deadline: u64)` |
| `setupd` | `update_will_settings` | `(owner: Address, updated_fields: Vec<Symbol>)` |
| `topup` | `top_up` | `(owner: Address, token: Address, amount: i128, new_balance: i128)` |
| `delegset` | `set_delegate` | `(owner: Address, delegate: Address)` |
| `delegclr` | `set_delegate` (clearing) | `owner: Address` |
| `gvote` | `guardian_trigger` | `(guardian: Address, weight: u32, total_weight: u32)` |
| `gcvote` | `guardian_cancel_trigger` | `(guardian: Address, weight: u32, total_weight: u32)` |
| `gcancel` | `guardian_cancel_trigger` (on quorum) | `(guardian: Address, next_deadline: u64)` |
| `merged` | `merge_wills` (topic is the surviving id) | `(owner: Address, consumed_will_id: u64, new_balance: i128)` |
| `migrated` | `migrate_will` | `(owner: Address, from_version: u32, to_version: u32)` |
| `cloned` | `clone_will` (topic is the new id) | `(source_id: u64, owner: Address)` |
| `batch` | `batch_create_wills` (second topic is `owner`) | `will_ids: Vec<u64>` |
| `split` | `split_will` (topic is the original id) | `(new_id: u64, owner: Address, split_amount: i128)` |
| `hclaim` | `reveal_and_claim` | `(claimant: Address, amount: i128)` |

## `archived`

`archive_will` moves a `Released` or `Cancelled` will out of active storage
and out of the owner and beneficiary indexes. After this event, `get_will`
returns `WillNotFound` and the will no longer shows up in `get_wills_by_owner`,
`get_wills_by_owner_and_status` or `get_wills_by_beneficiary`.

- `owner`: the will's owner.
- `timestamp`: ledger timestamp (Unix seconds) when the will was archived.
- `reason`: the terminal status the will was archived from, either
  `released` or `cancelled`.

Indexers should treat this as the will's final lifecycle event.

## Failed operations

A failed invocation rolls back all of its storage writes and events, so no
event is ever published for a failed operation. The same applies to
`get_protocol_stats`, which counts successful transitions only.
