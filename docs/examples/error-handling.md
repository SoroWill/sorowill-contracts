# Error Handling Example

This example shows how to handle common errors when integrating with the will contract.

## Example

```ts
import { ContractClient } from "../contract-client";

async function safeCreateWill(client: ContractClient, will: unknown) {
  try {
    const tx = await client.createWill(will);
    const receipt = await tx.wait();
    return { ok: true as const, receipt };
  } catch (error) {
    if (isUserRejected(error)) {
      console.warn("Transaction rejected by user");
      return { ok: false as const, reason: "rejected" };
    }

    if (isRevert(error)) {
      console.error("Contract reverted:", getRevertReason(error));
      return { ok: false as const, reason: "reverted" };
    }

    console.error("Unexpected error:", error);
    return { ok: false as const, reason: "unknown" };
  }
}

function isUserRejected(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    (error as { code?: number }).code === 4001
  );
}

function isRevert(error: unknown): boolean {
  return error instanceof Error && /revert/i.test(error.message);
}

function getRevertReason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
```

## Notes

- Distinguish user rejections from contract reverts and unexpected failures.
- Surface revert reasons to help users understand why a call failed.
- Always await the transaction receipt to catch on-chain failures.

## Related Examples

- [Create will example](./create-will.md)
- [Check-in example](./check-in.md)
- [Claim example](./claim.md)
