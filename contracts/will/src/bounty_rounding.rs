use soroban_sdk::{contracterror, Env};

/// Errors returned by bounty rounding helpers.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BountyRoundingError {
    /// The divisor supplied to a division operation was zero.
    DivisionByZero = 1,
}

/// Divide `amount` by `divisor`, returning a descriptive error instead of
/// panicking when `divisor` is zero.
pub fn safe_divide(
    _env: &Env,
    amount: i128,
    divisor: i128,
) -> Result<i128, BountyRoundingError> {
    if divisor == 0 {
        return Err(BountyRoundingError::DivisionByZero);
    }
    Ok(amount / divisor)
}

/// Compute the per-recipient share of `total` across `recipients`.
///
/// Returns `BountyRoundingError::DivisionByZero` when `recipients` is zero
/// rather than panicking.
pub fn per_recipient_share(
    env: &Env,
    total: i128,
    recipients: i128,
) -> Result<i128, BountyRoundingError> {
    safe_divide(env, total, recipients)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn safe_divide_returns_error_on_zero_divisor() {
        let env = Env::default();
        let result = safe_divide(&env, 100, 0);
        assert_eq!(result, Err(BountyRoundingError::DivisionByZero));
    }

    #[test]
    fn safe_divide_works_for_non_zero_divisor() {
        let env = Env::default();
        assert_eq!(safe_divide(&env, 100, 4), Ok(25));
    }

    #[test]
    fn per_recipient_share_returns_error_on_zero_recipients() {
        let env = Env::default();
        let result = per_recipient_share(&env, 100, 0);
        assert_eq!(result, Err(BountyRoundingError::DivisionByZero));
    }

    #[test]
    fn per_recipient_share_works_for_non_zero_recipients() {
        let env = Env::default();
        assert_eq!(per_recipient_share(&env, 100, 5), Ok(20));
    }
}

/// Collects the dust left over after distributing `shares` out of `total`.
///
/// Each share is expected to have been rounded down, so the sum of `shares`
/// is at most `total`. The returned dust is `total - sum(shares)` and is
/// always non-negative. A negative result (shares over-allocated) is clamped
/// to zero so callers never distribute more than `total`.
pub fn collect_dust(total: i128, shares: &[i128]) -> i128 {
    let distributed: i128 = shares.iter().copied().sum();
    let dust = total - distributed;
    if dust > 0 {
        dust
    } else {
        0
    }
}

/// Distributes `total` across `shares`, adding the collected dust to the last
/// share so no sub-unit amount is stranded in the contract.
///
/// Returns the adjusted shares. When `shares` is empty the whole `total` is
/// returned as a single dust entry so the caller can refund it to the owner.
pub fn distribute_with_dust(total: i128, shares: &[i128]) -> Vec<i128> {
    let mut adjusted = shares.to_vec();
    let dust = collect_dust(total, shares);
    if adjusted.is_empty() {
        if dust > 0 {
            adjusted.push(dust);
        }
        return adjusted;
    }
    if dust > 0 {
        let last = adjusted.len() - 1;
        adjusted[last] += dust;
    }
    // Dust prevention invariant: the distributed shares must account for the
    // full distributable amount, leaving nothing stranded.
    let distributed: i128 = adjusted.iter().copied().sum();
    debug_assert_eq!(distributed, total, "dust prevention: shares must sum to total");
    adjusted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeper_bounty_rounds_down() {
        assert_eq!(keeper_bounty(10_000, 100), 100);
        assert_eq!(keeper_bounty(9_999, 100), 99);
        assert_eq!(keeper_bounty(99, 100), 0);
    }

    #[test]
    fn collect_dust_returns_remainder() {
        // 100 split three ways rounds down to 33 each, leaving 1 dust.
        assert_eq!(collect_dust(100, &[33, 33, 33]), 1);
        // Exact split leaves no dust.
        assert_eq!(collect_dust(100, &[50, 50]), 0);
        // Over-allocation is clamped to zero.
        assert_eq!(collect_dust(10, &[6, 6]), 0);
    }

    #[test]
    fn distribute_with_dust_leaves_nothing_stranded() {
        let adjusted = distribute_with_dust(100, &[33, 33, 33]);
        assert_eq!(adjusted, vec![33, 33, 34]);
        assert_eq!(adjusted.iter().sum::<i128>(), 100);
    }

    #[test]
    fn distribute_with_dust_handles_empty_shares() {
        let adjusted = distribute_with_dust(7, &[]);
        assert_eq!(adjusted, vec![7]);
        assert_eq!(adjusted.iter().sum::<i128>(), 7);
    }

    #[test]
    fn distribute_with_dust_no_dust_when_exact() {
        let adjusted = distribute_with_dust(100, &[25, 25, 25, 25]);
        assert_eq!(adjusted, vec![25, 25, 25, 25]);
        assert_eq!(adjusted.iter().sum::<i128>(), 100);
    }
}
