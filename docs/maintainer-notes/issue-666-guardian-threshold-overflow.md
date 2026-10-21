# Issue #666: Guardian Threshold and Weight Overflow Prevention

## Context
When computing accumulated guardian vote weights during `guardian_trigger`, `guardian_cancel_trigger`, and `live_guardian_vote_weight`, weights were previously added using primitive arithmetic (`weight += guardian.weight`).

If a guardian is configured with `weight = u32::MAX` or multiple guardians' weights sum up to exceed `u32::MAX`, standard integer addition triggers an arithmetic overflow panic in Soroban contracts.

## Resolution
1. Replaced primitive addition in `storage::tally_guardian_votes` with `weight = weight.saturating_add(guardian.weight)`.
2. Replaced primitive addition in `storage::tally_guardian_cancel_votes` with `weight = weight.saturating_add(guardian.weight)`.
3. Replaced primitive addition in `guardian_vote_freshness::live_guardian_vote_weight` with `total = total.saturating_add(g.weight)`.
4. Aligned tally functions with existing `recount_guardian_votes` saturating arithmetic, ensuring consistent behavior across all voting pathways.
