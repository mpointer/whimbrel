//! Monthly AI budgets (docs/superpowers/specs/2026-11-05-provider-budgets-design.md): each
//! device's spend per paid provider per UTC month (`provider_spend`, synced), the budgets
//! (`provider_budget`), and which 80%/100% notices this device already gave (`budget_notice`).
//! Each row is keyed by device and install (the database's `sync_entity_epoch`), so a device
//! reinstalled under the same Keychain id adds a new row instead of replacing its old one.
//! A provider's month spend = max(this database's `usage_log` sum, its own `provider_spend` row)
//! + every other row (peers, and this device's earlier installs). Without a device id the own
//! row is kept under `""`; it stays own once an id is set.

use std::collections::BTreeMap;

use chrono::{DateTime, Months, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};

use crate::{Store, OUTBOX_ROW_TYPE_PROVIDER_SPEND};

/// `llm::Provider::as_str` of every provider that bills. `ollama` and `local` are free.
const PAID_PROVIDERS: [&str; 4] = ["anthropic", "openai", "google", "openrouter"];
const THRESHOLDS: [u8; 2] = [80, 100];
/// $1,000,000: the most a budget can be (the Settings field's limit too).
pub const MAX_BUDGET_CENTS: i64 = 100_000_000;
/// A synced row's cents are capped here so summing every row can't overflow SQLite's SUM.
const MAX_ROW_CENTS: i64 = 1_000_000_000_000;

/// A `provider_spend` row on the wire; the sending device is the row's device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedProviderSpend {
    /// The sending database's `sync_entity_epoch`.
    #[serde(default)]
    pub install: String,
    pub month: String,
    pub provider: String,
    pub cents: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetNotice {
    pub provider: String,
    /// 80 or 100.
    pub threshold: u8,
    pub spent_cents: i64,
    pub budget_cents: i64,
    pub month: String,
}

pub fn is_paid_provider(provider: &str) -> bool {
    PAID_PROVIDERS.contains(&provider)
}

/// The UTC calendar month of `unix_ms`, as "YYYY-MM".
pub fn month_label(unix_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(unix_ms)
        .unwrap_or_default()
        .format("%Y-%m")
        .to_string()
}

/// [start, end) in unix ms of a "YYYY-MM" month.
fn month_bounds(month: &str) -> Option<(i64, i64)> {
    let start = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").ok()?;
    let end = start.checked_add_months(Months::new(1))?;
    let ms = |d: NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .map(|t| t.and_utc().timestamp_millis())
    };
    Some((ms(start)?, ms(end)?))
}

