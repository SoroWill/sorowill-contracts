# Resource costs

Soroban meters every invocation and bills for it: CPU instructions, the number
of ledger entries touched, their size in bytes, and rent for keeping persistent
entries alive. This document records what each SoroWill entry point costs, how
to measure it yourself, and the reasoning behind the storage layout decisions
that drive those numbers.

## Measuring

The profile lives in [`contracts/will/src/profile.rs`](../contracts/will/src/profile.rs)
and runs as part of the normal test suite. To see the table:

```sh
cargo test -p will --lib profile -- --nocapture
```

It reports, per entry point, the figures from `Env::cost_estimate()`: metered
instructions, ledger entries read and written, bytes read and written,
cumulative rent in ledger-bytes, and the fee those resources would attract at a
snapshot of Stellar pubnet rates.

Two caveats. The contract is registered natively rather than as Wasm, so
everything the host charges for reading, instantiating and running the Wasm
module is missing from `instructions`; ledger-entry counts and byte sizes are
unaffected. And resource estimation is approximate by design — the SDK points
at RPC simulation for the exact resources of a real submission. The numbers are
therefore a good basis for comparing a change against its baseline, and a poor
basis for predicting an exact bill.

`read_entries` counts entries read but *not* modified, so an entry point's
total ledger footprint is `read_entries + write_entries`.

## Current profile

Measured with 5 beneficiaries and 2 guardians.

| entry point | instructions | read entries | write entries | read bytes | write bytes | rent ledger-bytes | fee (stroops) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| create_will | 410549 | 3 | 11 | 888 | 2720 | 2239684560 | 14088200 |
| check_in | 119505 | 2 | 2 | 1268 | 1188 | 0 | 1339630 |
| get_will | 59019 | 2 | 0 | 1268 | 0 | 0 | 14860 |
| top_up | 265035 | 3 | 4 | 2184 | 1636 | 0 | 1388170 |
| trigger_will | 106282 | 1 | 1 | 1268 | 1128 | 12441600 | 119028 |
| emergency_checkin (no votes cast) | 121547 | 2 | 2 | 1280 | 1188 | 0 | 1339656 |
| release_inheritance | 824313 | 3 | 7 | 2064 | 2468 | 580608000 | 3467971 |
| cancel_will | 255576 | 3 | 4 | 2104 | 1560 | 4147200 | 1420633 |
| update_beneficiaries (shares only) | 155492 | 2 | 2 | 1268 | 1188 | 0 | 1339485 |
| update_beneficiaries (full replacement) | 501292 | 2 | 12 | 2048 | 2688 | 808704000 | 6081021 |
| update_guardians (no votes cast) | 122944 | 2 | 2 | 1268 | 1188 | 0 | 1339404 |
| guardian_trigger (below threshold) | 154493 | 2 | 3 | 1268 | 1336 | 153446400 | 2223292 |
| guardian_trigger (reaches threshold) | 892299 | 4 | 9 | 2052 | 2680 | 734054400 | 5653138 |
| update_guardians (clearing a vote) | 140029 | 2 | 4 | 1416 | 1188 | 0 | 1372205 |
| get_wills_by_owner (1 will) | 70897 | 3 | 0 | 1340 | 0 | 0 | 21266 |
| get_wills_by_beneficiary (1 will) | 71307 | 3 | 0 | 1344 | 0 | 0 | 21274 |
| check_in (rent due) | 123028 | 2 | 2 | 1188 | 1108 | 828800000 | 5967529 |
| cancel_will (rent due) | 266013 | 3 | 4 | 2104 | 1560 | 180147200 | 2422527 |

### Why two rows for some calls

Most rows run against entries written moments earlier, whose remaining lifetime
still exceeds the 30-day extension threshold. `extend_ttl` is a no-op there, so
rent reads zero. The `(rent due)` rows age the ledger past that threshold first
and show what the same call costs on the roughly monthly occasion that does
have to top the will's rent up — for `check_in`, about 4.5x a free one. That
periodic top-up, not the instruction count, is the dominant long-run cost of
holding a will open.

## What each entry point touches

The reverse indexes (`OwnerWills`, `BeneficiaryWills`) exist so the contract can
answer "which wills am I named in" without an off-chain indexer. They are the
main source of storage traffic beyond the will entry itself, which is why most
of the tuning below concerns them.

