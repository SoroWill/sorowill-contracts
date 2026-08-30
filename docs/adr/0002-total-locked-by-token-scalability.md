# ADR 0002: `total_locked_by_token` — Bounded Vec vs. Unbounded Growth

## Status

Open — fix not yet implemented.

## Context

`ProtocolStats` carries a `total_locked_by_token: Vec<TokenLockedBalance>`
that aggregates, for every distinct token ever locked by any will, the current
sum of amounts held across all active wills.  It is stored in **instance
storage** and updated on every `create_will`, `top_up`, and `cancel_will`.

The helper that updates it is `storage::adjust_locked_value`:

```rust
// storage.rs — current implementation
pub fn adjust_locked_value(env: &Env, token: &Address, delta: i128) {
    let mut stats = get_protocol_stats(env);
    let mut found = false;
    let mut updated = Vec::new(env);
    for entry in stats.total_locked_by_token.iter() {   // O(N) scan
        if entry.token == *token {
            updated.push_back(TokenLockedBalance {
                token: entry.token.clone(),
                total_locked: entry.total_locked + delta,
            });
            found = true;
        } else {
            updated.push_back(entry.clone());            // full Vec rebuild
        }
    }
    if !found {
        updated.push_back(TokenLockedBalance {
            token: token.clone(),
            total_locked: delta,
        });
    }
    stats.total_locked_by_token = updated;
    save_protocol_stats(env, &stats);
}
```

Two costs scale with `N`, the number of **distinct token addresses ever used
across all wills on the protocol**:

1. **CPU instructions** — every element in the vector is compared and cloned.
   The Soroban instruction meter charges for both.
2. **Read and write bytes** — `get_protocol_stats` deserialises the full
   `ProtocolStats` entry (including every `TokenLockedBalance` tuple) from
   instance storage, and `save_protocol_stats` serialises the rebuilt vector
   back.  The byte cost therefore grows as `O(N × sizeof(TokenLockedBalance))`.

Neither `N` nor the byte size of the entry has an upper bound today.  Any
address on the Stellar network can create a will that references a novel token,
permanently adding one entry to the vector and making every subsequent
`create_will`, `top_up`, and `cancel_will` slightly more expensive — for
everyone, regardless of which tokens their own wills use.

### Additional issue discovered during analysis

`cancel_will` currently calls `adjust_locked_value` only for `will.token` (the
legacy single-token field retained for backward compatibility), not for all
entries in `will.balances`.  For a multi-token will this means the
`total_locked` counters for every non-primary token are **never decremented on
cancellation**, so the aggregate totals drift upward over time.  The same
omission may exist in `release_inheritance` / `distribute` (no call to
`adjust_locked_value` was found there at all).  These are correctness bugs
compounding the scalability issue: the vector grows unboundedly *and* its
values become wrong for multi-token wills.

## Decision drivers

* Soroban charges for bytes read and written per invocation; a growing entry
  inflates the fee of every call that touches `ProtocolStats`, not just the
  ones that add new tokens.
* Instance storage is read as a single ledger entry; there is no partial-read
  API.  A large `total_locked_by_token` vector means the entire entry is
  deserialised on every call, even if only one token needs updating.
* The contract already caps per-will token diversity at `MAX_TOKENS = 10`.
  That cap does *not* bound `N` — it only limits how many tokens a single new
  will can add in one call.
* An off-chain consumer (SDK, explorer) only needs the aggregate totals for
  `get_protocol_stats`; it does not need them to be recomputed in-process.

## Options considered

### Option A — Enforce a hard cap on `N` (document / enforce)

Add a constant `MAX_PROTOCOL_TOKENS: u32` checked inside `adjust_locked_value`
before inserting a new entry.  If the cap is reached, either:

* (A1) panic with a new `WillError::TooManyProtocolTokens`, or
* (A2) silently skip the update (the total becomes approximate but the call
  succeeds).

**Pros:** trivial to implement; makes worst-case cost predictable at
compile time; no data-structure migration.

**Cons (A1):** a sufficiently popular protocol with many distinct tokens would
eventually refuse new `create_will` calls for novel tokens, which is a
liveness issue the owner cannot work around.

**Cons (A2):** silent data loss; aggregate totals become unreliable.

### Option B — Switch `total_locked_by_token` to a `Map<Address, i128>`

Replace `Vec<TokenLockedBalance>` with a Soroban `Map<Address, i128>` keyed by
token address.  The update becomes a single `get`/`set` pair instead of a
full-scan rebuild:

```rust
// proposed
pub fn adjust_locked_value(env: &Env, token: &Address, delta: i128) {
    let mut stats = get_protocol_stats(env);
    let current = stats.total_locked_by_token.get(token.clone()).unwrap_or(0);
    stats.total_locked_by_token.set(token.clone(), current + delta);
    save_protocol_stats(env, &stats);
}
```

