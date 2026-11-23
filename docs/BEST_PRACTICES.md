# Contract Best Practices

## Security Best Practices

### Authentication and Authorization
- Always call `require_auth()` on the caller address before state-modifying operations
- Validate ownership before allowing will modifications
- Use `load_owned()` helper to enforce owner checks consistently

### Input Validation
- Validate all addresses using `assert_valid_beneficiary_address()` and `assert_valid_token()`
- Check beneficiary counts against `MAX_BENEFICIARIES` (10)
- Validate token counts against `MAX_TOKENS` (10)
- Ensure percentage allocations sum to exactly 10,000 basis points
- Validate period ranges: 1 to `MAX_PERIOD_DAYS` (3650)

### Balance Safety
- Always use checked arithmetic (`checked_add`, `checked_sub`) for balance operations
- Validate balances against `MAX_BALANCE` to prevent overflow
- Use `assert_deposit_within_max()` before accepting token deposits

### Status Validation
- Always check will status before operations using `assert_status()`
- Respect the state machine: validate status transitions
- Handle terminal states (`Cancelled`, `Settled`) correctly

### Guardian Security
- Enforce guardian cooldown periods after list changes
- Validate guardian vote expiry windows
- Use weighted quorum checks for guardian triggers
- Clear expired votes before tallying

### Distribution Safety
- Verify grace period has fully elapsed before releasing inheritance
- Check that payouts sum to exact balance to prevent dust or overdraft
- Skip keeper bounty when keeper address equals contract address
- Use atomic distribution: all payouts in a single transaction

### Reentrancy Protection
- Hold release lock during entire `release_inheritance` execution
- Never callback to untrusted contracts during critical operations
- Use `safe_transfer_token()` for all token transfers

### Time Boundaries
- Grace deadline boundary belongs to owner (`now > deadline` releases)
- Use `grace_deadline()` helper for consistent deadline calculation
- Account for ledger timestamp precision (5 second intervals)

## Usage Best Practices

### Will Creation
- Set realistic check-in and grace periods for your use case
- Consider using confirmation delays for high-value wills
- Pre-fund the will immediately after creation
- Keep beneficiary lists under 10 for lower gas costs

### Check-ins
- Schedule automated check-ins well before deadline
- Use `batch_check_in()` for multiple wills to save gas
- Consider setting up a delegate for backup check-ins

### Token Management
- Top up multiple tokens using separate calls
- Monitor `MAX_TOKENS` limit when adding new token types
- Use primary token for keeper bounty calculations

### Beneficiary Management
- Use fixed amounts for predictable distributions
- Use percentages for proportional splitting
- Consider hashed beneficiaries for privacy
- Validate allocations sum correctly before updating

### Guardian Configuration
- Choose trusted guardians carefully
- Use weighted guardians for nuanced quorum rules
- Set appropriate vote expiry windows
- Respect cooldown periods after guardian changes

### Migration and Maintenance
- Call `migrate_will()` after contract upgrades
- Archive terminal wills to reclaim storage
- Monitor protocol stats for consistency

### Error Handling
- Catch and handle specific `WillError` codes
- Use `retry_failed_payout()` for recoverable transfer failures
- Check error code reference in README for meanings

### Gas Optimization
- Use pagination for large result sets
- Batch operations when possible
- Archive completed wills to reduce storage costs

## Integration Best Practices

### Event Monitoring
- Subscribe to contract events for state changes
- Treat events as the durable audit log
- Handle event atomicity: failed calls discard events

### Off-chain Storage
- Store will history off-chain via events
- Don't rely on `get_will_history()` after archival
- Index events for quick beneficiary/owner lookups

### Client-side Validation
- Pre-validate inputs before contract calls
- Handle `WillNotFound` for archived wills gracefully
- Respect cursor pagination for consistency

### Version Checking
- Call `get_contract_version()` to detect upgrades
- Handle version mismatches in SDK/app logic
- Document version compatibility in integrations
