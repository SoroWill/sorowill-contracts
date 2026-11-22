use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

/// Per-operation usage counters tracked by the will contract.
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationStats {
    /// Number of times this operation was invoked.
    pub call_count: u64,
    /// Cumulative amount (in stroops) processed by this operation.
    pub total_amount: i128,
    /// Number of distinct users that invoked this operation.
    pub user_count: u64,
}

/// Aggregate analytics for the will contract.
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageAnalytics {
    pub total_calls: u64,
    pub total_amount: i128,
    pub unique_users: u64,
}

const ANALYTICS_KEY: Symbol = symbol_short!("analytics");
const USER_SEEN_PREFIX: Symbol = symbol_short!("seen");

fn op_key(op: &Symbol) -> (Symbol, Symbol) {
    (symbol_short!("opstat"), op.clone())
}

fn user_seen_key(op: &Symbol, user: &Address) -> (Symbol, Symbol, Address) {
    (USER_SEEN_PREFIX, op.clone(), user.clone())
}

/// Record a single usage of `op` by `user` for `amount`.
///
/// Updates per-operation call counts, cumulative amounts, and unique-user
/// counts, then emits an analytics event describing the usage.
pub fn track_usage(env: &Env, op: Symbol, user: Address, amount: i128) {
    let mut stats: OperationStats = env
        .storage()
        .instance()
        .get(&op_key(&op))
        .unwrap_or_default();

    stats.call_count = stats.call_count.saturating_add(1);
    stats.total_amount = stats.total_amount.saturating_add(amount);

    let seen_key = user_seen_key(&op, &user);
    let already_seen: bool = env.storage().instance().get(&seen_key).unwrap_or(false);
    if !already_seen {
        env.storage().instance().set(&seen_key, &true);
        stats.user_count = stats.user_count.saturating_add(1);
    }

    env.storage().instance().set(&op_key(&op), &stats);

    let mut aggregate: UsageAnalytics = env
        .storage()
        .instance()
        .get(&ANALYTICS_KEY)
        .unwrap_or_default();
    aggregate.total_calls = aggregate.total_calls.saturating_add(1);
    aggregate.total_amount = aggregate.total_amount.saturating_add(amount);
    if !already_seen {
        aggregate.unique_users = aggregate.unique_users.saturating_add(1);
    }
    env.storage().instance().set(&ANALYTICS_KEY, &aggregate);

    env.events().publish(
        (symbol_short!("analytics"), op),
        (user, amount, stats.call_count, stats.user_count),
    );
}

/// Read the per-operation usage stats.
pub fn get_operation_stats(env: &Env, op: Symbol) -> OperationStats {
    env.storage()
        .instance()
        .get(&op_key(&op))
        .unwrap_or_default()
}

/// Read the aggregate usage analytics.
pub fn get_usage_analytics(env: &Env) -> UsageAnalytics {
    env.storage()
        .instance()
        .get(&ANALYTICS_KEY)
        .unwrap_or_default()
}
