# Create Will Example

This example shows how to create a will through the contract integration layer.

## Prerequisites

- A configured contract client (see your integration setup)
- A connected wallet with permission to create wills
- The beneficiary address and will parameters

## Example

```ts
import { ContractClient } from "../contract-client";

async function createWill() {
  const client = new ContractClient();

  const will = {
    beneficiary: "0x1234...abcd",
    amount: "1000000000000000000", // 1 token (in base units)
    checkInInterval: 60 * 60 * 24 * 30, // 30 days in seconds
    gracePeriod: 60 * 60 * 24 * 7, // 7 days in seconds
  };

  const tx = await client.createWill(will);
  const receipt = await tx.wait();

  console.log("Will created:", receipt.transactionHash);
  return receipt;
}
```

## Notes

- `amount` must be provided in the token's base units.
- `checkInInterval` and `gracePeriod` are expressed in seconds.
- Always await the transaction receipt before treating the will as created.

## Related Examples

- [Check-in example](./check-in.md)
- [Claim example](./claim.md)
- [Error handling example](./error-handling.md)
