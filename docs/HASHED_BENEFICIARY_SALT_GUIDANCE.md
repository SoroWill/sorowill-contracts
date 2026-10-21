# Choosing a salt for hashed beneficiaries (#589)

`add_hashed_beneficiary` stores a 32-byte SHA-256 commitment to the pre-image
`address_bytes || salt_bytes` (`PREIMAGE_LENGTH` = 64 bytes: a 32-byte address
encoding followed by a 32-byte salt). The contract can only check the
commitment length. It cannot see the salt, so it cannot check the salt's
entropy. Choosing a good salt is the owner's responsibility.

## Why it matters

The beneficiary address is often guessable (it may be a known family member's
account). If the salt is short or predictable (a date, a name, a counter), an
observer who reads the commitment from contract storage can brute-force
candidate salts offline and learn who the hidden beneficiary is, or recover
the full pre-image and front-run the claim.

## Recommendation

- Use **32 bytes (256 bits) of output from a cryptographically secure random
  number generator**. This is the full salt width the contract expects.
- Generate a fresh salt per commitment. Never reuse a salt across wills or
  beneficiaries.
- Never derive the salt from a password, a passphrase, a timestamp, the will id
  or any other low-entropy or public value.
- Store the pre-image (address and salt) offline and give it to the beneficiary
  through a separate channel. Anyone holding it can attempt to claim, although
  `reveal_and_claim` binds the pre-image to the claimant address (#369).

## Example

```sh
# 32 random bytes as 64 hex characters
openssl rand -hex 32
```

```js
// Node.js
import { randomBytes, createHash } from "node:crypto";

const salt = randomBytes(32);                       // 256 bits of CSPRNG output
// addrBytes32: the 32-byte address encoding the contract expects in
// preimage[0..32] (see `reveal_and_claim`).
const preimage = Buffer.concat([addrBytes32, salt]); // 64 bytes
const commitment = createHash("sha256").update(preimage).digest(); // 32 bytes
```

Pass `commitment` to `add_hashed_beneficiary`; keep `preimage` secret until
the claim.

## Further reading

- NIST SP 800-90A/B/C, random number generation recommendations.
- RFC 4086, Randomness Requirements for Security.
- OWASP Cryptographic Storage Cheat Sheet (salt and randomness guidance).
