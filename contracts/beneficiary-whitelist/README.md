# Beneficiary whitelist

Deploy the contract with the administrator's address as the constructor
argument. The administrator authorizes deployment and every call to `add`
or `remove`. Anyone can call `contains` to check an address. An address starts
off the list. A successful change returns `true` and emits a
`membership_changed` event with the beneficiary as an indexed topic and the
new `allowed` value as its data. Repeating a change returns `false` and emits
no event.

Each member occupies one persistent ledger entry. Historical entries archived
by Stellar may require restoration before invocation. The will contract can
be deployed and used independently of the whitelist.
