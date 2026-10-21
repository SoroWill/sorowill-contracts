# Pagination consistency guarantees (#591)

`get_wills_by_owner` (and the other owner/beneficiary/triggered listings) page
through an ascending index of will ids. The `cursor` is a **value bound**, not
a position: a page contains the ids strictly greater than `cursor`, in
ascending order (`storage::paginate_ids`, #492).

Because will ids are monotonically increasing and the index is kept sorted
(removals are order-preserving), the following hold when the collection changes
between two page requests:

- **Added wills** get a higher id than every existing one, so they appear on a
  later page. They are never inserted before the cursor, so no entry is
  repeated.
- **Removed wills**, including the will the cursor points at, do not shift
  anything: the next page resumes at the first remaining id above the cursor.
  Nothing still present is skipped or repeated.
- **A cursor at or beyond the largest id** returns an empty page, so a stale
  cursor never errors.
- **Status changes** do not move a will within the index, but the `Will`
  returned for an id always reflects its state at the time of the call.

## What is not guaranteed

Pages are not a snapshot. A client that wants a consistent view must tolerate
that a will present on an earlier page may have changed or been removed by the
time a later page is read, and that wills created mid-pagination appear only on
later pages (or are missed if paging has already ended). For totals use
`get_owner_stats`. There is no cursor versioning: because the cursor is an id
bound rather than an index offset, none is needed to avoid skips or repeats.