pub(crate) fn ensure_provider_budget_tables(conn: &Connection) -> rusqlite::Result<()> {
    // A pre-release build keyed rows without `install`; that table is rebuilt from sync.
    let keyed_by_install: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info('provider_spend') WHERE name = 'install'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if keyed_by_install.is_none() {
        conn.execute("DROP TABLE IF EXISTS provider_spend", [])?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_spend (
            device_id TEXT NOT NULL,
            install TEXT NOT NULL,
            month TEXT NOT NULL,
            provider TEXT NOT NULL,
            cents INTEGER NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY(device_id, install, month, provider)
        );
        CREATE TABLE IF NOT EXISTS provider_budget (
            provider TEXT PRIMARY KEY,
            monthly_cents INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS budget_notice (
            month TEXT,
            provider TEXT,
            threshold INTEGER,
            budget_cents INTEGER,
            PRIMARY KEY(month, provider, threshold, budget_cents)
        );",
    )
}

/// (provider, cents) rows of a two-column query.
fn pairs(
    conn: &Connection,
    sql: &str,
    args: &[&dyn ToSql],
) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(args, |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

impl Store {
    /// Rewrites this device's row for the month of a paid call just logged in `tx`: its own
    /// `usage_log` sum, never below the row as stored, stamped past the row's last stamp.
    pub(crate) fn record_provider_spend_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        provider: &str,
        ts_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        if !is_paid_provider(provider) {
            return Ok(());
        }
        let month = month_label(ts_unix_ms);
        let Some((start, end)) = month_bounds(&month) else {
            return Ok(());
        };
        let device = self.own_device_id().unwrap_or_default();
        let install = self.sync_entity_epoch()?;
        let used: i64 = tx.query_row(
            "SELECT COALESCE(SUM(estimated_cost_cents), 0) FROM usage_log \
             WHERE provider = ?1 AND ts_unix_ms >= ?2 AND ts_unix_ms < ?3 AND estimated_cost_cents > 0",
            params![provider, start, end],
            |r| r.get(0),
        )?;
        let (cents, updated_at_unix_ms) = tx.query_row(
            "INSERT INTO provider_spend (device_id, install, month, provider, cents, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(device_id, install, month, provider) DO UPDATE SET cents = MAX(excluded.cents, provider_spend.cents), \
             updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, provider_spend.updated_at_unix_ms + 1) \
             RETURNING cents, updated_at_unix_ms",
            params![device, install, month, provider, used, crate::now_unix_ms()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let id = format!("{device}:{month}:{provider}");
        let row = AppliedProviderSpend {
            install,
            month,
            provider: provider.to_string(),
            cents,
            updated_at_unix_ms,
        };
        self.enqueue_upsert_tx(tx, OUTBOX_ROW_TYPE_PROVIDER_SPEND, id, &row)
    }

    /// Stores a synced row. A device's month only grows, so the stored cents and stamp only
    /// rise (a stale or reinstalled sender can't lower them). A row from this device's own
    /// id is its own row coming back: from this install it merges into the own row, from an
    /// earlier install it counts on top. Never stored under `""`, which means "this device".
    pub fn apply_synced_provider_spend(
        &self,
        device_id: &str,
        row: &AppliedProviderSpend,
    ) -> rusqlite::Result<()> {
        if device_id.is_empty()
            || row.cents < 0
            || !is_paid_provider(&row.provider)
            || month_bounds(&row.month).is_none()
        {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO provider_spend (device_id, install, month, provider, cents, updated_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(device_id, install, month, provider) DO UPDATE SET cents = MAX(excluded.cents, provider_spend.cents), \
             updated_at_unix_ms = MAX(excluded.updated_at_unix_ms, provider_spend.updated_at_unix_ms)",
            params![device_id, row.install, row.month, row.provider, row.cents.min(MAX_ROW_CENTS), row.updated_at_unix_ms],
        )?;
        Ok(())
    }

    /// Whether this database already read its device's own rows back from the relay.
    pub fn own_provider_spend_restored(&self) -> rusqlite::Result<bool> {
        self.conn
            .query_row(
                "SELECT 1 FROM kv_meta WHERE key = 'provider_spend_restored'",
                [],
                |_| Ok(()),
            )
            .optional()
            .map(|r| r.is_some())
    }

    pub fn mark_own_provider_spend_restored(&self) -> rusqlite::Result<()> {
        self.conn
            .execute("INSERT OR REPLACE INTO kv_meta (key, value) VALUES ('provider_spend_restored', '1')", [])
            .map(|_| ())
    }

    /// Each paid provider's spend in `month` ("YYYY-MM") across this device and its peers,
    /// by provider; providers with no spend are left out.
    pub fn month_spend_by_provider(&self, month: &str) -> rusqlite::Result<Vec<(String, i64)>> {
        let Some((start, end)) = month_bounds(month) else {
            return Ok(Vec::new());
        };
        // Only this database writes rows under its own install, whatever the device id. Read,
        // never created here: with no epoch yet, this database has written no row.
        let install: String = self
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'sync_entity_epoch'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default();
        let mut totals: BTreeMap<String, i64> = BTreeMap::new();
        let used = pairs(
            &self.conn,
            "SELECT provider, SUM(estimated_cost_cents) FROM usage_log \
             WHERE ts_unix_ms >= ?1 AND ts_unix_ms < ?2 AND estimated_cost_cents > 0 GROUP BY provider",
            &[&start, &end],
        )?;
        totals.extend(used);
        let own_rows = pairs(
            &self.conn,
            "SELECT provider, MAX(cents) FROM provider_spend WHERE month = ?1 AND install = ?2 GROUP BY provider",
            &[&month, &install],
        )?;
        for (p, c) in own_rows {
            let t = totals.entry(p).or_default();
            *t = (*t).max(c);
        }
        let other_rows = pairs(
            &self.conn,
            "SELECT provider, SUM(cents) FROM provider_spend WHERE month = ?1 AND install != ?2 GROUP BY provider",
            &[&month, &install],
        )?;
        for (p, c) in other_rows {
            let t = totals.entry(p).or_default();
            *t = t.saturating_add(c);
        }
        Ok(totals
            .into_iter()
            .filter(|(p, c)| is_paid_provider(p) && *c > 0)
            .collect())
    }

    /// Replaces every budget. A free provider or an amount of 0 or less sets none; an amount
    /// over `MAX_BUDGET_CENTS` is set to it.
    pub fn set_provider_budgets(&self, budgets: &[(String, i64)]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM provider_budget", [])?;
        for (provider, cents) in budgets
            .iter()
            .filter(|(p, c)| is_paid_provider(p) && *c > 0)
        {
            tx.execute(
                "INSERT OR REPLACE INTO provider_budget (provider, monthly_cents) VALUES (?1, ?2)",
                params![provider, (*cents).min(MAX_BUDGET_CENTS)],
            )?;
        }
        tx.commit()
    }

    /// (provider, monthly cents), by provider.
    pub fn provider_budgets(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        pairs(
            &self.conn,
            "SELECT provider, monthly_cents FROM provider_budget ORDER BY provider",
            &[],
        )
    }

    /// Every threshold reached this month that this device hasn't noticed yet for this budget
    /// amount, recorded as noticed. A new amount re-arms both thresholds.
    pub fn take_budget_notices(&self, now_unix_ms: i64) -> rusqlite::Result<Vec<BudgetNotice>> {
        let month = month_label(now_unix_ms);
        let spend: BTreeMap<String, i64> =
            self.month_spend_by_provider(&month)?.into_iter().collect();
        let tx = self.conn.unchecked_transaction()?;
        let mut notices = Vec::new();
        for (provider, budget) in self.provider_budgets()? {
            let spent = spend.get(&provider).copied().unwrap_or(0);
            for threshold in THRESHOLDS
                .into_iter()
                .filter(|t| spent.saturating_mul(100) >= i64::from(*t).saturating_mul(budget))
            {
                let new = tx.execute(
                    "INSERT OR IGNORE INTO budget_notice (month, provider, threshold, budget_cents) VALUES (?1, ?2, ?3, ?4)",
                    params![month, provider, threshold, budget],
                )?;
                if new == 1 {
                    notices.push(BudgetNotice {
                        provider: provider.clone(),
                        threshold,
                        spent_cents: spent,
                        budget_cents: budget,
                        month: month.clone(),
                    });
                }
            }
        }
        tx.commit()?;
        Ok(notices)
    }

    /// `Some((spent, budget))` when `provider` has a budget and this month's spend has reached it.
    pub fn provider_over_budget(
        &self,
        provider: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Option<(i64, i64)>> {
        if !is_paid_provider(provider) {
            return Ok(None);
        }
        let budget: Option<i64> = self
            .conn
            .query_row(
                "SELECT monthly_cents FROM provider_budget WHERE provider = ?1",
                params![provider],
                |r| r.get(0),
            )
            .optional()?;
        let Some(budget) = budget.filter(|b| *b > 0) else {
            return Ok(None);
        };
        let spent = self
            .month_spend_by_provider(&month_label(now_unix_ms))?
            .into_iter()
            .find_map(|(p, c)| (p == provider).then_some(c))
            .unwrap_or(0);
        Ok((spent >= budget).then_some((spent, budget)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewUsageLogEntry, Store};

    // 2026-11-15T12:00:00Z
    const NOV: i64 = 1_794_744_000_000;

    fn log(s: &Store, provider: &str, cents: i64, ts: i64) {
        s.log_usage(&NewUsageLogEntry {
            user_id: None,
            provider: provider.into(),
            model: "m".into(),
            route: None,
            input_tokens: None,
            output_tokens: None,
            estimated_cost_cents: cents,
            cache_hit: false,
            trace_id: "t".into(),
            ts_unix_ms: ts,
        })
        .unwrap();
    }

    fn spend_outbox(s: &Store) -> Vec<(String, AppliedProviderSpend)> {
        let mut stmt = s
            .conn
            .prepare("SELECT aggregate_id, payload_json FROM sync_outbox WHERE row_type = 'provider_spend' ORDER BY local_seq")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap();
        rows.map(|r| r.unwrap())
            .map(|(id, p)| (id, serde_json::from_str(&p).unwrap()))
            .collect()
    }

    fn own_row(s: &Store, device: &str, month: &str, provider: &str) -> Option<i64> {
        s.conn
            .query_row(
                "SELECT cents FROM provider_spend WHERE device_id = ?1 AND month = ?2 AND provider = ?3",
                rusqlite::params![device, month, provider],
                |r| r.get(0),
            )
            .ok()
    }

    #[test]
    fn month_label_is_utc() {
        assert_eq!(month_label(NOV), "2026-11");
        // 2026-11-30T23:59:59.999Z and 2026-12-01T00:00:00Z.
        assert_eq!(month_label(1_796_083_199_999), "2026-11");
        assert_eq!(month_label(1_796_083_200_000), "2026-12");
        assert_eq!(month_label(0), "1970-01");
    }

    #[test]
    fn every_log_usage_write_updates_provider_spend() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        // `llm::charge_skin_call` calls `log_usage` directly, like this.
        log(&s, "anthropic", 120, NOV);
        log(&s, "anthropic", 30, NOV + 1);
        log(&s, "openai", 7, NOV + 2);
        assert_eq!(own_row(&s, "dev", "2026-11", "anthropic"), Some(150));
        assert_eq!(own_row(&s, "dev", "2026-11", "openai"), Some(7));
        let ops = spend_outbox(&s);
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[1].0, "dev:2026-11:anthropic");
        assert_eq!(
            (
                ops[1].1.month.as_str(),
                ops[1].1.provider.as_str(),
                ops[1].1.cents
            ),
            ("2026-11", "anthropic", 150)
        );
        assert!(ops[1].1.updated_at_unix_ms > ops[0].1.updated_at_unix_ms);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 150), ("openai".into(), 7)]
        );
    }

    #[test]
    fn without_device_id_row_is_kept_and_nothing_enqueued() {
        let s = Store::open_in_memory().unwrap();
        log(&s, "google", 40, NOV);
        assert!(spend_outbox(&s).is_empty());
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("google".into(), 40)]
        );
        // Getting a device id later doesn't count that row as a peer's.
        s.set_own_device_id("dev");
        log(&s, "google", 10, NOV + 1);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("google".into(), 50)]
        );
    }

    #[test]
    fn free_providers_never_recorded() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        log(&s, "ollama", 5, NOV);
        log(&s, "local", 5, NOV);
        log(&s, "anthropic", 0, NOV);
        let rows: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM provider_spend", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        assert!(spend_outbox(&s).is_empty());
        assert!(s.month_spend_by_provider("2026-11").unwrap().is_empty());
        s.set_provider_budgets(&[("ollama".into(), 1)]).unwrap();
        assert_eq!(s.provider_over_budget("ollama", NOV).unwrap(), None);
    }

    #[test]
    fn month_spend_resets_at_utc_month() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        let nov_end = 1_796_083_140_000; // 2026-11-30T23:59Z
        let dec_start = 1_796_083_260_000; // 2026-12-01T00:01Z
        log(&s, "openai", 500, nov_end);
        log(&s, "openai", 20, dec_start);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("openai".into(), 500)]
        );
        assert_eq!(
            s.month_spend_by_provider("2026-12").unwrap(),
            vec![("openai".into(), 20)]
        );
        s.set_provider_budgets(&[("openai".into(), 500)]).unwrap();
        assert_eq!(
            s.provider_over_budget("openai", nov_end).unwrap(),
            Some((500, 500))
        );
        assert_eq!(s.provider_over_budget("openai", dec_start).unwrap(), None);
    }

    #[test]
    fn own_total_never_below_stored_row() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        // As last received back through sync, e.g. before a reinstall.
        let install = s.sync_entity_epoch().unwrap();
        s.conn
            .execute(
                "INSERT INTO provider_spend VALUES ('dev', ?1, '2026-11', 'anthropic', 900, 1)",
                [&install],
            )
            .unwrap();
        log(&s, "anthropic", 300, NOV);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 900)]
        );
        assert_eq!(own_row(&s, "dev", "2026-11", "anthropic"), Some(900));
        assert_eq!(spend_outbox(&s)[0].1.cents, 900);
        // A peer's row adds on top.
        s.conn
            .execute(
                "INSERT INTO provider_spend VALUES ('peer', 'p1', '2026-11', 'anthropic', 100, 1)",
                [],
            )
            .unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 1000)]
        );
    }

    fn row(cents: i64, at: i64) -> AppliedProviderSpend {
        AppliedProviderSpend {
            install: "p1".into(),
            month: "2026-11".into(),
            provider: "anthropic".into(),
            cents,
            updated_at_unix_ms: at,
        }
    }

    #[test]
    fn stale_peer_row_does_not_lower_total() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.apply_synced_provider_spend("peer", &row(400, 20))
            .unwrap();
        s.apply_synced_provider_spend("peer", &row(100, 10))
            .unwrap(); // older
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 400)]
        );
        s.apply_synced_provider_spend("peer", &row(450, 30))
            .unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 450)]
        );
        // Never a peer under "" (that is this device), nor junk rows.
        s.apply_synced_provider_spend("", &row(5, 99)).unwrap();
        s.apply_synced_provider_spend(
            "peer",
            &AppliedProviderSpend {
                provider: "ollama".into(),
                ..row(5, 99)
            },
        )
        .unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 450)]
        );
    }

    #[test]
    fn newer_peer_row_with_lower_cents_keeps_max() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.apply_synced_provider_spend("peer", &row(2000, 10))
            .unwrap();
        // Newer but lower (e.g. a resent row from a lagging copy): a month only grows.
        s.apply_synced_provider_spend("peer", &row(50, 20)).unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 2000)]
        );
        // The stamp still moves forward.
        let at: i64 = s
            .conn
            .query_row(
                "SELECT updated_at_unix_ms FROM provider_spend WHERE device_id = 'peer'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(at, 20);
        // A reinstalled peer's new install counts on top of its old one.
        s.apply_synced_provider_spend(
            "peer",
            &AppliedProviderSpend {
                install: "p2".into(),
                ..row(50, 30)
            },
        )
        .unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 2050)]
        );
    }

    #[test]
    fn own_row_returning_restores_total() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        let this_install = || AppliedProviderSpend {
            install: s.sync_entity_epoch().unwrap(),
            ..row(0, 0)
        };
        // From an earlier install of this device (a reinstall): counts on top of new spend.
        s.apply_synced_provider_spend("dev", &row(900, 50)).unwrap();
        log(&s, "anthropic", 30, NOV);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 930)]
        );
        s.apply_synced_provider_spend("dev", &row(200, 10)).unwrap(); // stale copy
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 930)]
        );
        // This install's own row coming back merges into it, never twice.
        s.apply_synced_provider_spend(
            "dev",
            &AppliedProviderSpend {
                cents: 30,
                updated_at_unix_ms: 99,
                ..this_install()
            },
        )
        .unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 930)]
        );
    }

    #[test]
    fn own_row_from_this_install_is_never_below_stored() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        let row = |cents, at| AppliedProviderSpend {
            install: s.sync_entity_epoch().unwrap(),
            ..row(cents, at)
        };
        s.apply_synced_provider_spend("dev", &row(900, 50)).unwrap();
        assert_eq!(own_row(&s, "dev", "2026-11", "anthropic"), Some(900));
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 900)]
        );
        // A stale copy cannot lower it.
        s.apply_synced_provider_spend("dev", &row(200, 10)).unwrap();
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 900)]
        );
        // New spend continues from the restored row.
        log(&s, "anthropic", 300, NOV);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 900)]
        );
        log(&s, "anthropic", 700, NOV + 1);
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 1000)]
        );
    }

    #[test]
    fn budgets_replace_and_zero_removes() {
        let s = Store::open_in_memory().unwrap();
        s.set_provider_budgets(&[("anthropic".into(), 2500), ("openai".into(), 1000)])
            .unwrap();
        assert_eq!(
            s.provider_budgets().unwrap(),
            vec![("anthropic".into(), 2500), ("openai".into(), 1000)]
        );
        s.set_provider_budgets(&[("anthropic".into(), 0), ("google".into(), 300)])
            .unwrap();
        assert_eq!(s.provider_budgets().unwrap(), vec![("google".into(), 300)]);
        s.set_provider_budgets(&[]).unwrap();
        assert!(s.provider_budgets().unwrap().is_empty());
    }

    #[test]
    fn notices_once_per_threshold_and_rearm_on_new_budget() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.set_provider_budgets(&[("anthropic".into(), 2500)])
            .unwrap();
        log(&s, "anthropic", 1999, NOV);
        assert!(s.take_budget_notices(NOV).unwrap().is_empty());
        log(&s, "anthropic", 1, NOV);
        let n = s.take_budget_notices(NOV).unwrap();
        assert_eq!(
            n,
            vec![BudgetNotice {
                provider: "anthropic".into(),
                threshold: 80,
                spent_cents: 2000,
                budget_cents: 2500,
                month: "2026-11".into()
            }]
        );
        assert!(s.take_budget_notices(NOV).unwrap().is_empty());
        log(&s, "anthropic", 500, NOV);
        let n = s.take_budget_notices(NOV).unwrap();
        assert_eq!(n.iter().map(|n| n.threshold).collect::<Vec<_>>(), vec![100]);
        assert!(s.take_budget_notices(NOV).unwrap().is_empty());
        // Raising the budget re-arms both thresholds against the new amount.
        s.set_provider_budgets(&[("anthropic".into(), 3000)])
            .unwrap();
        let n = s.take_budget_notices(NOV).unwrap();
        assert_eq!(
            n.iter()
                .map(|n| (n.threshold, n.budget_cents))
                .collect::<Vec<_>>(),
            vec![(80, 3000)]
        );
        // A new month starts over.
        let dec = NOV + 31 * 24 * 3600 * 1000;
        assert!(s.take_budget_notices(dec).unwrap().is_empty());
    }

    #[test]
    fn lowering_budget_below_spend_notices_at_once() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.set_provider_budgets(&[("openai".into(), 10_000)])
            .unwrap();
        log(&s, "openai", 1200, NOV);
        assert!(s.take_budget_notices(NOV).unwrap().is_empty());
        s.set_provider_budgets(&[("openai".into(), 1000)]).unwrap();
        let n = s.take_budget_notices(NOV).unwrap();
        assert_eq!(
            n.iter()
                .map(|n| (n.threshold, n.spent_cents, n.budget_cents))
                .collect::<Vec<_>>(),
            vec![(80, 1200, 1000), (100, 1200, 1000)]
        );
        assert_eq!(
            s.provider_over_budget("openai", NOV).unwrap(),
            Some((1200, 1000))
        );
    }

    #[test]
    fn notices_rearm_in_a_new_month() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.set_provider_budgets(&[("anthropic".into(), 1000)])
            .unwrap();
        log(&s, "anthropic", 800, NOV);
        assert_eq!(
            s.take_budget_notices(NOV)
                .unwrap()
                .iter()
                .map(|n| n.threshold)
                .collect::<Vec<_>>(),
            vec![80]
        );
        // 2026-12-15: same budget, December's own spend reaches 80% again.
        let dec = NOV + 30 * 24 * 3600 * 1000;
        log(&s, "anthropic", 799, dec);
        assert!(s.take_budget_notices(dec).unwrap().is_empty());
        log(&s, "anthropic", 1, dec);
        let n = s.take_budget_notices(dec).unwrap();
        assert_eq!(
            n,
            vec![BudgetNotice {
                provider: "anthropic".into(),
                threshold: 80,
                spent_cents: 800,
                budget_cents: 1000,
                month: "2026-12".into()
            }]
        );
    }

    #[test]
    fn spend_hook_error_never_loses_the_usage_row() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.conn.execute("DROP TABLE provider_spend", []).unwrap();
        log(&s, "anthropic", 120, NOV);
        let rows: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM usage_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn budgets_are_clamped_and_huge_spend_never_overflows() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        s.set_provider_budgets(&[
            ("anthropic".into(), i64::MAX),
            ("openai".into(), 100_000_000),
        ])
        .unwrap();
        assert_eq!(
            s.provider_budgets().unwrap(),
            vec![
                ("anthropic".into(), 100_000_000),
                ("openai".into(), 100_000_000)
            ]
        );
        // Synced rows are capped, so many huge peers still sum without overflow.
        for peer in ["peer", "peer2", "peer3"] {
            s.apply_synced_provider_spend(peer, &row(i64::MAX / 2, 1))
                .unwrap();
        }
        assert_eq!(
            s.month_spend_by_provider("2026-11").unwrap(),
            vec![("anthropic".into(), 3 * MAX_ROW_CENTS)]
        );
        // A row past the cap (stored before it) still can't overflow the threshold math.
        s.conn.execute("DELETE FROM provider_spend", []).unwrap();
        s.conn
            .execute(
                "INSERT INTO provider_spend VALUES ('peer', 'p1', '2026-11', 'anthropic', ?1, 1)",
                [i64::MAX / 2],
            )
            .unwrap();
        let n = s.take_budget_notices(NOV).unwrap();
        assert_eq!(
            n.iter().map(|n| n.threshold).collect::<Vec<_>>(),
            vec![80, 100]
        );
        assert!(s.provider_over_budget("anthropic", NOV).unwrap().is_some());
    }

    #[test]
    fn over_budget_at_equal() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev");
        log(&s, "openrouter", 999, NOV);
        assert_eq!(
            s.provider_over_budget("openrouter", NOV).unwrap(),
            None,
            "no budget"
        );
        s.set_provider_budgets(&[("openrouter".into(), 1000)])
            .unwrap();
        assert_eq!(s.provider_over_budget("openrouter", NOV).unwrap(), None);
        log(&s, "openrouter", 1, NOV);
        assert_eq!(
            s.provider_over_budget("openrouter", NOV).unwrap(),
            Some((1000, 1000))
        );
    }
}