- **`check_in` / `get_will`** — the hot paths, and already minimal: one read of
  the will entry, and for `check_in` one write back. Neither touches an index.
  See [below](#what-was-considered-and-rejected) for why they were left alone.
- **`create_will`** — unavoidably the most expensive call: it writes the will,
  the owner index, one index per beneficiary, and moves tokens.
- **`update_beneficiaries`** — cost depends entirely on how much the list
  actually changed, not on how large it is.
- **`guardian_trigger`** — cheap below the threshold; the vote that reaches
  quorum also distributes, so it carries the release cost too.
- **`get_wills_by_*`** — one read of the index plus one per will listed. Linear
  in the caller's number of wills, and read-only.

## Tuning applied

Five changes, each measured against the commit before them.

### Terminal wills stop paying rent

`Released` and `Cancelled` are terminal states — every entry point that could
touch a will again first asserts it is `Active` or `Triggered`. `save_will` no
longer extends the TTL of a will in either state, so the contract stops buying
60 more days of rent for an entry that can never change again.

| | before | after |
| --- | ---: | ---: |
| `cancel_will (rent due)` rent ledger-bytes | 1012147200 | 180147200 |
| `cancel_will (rent due)` fee | 7058777 | 2422527 |

A 66% fee reduction on that path, and the largest single saving in this pass.
The residual rent is the token contract's, not the will's.

**This is a deliberate behaviour change.** A terminal will keeps whatever
lifetime its last active operation bought — up to 60 days — and is then allowed
to archive rather than being renewed indefinitely. `get_will` on a long-settled
will may therefore need the entry restored. Reconstructing history from events
is unaffected: every state change still publishes one.

### Beneficiary updates only touch indexes that changed

`update_beneficiaries` used to remove the reverse index of every current
beneficiary and then re-add every new one. When an address appeared in both
lists — the common case, since most updates only re-cut the percentages — that
was a storage read and write per address for no net effect. Membership is now
decided against the two lists already in memory, at no storage cost.

| `update_beneficiaries (shares only)` | before | after |
| --- | ---: | ---: |
| write entries | 7 | 2 |
| instructions | 413024 | 155492 |
| fee | 1431880 | 1339485 |

The list is scanned quadratically to decide membership, which costs a full
replacement of 5 beneficiaries about 47,000 extra instructions. At 25 stroops
per 10,000 instructions that is 118 stroops, against 10,000 stroops for each
write entry it can avoid — so the trade is worth taking even when it does not
pay off. The measured worst case is `update_beneficiaries (full replacement)`:
+10.4% instructions, +0.002% fee.

### Guardian votes are only cleared when some exist

`will.guardian_votes` moves in lockstep with the per-guardian `GuardianVote`
markers, so a zero count means there is nothing to remove. `reset_guardian_votes`
now returns immediately in that case instead of issuing a storage removal per
guardian — the common case, since most wills never see a guardian vote.

| | before writes | after writes | before fee | after fee |
| --- | ---: | ---: | ---: | ---: |
| `emergency_checkin (no votes cast)` | 4 | 2 | 1372206 | 1339656 |
| `update_guardians (no votes cast)` | 4 | 2 | 1371962 | 1339404 |

`update_guardians (clearing a vote)` is unchanged, as it should be.

### The quorum-reaching guardian vote saves the will once

`guardian_trigger` wrote the will entry and then called `distribute`, which
wrote it again. The ledger footprint was already correct — the host counts
entries, not `set` calls — but the second serialisation was pure CPU.

| `guardian_trigger (reaches threshold)` | before | after |
| --- | ---: | ---: |
| instructions | 947917 | 892299 |

### Removals that change nothing do not write

`remove_beneficiary_index` rebuilt the index list and wrote it back even when
the will id was not in it. It now returns after the read. It also removes in
place instead of building a replacement list.

## What was considered and rejected

**Splitting the will entry into hot and cold halves.** `check_in` reads and
writes the whole `Will`, including the beneficiary and guardian lists it never
looks at. Storing the mutable fields (`last_checkin`, `status`, `trigger_time`,
`guardian_votes`, `balance`) separately from the immutable ones would cut its
write bytes by roughly 5x. It was rejected because `get_will` — the other
function this pass was asked to focus on — would need two reads instead of one
and pay the per-entry read fee twice, and because it is a storage layout change
requiring migration for any already-deployed will. Worth revisiting before
mainnet, when there is nothing to migrate.

**Storing `Will` as a tuple struct.** `#[contracttype]` encodes a named-field
struct as a map keyed by field-name symbols, so every read and write of a will
carries roughly 120 bytes of field names. A tuple struct would encode as a
vector and drop them. Rejected as a bad trade against the readability of the
type and its generated client bindings.

**Bumping `check_in` further.** After the changes above, `check_in` is one read
and one write of a single entry, and `get_will` is one read and no writes.
Neither has any redundant storage access left to remove; both were already at
that floor before this pass. Their numbers are unchanged, and the honest
finding is that the waste was elsewhere.

## `create_will` cost as protocol-wide distinct tokens grow

`ProtocolStats.total_locked_by_token` is a `Vec<TokenLockedBalance>` stored as
a single instance-storage entry. Every `create_will`, `top_up`, and
`cancel_will` call reads the whole entry, linearly scans it to find a matching
token, rebuilds the entire vector, and writes it back. The CPU and byte cost of
that operation scales with `N`, the number of **distinct token addresses ever
used across all wills on the protocol** — not the number of tokens in the will
being created.

To measure this effect, run the profile scenario `create_will (N protocol
tokens)` in `profile.rs` (see below for how to add it). The scenario pre-seeds
the `ProtocolStats` entry with varying values of `N` before calling
`create_will`, then records cost at each step.

### Expected growth

Each `TokenLockedBalance` entry serialises to roughly **72 bytes** (a 56-byte
`Address` plus a 16-byte `i128`). The table below shows the expected minimum
read/write byte growth on top of the baseline `create_will` cost (410 549
instructions, 888 read bytes, 2 720 write bytes at N = 1):

| N distinct protocol tokens | extra read bytes | extra write bytes | notes |
| ---: | ---: | ---: | --- |
| 1 (baseline) | 0 | 0 | current profile row |
| 10 | ~648 | ~648 | 9 extra entries × 72 bytes |
| 50 | ~3 528 | ~3 528 | |
| 100 | ~7 128 | ~7 128 | instance entry approaches 8 KB |
| 200 | ~14 328 | ~14 328 | instance entry exceeds 16 KB |

Instructions grow proportionally — each extra entry requires an address
comparison plus a clone. At 25 stroops per 10 000 instructions the instruction
cost is secondary to the byte cost, but both contribute to the fee.

### Why the current profile does not show this

The existing `profile_lifecycle` scenario creates one will with one token into a
fresh environment, so `N` is always 1. The growth is invisible unless the
protocol state is pre-seeded with many prior distinct tokens. Adding a
`profile_create_will_token_scaling` scenario to `profile.rs` that pre-seeds
`ProtocolStats` with 1, 10, 50, and 100 entries before each measurement would
make this growth visible in CI output. The scenario is not included in the
current profile because it requires no code change to understand the problem —
the growth follows directly from the linear-scan implementation — but it should
be added alongside any fix so the improvement can be measured against the
baseline.

### How to add the scenario

```rust
// In profile.rs — illustrative, not yet wired into profile_public_entry_points
fn profile_create_will_token_scaling(report: &mut Report) {
    for n in [1_u32, 10, 50, 100] {
        let f = fixture();
        // Pre-seed ProtocolStats with `n - 1` synthetic token entries so the
        // next create_will sees a vector of length `n - 1` before adding its own.
        f.env.as_contract(&f.client.address, || {
            use crate::storage::{DataKey, save_protocol_stats, get_protocol_stats};
            let mut stats = get_protocol_stats(&f.env);
            for _ in 1..n {
                stats.total_locked_by_token.push_back(
                    crate::types::TokenLockedBalance {
                        token: Address::generate(&f.env),
                        total_locked: 1,
                    }
                );
            }
            save_protocol_stats(&f.env, &stats);
        });
        f.create(&Vec::new(&f.env));
        let label = std::format!("create_will ({n} protocol tokens)");
        // report.record requires a &'static str; use a fixed set of labels
        // or extend Report to accept String.
    }
}
```

See [`docs/adr/0002-total-locked-by-token-scalability.md`](adr/0002-total-locked-by-token-scalability.md)
for the full analysis, options considered, and recommended fix.

## An observation, not addressed here

Nothing in the contract renews its own instance entry. A contract instance is
written once at deployment with a short default lifetime, and if it archives,
every call fails until someone restores it. Most contracts call
`env.storage().instance().extend_ttl(...)` in their entry points to prevent
this. The profile has to renew the instance from the test side to make its
long-jump scenarios work at all, which is how this surfaced.

Adding those bumps would *increase* per-call cost, so it does not belong in a
pass about reducing it — but it is a liveness question worth a separate look.
