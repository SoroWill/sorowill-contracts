# Claim Example

This example shows how a beneficiary claims a will after the check-in interval and grace period have elapsed.

## Example

```ts
import { ContractClient } from "../contract-client";

async function claim(willId: string) {
  const client = new ContractClient();

  const tx = await client.claim(willId);
  const receipt = await tx.wait();

  console.log("Will claimed:", willId, "tx:", receipt.transactionHash);
  return receipt;
}
```

## Notes

- A claim is only valid after the check-in interval and grace period have passed.
- Only the designated beneficiary can claim the will.
- See the [error handling example](./error-handling.md) for handling premature or unauthorized claims.

## Related Examples

- [Create will example](./create-will.md)
- [Check-in example](./check-in.md)
