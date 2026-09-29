# Maintainer notes (rdj-savyy)

## Issue #416: archive_will leaves guardian votes and history behind

Open PR #408 ("Drop history and guardian vote entries when archiving a will") already removes the `WillHistory` entry and every `GuardianVote` / `GuardianCancelVote` entry in `archive_will`, documents that history does not survive archival, and adds tests in `archive_will_test.rs`. On main, `archive_will` already removes the will from the owner, beneficiary and Triggered indexes. No duplicate code is added here; merging #408 resolves this issue.

## Issue #417: get_wills_by_owner ordering and cursor stability

Owner wills are held in the `OwnerWills` index, a `Vec<u64>` to which ids are only ever appended. Will ids come from a monotonically increasing counter, so the index is always in ascending will-id order, and `remove_owner_index` filters while preserving order. `storage::paginate_ids` resumes strictly after the cursor id (`id <= cursor` is skipped), so the cursor is keyed by will id rather than by position. A will created between two page calls receives a larger id than any existing one and therefore lands after the cursor, so pages neither skip nor duplicate entries. The guaranteed order is: ascending by will id, cursor exclusive. No code change is needed.

## Issue #418: beneficiary account validation in release_inheritance

Soroban contracts cannot verify on-chain that a Stellar account is funded or initialized, and no such host function exists. This issue offers documenting that limit as an alternative to validating, and that is the resolution here: the contract validates only that beneficiary addresses are well-formed, distinct and that allocations sum correctly (`create_will` / `update_beneficiaries`). It does not check that a beneficiary account exists or can receive the token. Clients and owners must confirm each beneficiary is a funded account (and has a trustline for non-native assets) before naming it. Tokens sent to an address nobody controls are unrecoverable. No code change is made, to avoid a false sense of safety.

## Issue #419: check_in_history staleness

On main there is no separate `check_in_history` field: `get_will` returns the single `Will` record, and `check_in` (and `batch_check_in`) write `last_checkin` and the will in one `storage::save_will` call. Both check-ins in a sequence are therefore reflected atomically in the returned will; there is nothing to go stale. The status-transition trail is a separate `WillHistory` entry read by `get_will_history`; open PR #407 bounds that entry and adds a paged read. No code change is needed here.
