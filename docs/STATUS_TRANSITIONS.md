# Will status transition atomicity (#663)

Every lifecycle change to an existing will must use
`status_transition::transition` (or its non-panicking test/helper form,
`try_transition`) rather than assigning `will.status` and saving the snapshot
directly. The helper accepts the statuses that may transition from and the
existing error to return when the transition is no longer valid.

The helper verifies both that the caller's snapshot has an allowed status and
that the status persisted for that will still matches the snapshot. Only then
does it set the new status and persist the will. If the check fails, it makes
no write. This protects lifecycle state from a stale snapshot and preserves the
specific error behavior of each entry point. New wills are initialized with
their first status; same-status updates such as check-ins are not lifecycle
transitions.

Soroban executes transactions serially and rolls back all state changes when
an invocation fails. Transactions therefore cannot interleave between a
status check and update. The storage comparison is defense in depth for stale
snapshots within an invocation (including future code that introduces an
intervening call); it is not a cross-transaction lock or a general
compare-and-swap primitive. Callers must do their validation before the
transition and must not make external calls between validating and
transitioning. The helper compares only `status`, so callers must not use a
stale snapshot to overwrite unrelated fields that may have changed.

The regression tests simulate an intervening status change by retaining a
snapshot, moving the stored will to a different status, and then attempting
the old transition. The stale transition must fail and leave the newer
persisted status intact. Other lifecycle tests verify that repeated or
conflicting actions cannot apply a transition twice or duplicate its history
entry.
