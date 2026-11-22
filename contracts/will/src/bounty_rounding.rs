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
