# Contract stress tests

Run `cargo test -p will --test stress -- --nocapture`.

| Scenario | Limit exercised |
| --- | --- |
| Batch creation | 10 wills per call |
| Owner and beneficiary indexes | 1,000 wills per address. An additional create returns `TooManyWills` and leaves token balances unchanged. |
| Index queries | 50 rows per page. The test walks the full index and checks both indexes against the IDs returned by creation. |
| Batch check-in | 50 IDs accepted. A 51-ID request fails without checking in the extra will. |
| Competing lifecycle calls | Four parallel native hosts run 64 wills each. Each host executes transactions serially. Emergency check-in wins in two hosts and release wins in two hosts. Retries must preserve status, token balances, and the triggered index. |

The large dataset test lifts the native host's execution budget to reach
the production storage cap. That cap is a logical limit. The stress suite
does not measure transaction throughput, contention between network
submissions, or whether a maximum-size batch fits current network resource
limits.
