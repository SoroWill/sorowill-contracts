# Check-in Example

This example shows how to perform a periodic check-in to keep a will active.

## Example

```ts
import { ContractClient } from "../contract-client";

async function checkIn(willId: string) {
  const client = new ContractClient();

  const tx = await client.checkIn(willId);
  const receipt = await tx.wait();

  console.log("Checked in for will", willId, "at", receipt.transactionHash);
  return receipt;
}
```

## Notes

- Check in before the `checkInInterval` elapses to avoid triggering the grace period.
- Each successful check-in resets the timer for the will.
- Handle failures (e.g. expired will) as shown in the [error handling example](./error-handling.md).

## Related Examples

- [Create will example](./create-will.md)
- [Claim example](./claim.md)