CPU cost drops from `O(N)` to `O(log N)` (Soroban `Map` is an ordered map with
`O(log N)` access).  Byte traffic still grows as more distinct tokens are added,
because the whole `ProtocolStats` entry is still serialised as one ledger entry,
but that is now irreducible: any representation that stores N token totals must
carry at least N entries.

**Pros:** eliminates the avoidable quadratic work; consistent with how `Will`
already stores `balances: Map<Address, i128>`; no new error variants or liveness
risk; the `Vec<TokenLockedBalance>` type returned by `get_protocol_stats` can
be reconstructed on read if backward-compatible serialisation is needed.

**Cons:** requires a storage-layout migration for the `ProtocolStats` instance
entry (the `contracttype` encoding of `Map` differs from `Vec`); the byte
footprint of instance storage still grows with `N`, just unavoidably so.

### Option C — Move per-token totals out of instance storage

Store each token's aggregate as a separate instance-storage key
`DataKey::LockedByToken(Address)`.  `adjust_locked_value` then reads and writes
exactly one small entry regardless of how many distinct tokens the protocol
has ever seen.  `get_protocol_stats` becomes a scan across all
`LockedByToken` keys, which is not directly supported by the Soroban storage
API (no enumerate/scan).

**Pros:** per-call read/write bytes are constant in `N`; no single bloated
entry.

**Cons:** Soroban's storage API does not expose key enumeration, so
`get_protocol_stats` would have to reconstruct the list from an explicit index
(a `Vec<Address>` of all tokens seen), reintroducing a separate growing
structure.  The complexity cost shifts rather than disappears.  Rejected for
now; worth revisiting if the number of distinct tokens actually approaches
practical instance-storage size limits.

## Recommended approach

**Option B**, with a documented soft cap as an operational guardrail.

1. Change `ProtocolStats.total_locked_by_token` from `Vec<TokenLockedBalance>`
   to `Map<Address, i128>`.  The existing `TokenLockedBalance` struct is kept
   for the `get_protocol_stats` return type (reconstructed on read from the
   map) so the public interface remains backward-compatible.
2. Rewrite `adjust_locked_value` to use a single map `get`/`set`.
3. Fix `cancel_will` and `release_inheritance`/`distribute` to call
   `adjust_locked_value` for **every token in `will.balances`**, not just
   `will.token`.
4. Document a soft operational limit (e.g. "the protocol works correctly at any
   `N`; beyond ~200 distinct tokens the instance entry exceeds 8 KB and reads
   become measurably more expensive — operators should monitor `N` and consider
   a stats-reset migration if it grows beyond that range").

### Why not Option A

Capping `N` at a fixed number and panicking creates a latent liveness hazard:
once the cap is hit, no owner can create a will with a token not already in the
vector, regardless of how many of those existing wills are still active.  On
a long-lived protocol this is unacceptable.  The `MAX_TOKENS = 10` per-will cap
provides a natural rate-of-growth bound (`N` grows by at most 10 per `create_will`
call) without creating a hard ceiling on `N` itself.

## Consequences

### If Option B is implemented

* `adjust_locked_value` drops from `O(N)` to `O(log N)` in CPU instructions.
* The `ProtocolStats` instance entry still grows with `N`, but that growth is
  now the unavoidable minimum for storing the data; there is no recoverable
  overhead on top of it.
* A one-time storage migration is needed for any deployed instance.  The
  migration path is: read the current `Vec<TokenLockedBalance>`, construct
  a `Map<Address, i128>`, write the new `ProtocolStats`.  This can be done via
  a contract upgrade and a dedicated `migrate_protocol_stats` admin call.
* The `get_protocol_stats` return type stays `ProtocolStats` with
  `total_locked_by_token: Vec<TokenLockedBalance>` (reconstructed from the map
  at query time) so that callers see no breaking change.

### If the issue is left open

Every `create_will`, `top_up`, and `cancel_will` call becomes permanently and
unavoidably more expensive as the protocol matures, with no action the caller
can take to avoid it.  At ~100 distinct tokens the extra deserialization is a
minor annoyance; at ~1 000 it is a meaningful fraction of the total invocation
fee.

## Related

* `docs/RESOURCE_COSTS.md` — profiles `create_will` cost at baseline; a
  supplementary scenario measuring cost as `N` grows is tracked there.
* `contracts/will/src/storage.rs` — `adjust_locked_value`, `get_protocol_stats`,
  `save_protocol_stats`.
* `contracts/will/src/types.rs` — `ProtocolStats`, `TokenLockedBalance`.
* `contracts/will/src/lib.rs` — `create_will` (calls `adjust_locked_value` per
  token), `cancel_will` (calls it only for `will.token`, missing `will.balances`
  entries), `top_up` (calls it once for the topped-up token).
