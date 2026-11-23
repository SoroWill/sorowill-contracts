# Contract execution profile

Run `cargo test -p will --test benchmarks -- --nocapture` to print CPU
instructions, read and write counts, bytes written, and fee estimates for
every public will function. The test checks that each public entry point
appears in the report. It uses a ten-beneficiary will, two guardians, and a
native Stellar asset contract. The release scenario includes one rejected
token transfer followed by a successful retry. The benchmark measures
`migrate_will` with the fixture as stored by the current contract.

## Bottlenecks

| Scenario | Instructions | Ledger writes | Bytes written |
| --- | ---: | ---: | ---: |
| Release inheritance, including one failed transfer | 4,637,144 | 26 | 9,120 |
| Merge two wills | 2,779,055 | 24 | 9,512 |
| Replace all beneficiaries | 2,093,997 | 24 | 5,692 |

Release had the highest CPU instruction count and write count in this
fixture. It builds the payout plan, updates ledger state and invokes the token
contract for each payout. Merge wrote the most bytes. Replacing an entire
beneficiary list rewrites the departing and incoming beneficiaries' indexes.

These are native Soroban SDK 28 test-host measurements. The numbers exclude
Wasm execution and use the SDK's fee schedule and ledger fixture. Actual
resource limits and fees depend on network configuration and ledger state.

## Owner pagination

Owner indexes store IDs in increasing order. A binary search now finds the
first ID after the cursor before slicing the requested page. The code uses the
insertion position if the will corresponding to the cursor has been removed.

The stress test queried the last 10 rows of a 1,000-will owner index after
using the 990th row's ID as the exclusive cursor. Both runs returned the
same rows and read 12 in-memory entries without writing any ledger entries.

| Cursor search | Instructions |
| --- | ---: |
| Linear scan | 1,093,228 |
| Binary search | 918,039 |

The binary search cut instructions by 16.0% for that page. The one-page
owner query in the small benchmark went from 265,409 to 263,196
instructions.
