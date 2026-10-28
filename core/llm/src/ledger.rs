use memory::{NewSpendCapEvent, Store};

/// Unset resolves to a conservative built-in default (below); an explicit `0` is the
/// only opt-out — "unset ≠ uncapped" carries over from llm-governance-gateway exactly.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpendCapConfig {
    pub user_daily_cents: Option<i64>,
    pub global_daily_cents: Option<i64>,
}

/// The caps `check_spend_cap` applies when the setting is unset (`None`); shells show
/// them so "X¢ / cap¢ today" never hardcodes a number.
pub const DEFAULT_USER_DAILY_CAP_CENTS: i64 = 200;
pub const DEFAULT_GLOBAL_DAILY_CAP_CENTS: i64 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendCapScope {
    User,
    /// The app-wide circuit breaker tripped — the caller did nothing wrong; this isn't
    /// "you're over your limit," it's "the app is busy," matching the gateway's own
    /// distinction (`SpendCapError.scope`).
    Global,
}

#[derive(Debug, Clone, Copy)]
pub struct SpendCapError {
    pub spent_cents: i64,
    pub cap_cents: i64,
    pub scope: SpendCapScope,
}

impl std::fmt::Display for SpendCapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.scope {
            SpendCapScope::User => write!(
                f,
                "daily spend cap exceeded: {} >= {} cents",
                self.spent_cents, self.cap_cents
            ),
            SpendCapScope::Global => write!(
                f,
                "app-wide daily spend circuit breaker tripped: {} >= {} cents",
                self.spent_cents, self.cap_cents
            ),
        }
    }
}

impl std::error::Error for SpendCapError {}

/// Global circuit breaker first (a per-identity cap can't see N identities' combined
/// spend on a bad day), then the per-identity/device cap. `now_unix_ms` is the caller's
/// job to compute (matches this crate's existing "caller computes the time range"
/// convention, e.g. `generate_daily_digest`'s `start_unix_ms`/`end_unix_ms`) — the
/// day-boundary used here is UTC midnight, matching llm-governance-gateway's own
/// `todayUtc.setUTCHours(0,0,0,0)`.
pub fn check_spend_cap(
    store: &Store,
    config: &SpendCapConfig,
    user_id: Option<&str>,
    route: Option<&str>,
    now_unix_ms: i64,
) -> Result<(), SpendCapError> {
    const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;
    let today_start_unix_ms = (now_unix_ms / MS_PER_DAY) * MS_PER_DAY;

    let global_cap = config
        .global_daily_cents
        .unwrap_or(DEFAULT_GLOBAL_DAILY_CAP_CENTS);
    if global_cap != 0 {
        let total = store
            .sum_spend_cents(today_start_unix_ms, None)
            .unwrap_or(0);
        if total >= global_cap {
            let _ = store.record_spend_cap_event(&NewSpendCapEvent {
                user_id: user_id.map(str::to_string),
                cap_cents: global_cap,
                spent_cents: total,
                route: route.map(str::to_string),
                would_block: true,
                ts_unix_ms: now_unix_ms,
            });
            return Err(SpendCapError {
                spent_cents: total,
                cap_cents: global_cap,
                scope: SpendCapScope::Global,
            });
        }
    }

    let user_cap = config
        .user_daily_cents
        .unwrap_or(DEFAULT_USER_DAILY_CAP_CENTS);
    if user_cap == 0 {
        return Ok(()); // explicit 0 = deliberate opt-out
    }
    let spent = match user_id {
        Some(id) => store.sum_spend_cents(today_start_unix_ms, Some(id)),
        None => store.sum_spend_cents_anonymous(today_start_unix_ms),
    }
    .unwrap_or(0);
    if spent >= user_cap {
        let _ = store.record_spend_cap_event(&NewSpendCapEvent {
            user_id: user_id.map(str::to_string),
            cap_cents: user_cap,
            spent_cents: spent,
            route: route.map(str::to_string),
            would_block: true,
            ts_unix_ms: now_unix_ms,
        });
        return Err(SpendCapError {
            spent_cents: spent,
            cap_cents: user_cap,
            scope: SpendCapScope::User,
        });
    }
    Ok(())
}

/// Why [`charge_skin_call`] refused a call.
#[derive(Debug)]
pub enum ChargeError {
    SpendCap(SpendCapError),
    /// The usage row could not be written. The call is refused (fail closed): an
    /// unrecorded charge would let spend run past the cap unseen.
    Ledger(String),
}

impl From<SpendCapError> for ChargeError {
    fn from(err: SpendCapError) -> Self {
        ChargeError::SpendCap(err)
    }
}

/// Skins Release 2 (ruling R2): the spend handling for a flat-priced skin call (image
/// search, image generation). Checks the cap on `route`, then records `cents` against it
/// before the caller touches the network, so a failed call still costs its price — the
/// provider bills the attempt either way. The row is anonymous (`user_id: None`), which
/// is what the next check's per-user sum counts; `provider` is the fixed label `"image"`
/// and `model` repeats the route, since these calls have no token usage to price.
/// Returns `ChargeError` (not R2's bare `SpendCapError`) so a failed write fails closed.
pub fn charge_skin_call(
    store: &Store,
    config: &SpendCapConfig,
    route: &str,
    cents: i64,
    now_unix_ms: i64,
) -> Result<(), ChargeError> {
    check_spend_cap(store, config, None, Some(route), now_unix_ms)?;
    store
        .log_usage(&memory::NewUsageLogEntry {
            user_id: None,
            provider: "image".to_string(),
            model: route.to_string(),
            route: Some(route.to_string()),
            input_tokens: None,
            output_tokens: None,
            estimated_cost_cents: cents,
            cache_hit: false,
            trace_id: format!("{route}-{now_unix_ms}"),
            ts_unix_ms: now_unix_ms,
        })
        .map_err(|e| ChargeError::Ledger(e.to_string()))?;
    Ok(())
}
