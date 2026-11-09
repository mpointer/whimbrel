//! The TypeSafe ("Jev") token budget, scoped per feature: each feature keeps its own
//! first-pass budget, daily allowance and cost counter under its own kv prefix.

use rusqlite::{params, Connection, OptionalExtension};

const DAY_MS: i64 = 86_400_000;
/// Tokens a day after the first pass: about $0.01 at $0.042 per million.
pub const DAILY_ALLOWANCE_TOKENS: i64 = 250_000;

/// Whether a feature may call TypeSafe now (`JevBudget::gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeGate {
    /// `start` was never called for this feature.
    NotStarted,
    Open,
    /// First pass: the tokens used reached the budget set at Start.
    OverBudget,
    /// After the first pass: today's `DAILY_ALLOWANCE_TOKENS` are used up.
    PausedForToday,
}

/// One feature's budget; the prefix names its kv keys (`{prefix}_budget_tokens`, ...).
#[derive(Debug, Clone, Copy)]
pub struct JevBudget(pub &'static str);

pub const TOPIC_JUDGE: JevBudget = JevBudget("topic_judge");
pub const TOPIC_STEPS: JevBudget = JevBudget("topic_steps");

fn kv_i64(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT value FROM kv_meta WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| v.parse().ok()))
}

fn kv_put(conn: &Connection, key: &str, v: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, v.to_string()],
    )?;
    Ok(())
}

impl JevBudget {
    fn key(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.0)
    }

    /// Starts a first pass: up to `budget_tokens` until the queue drains, then
    /// `DAILY_ALLOWANCE_TOKENS` a day. Resets the tokens used.
    pub fn start(&self, conn: &Connection, budget_tokens: i64) -> rusqlite::Result<()> {
        kv_put(conn, &self.key("budget_tokens"), budget_tokens)?;
        kv_put(conn, &self.key("used_tokens"), 0)?;
        conn.execute(
            "DELETE FROM kv_meta WHERE key IN (?1, ?2, ?3)",
            params![
                self.key("first_pass_done"),
                self.key("day"),
                self.key("day_used_tokens")
            ],
        )?;
        Ok(())
    }

    pub fn gate(&self, conn: &Connection, now_ms: i64) -> rusqlite::Result<JudgeGate> {
        let Some(budget) = kv_i64(conn, &self.key("budget_tokens"))? else {
            return Ok(JudgeGate::NotStarted);
        };
        Ok(if kv_i64(conn, &self.key("first_pass_done"))?.is_some() {
            let today = now_ms / DAY_MS;
            let used = if kv_i64(conn, &self.key("day"))? == Some(today) {
                kv_i64(conn, &self.key("day_used_tokens"))?.unwrap_or(0)
            } else {
                0
            };
            if used >= DAILY_ALLOWANCE_TOKENS {
                JudgeGate::PausedForToday
            } else {
                JudgeGate::Open
            }
        } else if kv_i64(conn, &self.key("used_tokens"))?.unwrap_or(0) >= budget {
            JudgeGate::OverBudget
        } else {
            JudgeGate::Open
        })
    }

    /// Counts tokens spent: toward the first pass budget, and after the first pass toward
    /// today's allowance.
    pub fn add_used(&self, conn: &Connection, tokens: i64, now_ms: i64) -> rusqlite::Result<()> {
        let add = |key: String| {
            conn.execute(
                "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + CAST(excluded.value AS INTEGER)",
                params![key, tokens.to_string()],
            )
        };
        add(self.key("used_tokens"))?;
        if kv_i64(conn, &self.key("first_pass_done"))?.is_some() {
            let today = now_ms / DAY_MS;
            if kv_i64(conn, &self.key("day"))? != Some(today) {
                kv_put(conn, &self.key("day"), today)?;
                kv_put(conn, &self.key("day_used_tokens"), 0)?;
            }
            add(self.key("day_used_tokens"))?;
        }
        Ok(())
    }

    /// Whole cents to log for a call of `tokens`: the running cost's cents after the call
    /// minus before it, so per-call fractions of a cent carry.
    pub fn cost_cents(
        &self,
        conn: &Connection,
        tokens: i64,
        usd_per_million: f64,
    ) -> rusqlite::Result<i64> {
        let key = self.key("cost_tokens");
        let before = kv_i64(conn, &key)?.unwrap_or(0);
        let after = before + tokens.max(0);
        kv_put(conn, &key, after)?;
        let cents = |t: i64| (t as f64 * usd_per_million / 10_000.0).floor() as i64;
        Ok(cents(after) - cents(before))
    }

    /// `(budget, used)` tokens, or `None` before `start`.
    pub fn budget(&self, conn: &Connection) -> rusqlite::Result<Option<(i64, i64)>> {
        Ok(match kv_i64(conn, &self.key("budget_tokens"))? {
            Some(b) => Some((b, kv_i64(conn, &self.key("used_tokens"))?.unwrap_or(0))),
            None => None,
        })
    }

    /// Whether the first pass has ended.
    pub fn first_pass_done(&self, conn: &Connection) -> rusqlite::Result<bool> {
        Ok(kv_i64(conn, &self.key("first_pass_done"))?.is_some())
    }

    pub fn mark_first_pass_done(&self, conn: &Connection) -> rusqlite::Result<()> {
        kv_put(conn, &self.key("first_pass_done"), 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn scopes_do_not_share_tokens_or_days() {
        let s = Store::open_in_memory().unwrap();
        let c = &s.conn;
        TOPIC_JUDGE.start(c, 1_000).unwrap();
        TOPIC_STEPS.start(c, 1_000).unwrap();
        TOPIC_STEPS.add_used(c, 1_000, 0).unwrap();
        assert_eq!(TOPIC_STEPS.gate(c, 0).unwrap(), JudgeGate::OverBudget);
        assert_eq!(TOPIC_JUDGE.gate(c, 0).unwrap(), JudgeGate::Open);
        // After the first pass the day counters are separate too.
        TOPIC_STEPS.mark_first_pass_done(c).unwrap();
        TOPIC_JUDGE.mark_first_pass_done(c).unwrap();
        TOPIC_STEPS
            .add_used(c, DAILY_ALLOWANCE_TOKENS, DAY_MS)
            .unwrap();
        assert_eq!(
            TOPIC_STEPS.gate(c, DAY_MS).unwrap(),
            JudgeGate::PausedForToday
        );
        assert_eq!(TOPIC_JUDGE.gate(c, DAY_MS).unwrap(), JudgeGate::Open);
        assert_eq!(TOPIC_JUDGE.budget(c).unwrap(), Some((1_000, 0)));
        assert_eq!(
            TOPIC_STEPS.cost_cents(c, 1_000_000, 10_000.0).unwrap(),
            1_000_000
        );
        assert_eq!(
            TOPIC_JUDGE.cost_cents(c, 1_000_000, 10_000.0).unwrap(),
            1_000_000
        );
    }

    #[test]
    fn judge_keys_are_unchanged() {
        let s = Store::open_in_memory().unwrap();
        s.start_topic_judging(5).unwrap();
        let v: String = s
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'topic_judge_budget_tokens'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "5");
    }
}
