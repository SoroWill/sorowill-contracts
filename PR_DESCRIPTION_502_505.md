fix(protocol): decrement locked-value totals on release, and pin the invariants from #502–#505

Fixes a real accounting bug found while adding regression tests for #502–#505,
then adds those tests.

## The bug: `total_locked_by_token` was never decremented on release

`storage::adjust_locked_value` is the single write path for
`get_protocol_stats().total_locked_by_token`. It was called on `create_will`,
`top_up`, and `cancel_will` — but **not** on `release_inheritance`. Releasing a
will pays its balances out to beneficiaries through `distribute`, so the protocol
total stayed at its pre-release value forever:

```
create_will(200_000)  -> total_locked_by_token = 200_000
release_inheritance() -> total_locked_by_token = 200_000   // should be 0
```

This was not a latent guess. `issue_427_test::release_decrements_counter_to_zero`
already asserted the correct behaviour and had been **failing on `main`**:

```
---- issue_427_test::release_decrements_counter_to_zero stdout ----
assertion `left == right` failed: counter must be zero after releasing a will
  left: 200000
 right: 0
```

That test file's own documentation states the intended invariant explicitly:

> Decremented by the same amount at `cancel_will` time (one call per token the
> will held) **and at `release_inheritance` / `guardian_trigger` time (one call
> per token, via `distribute`)**.

So the contract was expected to do this and simply did not. `main` is currently
red on `Test` and `Coverage`; this turns both green. The fix mirrors
`cancel_will`: decrement every token the will held, before `distribute` performs
the transfers, preserving changes-then-interactions ordering.

## #502–#505: reported against code that has since changed

All four were filed against an older version of `merge_wills`, `split_will`, and
`release_inheritance`. Each invariant they ask for is now enforced elsewhere, so
rather than re-implement anything, this PR **pins each one with a test** — which
is also the "Test covers …" acceptance criterion every one of them asks for.

| Issue | Reported gap | Where it is actually enforced now |
| --- | --- | --- |
| #502 | `merge_wills` does not check both wills share the owner | `load_owned` for `will_id_a` (`NotOwner`), explicit `will_b.owner` check (`NotSameOwner`) |
| #503 | duplicate beneficiaries get two entries | `merge_beneficiaries` accumulates shares per address (added for #382) |
| #504 | `split_will` does not check address uniqueness | `split_uniqueness_check::assert_split_addresses_unique` (`InvalidSplit`), then `BeneficiaryNotFound` |
| #505 | `release_inheritance` does not validate the triggered record | the `WillStatus::Triggered` assertion, since the first release moves the will to `Released` |

On #505 specifically, there is no `triggered_wills` map to clean up — the only
such thing is the `get_triggered_wills()` view, which filters on status, so the
transition to `Released` is what removes a will from it. The tests assert that
too.

## Tests

`contracts/will/src/issues_502_505_test.rs`, 10 tests:

- **#502** — merge rejects a will owned by someone else (both argument orders),
  and still succeeds when both are mine. The cross-owner case is the actual
  attack: guessing another owner's will id.
- **#503** — a beneficiary in both wills ends up as exactly one entry, with
  percentages still totalling 100%; plus a three-way merge, which is where a
  naive de-duplication would still leak a second entry.
- **#504** — duplicate addresses and addresses not on the source are both
  rejected, and a valid split succeeds.
- **#505** — a second `release_inheritance` is rejected, the will leaves the
  triggered view, and releasing inside the grace period is rejected.

Each rejection asserts the **specific** `WillError`, following the existing idiom
in `issue_357_test.rs` (`.unwrap_err().unwrap()` then `assert_eq!`). A bare
`is_err()` would have passed even when the call failed for an unrelated reason —
two of these tests initially did exactly that, and tightening them is what caught
that the cross-owner path raises `NotSameOwner` rather than `NotOwner`, and that
duplicate split addresses are caught by the uniqueness helper as `InvalidSplit`
before the explicit duplicate check.

## Verification

- `cargo test --lib` — **285 passed, 0 failed** (was 274 passed / 1 failed on
  `main`; +10 new).
- `cargo clippy --all-targets` — no new warnings; my test file is clean.
- `cargo fmt` applied to the new file only. Two unrelated test files
  (`issue_425_test.rs`, `issue_426_test.rs`) are not formatted correctly on
  `main`; `cargo fmt` wanted to rewrite them and those changes were reverted, so
  this PR does not touch them.

## Note for maintainers

If these four issues are still considered open, the tests above are evidence the
behaviour is present, not a claim that the reports were wrong — the code changed
underneath them. They can be closed on that basis. If a maintainer believes any
of the four invariants is still wrong, the failing assertion identifies exactly
which check is missing.

closes #502
closes #503
closes #504
closes #505
