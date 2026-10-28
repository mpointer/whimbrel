//! User corrections to topics: rename, merge, split, remove a member, add a member.
//! Each is one transaction. `topic_cluster` never touches `user` or `removed` member rows,
//! and a topic whose members are all tombstones stays (hidden) so a removal sticks.

use rusqlite::{params, Connection};

use crate::{now_unix_ms, Store};

fn invalid(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(msg.into().into())
}

/// Errors unless `id` exists and is not merged into another topic.
fn live_topic(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    match conn.query_row(
        "SELECT merged_into IS NOT NULL FROM topic WHERE id = ?1",
        [id],
        |r| r.get::<_, bool>(0),
    ) {
        Ok(true) => Err(invalid(format!("topic {id} is merged"))),
        Ok(false) => Ok(()),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(invalid(format!("topic {id} not found"))),
        Err(e) => Err(e),
    }
}

fn mark_stale(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO topic_stats (topic_id, vec_n, vec_sum) VALUES (?1, -1, NULL) \
         ON CONFLICT(topic_id) DO UPDATE SET vec_n = -1, vec_sum = NULL",
        [id],
    )?;
    Ok(())
}

fn touch(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE topic SET updated_at_unix_ms = ?2 WHERE id = ?1",
        params![id, now_unix_ms()],
    )?;
    Ok(())
}

fn set_user_member(conn: &Connection, topic_id: i64, source_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO topic_member (topic_id, source_id, origin, score) VALUES (?1, ?2, 'user', 1.0) \
         ON CONFLICT(topic_id, source_id) DO UPDATE SET origin = 'user'",
        [topic_id, source_id],
    )?;
    Ok(())
}

impl Store {
    pub fn rename_topic(&self, id: i64, name: &str) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        live_topic(&tx, id)?;
        tx.execute(
            "UPDATE topic SET name = ?2, name_is_user = 1, name_dirty = 0, updated_at_unix_ms = ?3 WHERE id = ?1",
            params![id, name, now_unix_ms()],
        )?;
        tx.commit()
    }

    /// Copies `absorb`'s members into `keep` (strongest origin wins; `removed` sticks) and
    /// hides `absorb` via `merged_into`. Absorbed member rows stay behind, inert, so the
    /// topic row survives `prune_orphans` and old ids still resolve to `keep`.
    pub fn merge_topics(&self, keep: i64, absorb: i64) -> rusqlite::Result<()> {
        if keep == absorb {
            return Err(invalid("cannot merge a topic into itself"));
        }
        let tx = self.conn.unchecked_transaction()?;
        live_topic(&tx, keep)?;
        live_topic(&tx, absorb)?;
        tx.execute(
            "INSERT INTO topic_member (topic_id, source_id, origin, score) \
             SELECT ?1, source_id, origin, score FROM topic_member WHERE topic_id = ?2 \
             ON CONFLICT(topic_id, source_id) DO UPDATE SET \
               origin = CASE WHEN excluded.origin = 'removed' OR topic_member.origin = 'removed' THEN 'removed' \
                             WHEN excluded.origin = 'user' OR topic_member.origin = 'user' THEN 'user' \
                             ELSE 'auto' END, \
               score = MAX(topic_member.score, excluded.score)",
            [keep, absorb],
        )?;
        tx.execute(
            "UPDATE topic_idea SET topic_id = ?1 WHERE topic_id = ?2",
            [keep, absorb],
        )?;
        // Flatten chains: topics already merged into `absorb` now point at `keep`.
        tx.execute("UPDATE topic SET merged_into = ?1 WHERE merged_into = ?2", [keep, absorb])?;
        let now = now_unix_ms();
        tx.execute(
            "UPDATE topic SET merged_into = ?2, updated_at_unix_ms = ?3 WHERE id = ?1",
            params![absorb, keep, now],
        )?;
        tx.execute(
            "UPDATE topic SET name_dirty = CASE WHEN name_is_user = 1 THEN name_dirty ELSE 1 END, \
               last_member_ts_unix_ms = MAX(COALESCE(last_member_ts_unix_ms, 0), \
                 COALESCE((SELECT last_member_ts_unix_ms FROM topic WHERE id = ?2), 0)), \
               updated_at_unix_ms = ?3 WHERE id = ?1",
            params![keep, absorb, now],
        )?;
        mark_stale(&tx, keep)?;
        tx.commit()
    }

    /// Moves `source_ids` (all must be non-removed members of `id`) into a new topic as
    /// `user` members; the original keeps `removed` tombstones. Returns the new topic id.
    pub fn split_topic(
        &self,
        id: i64,
        source_ids: &[i64],
        new_name: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        live_topic(&tx, id)?;
        if source_ids.is_empty() {
            return Err(invalid("split needs at least one source"));
        }
        for s in source_ids {
            let ok: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM topic_member WHERE topic_id = ?1 AND source_id = ?2 AND origin != 'removed')",
                [id, *s],
                |r| r.get(0),
            )?;
            if !ok {
                return Err(invalid(format!("source {s} is not a member of topic {id}")));
            }
        }
        let now = now_unix_ms();
        let ids_json = serde_json::to_string(source_ids).map_err(crate::json_to_sql_error)?;
        tx.execute(
            "INSERT INTO topic (name, name_is_user, name_dirty, created_at_unix_ms, updated_at_unix_ms, last_member_ts_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?4, (SELECT MAX(ts_unix_ms) FROM topic_source WHERE id IN (SELECT value FROM json_each(?5))))",
            params![new_name.unwrap_or(""), new_name.is_some(), new_name.is_none(), now, ids_json],
        )?;
        let new_id = tx.last_insert_rowid();
        for s in source_ids {
            set_user_member(&tx, new_id, *s)?;
            tx.execute(
                "UPDATE topic_member SET origin = 'removed' WHERE topic_id = ?1 AND source_id = ?2",
                [id, *s],
            )?;
        }
        tx.execute(
            "UPDATE topic SET name_dirty = 1 WHERE id = ?1 AND name_is_user = 0",
            [id],
        )?;
        touch(&tx, id)?;
        mark_stale(&tx, id)?;
        mark_stale(&tx, new_id)?;
        tx.commit()?;
        Ok(new_id)
    }

    pub fn remove_from_topic(&self, topic_id: i64, source_id: i64) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        live_topic(&tx, topic_id)?;
        let n = tx.execute(
            "UPDATE topic_member SET origin = 'removed' WHERE topic_id = ?1 AND source_id = ?2",
            [topic_id, source_id],
        )?;
        if n == 0 {
            return Err(invalid(format!(
                "source {source_id} is not in topic {topic_id}"
            )));
        }
        touch(&tx, topic_id)?;
        mark_stale(&tx, topic_id)?;
        tx.commit()
    }

    pub fn add_to_topic(&self, topic_id: i64, source_id: i64) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        live_topic(&tx, topic_id)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM topic_source WHERE id = ?1)",
            [source_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(invalid(format!("source {source_id} not found")));
        }
        set_user_member(&tx, topic_id, source_id)?;
        touch(&tx, topic_id)?;
        mark_stale(&tx, topic_id)?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::topics::cluster::run_topic_cluster_job;
    use crate::topics::fixture::{build, Fixture, ACME, BOARD, HOME};

    fn run(s: &Store) {
        while run_topic_cluster_job(s, 200).unwrap() {}
    }

    fn recluster(s: &Store) {
        s.conn
            .execute("UPDATE topic_source SET assigned_at_unix_ms = NULL", [])
            .unwrap();
        run(s);
    }

    /// Live (unmerged) topic id -> its non-removed members.
    fn live(s: &Store) -> BTreeMap<i64, BTreeSet<i64>> {
        let mut out: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        let mut stmt = s
            .conn
            .prepare(
                "SELECT tm.topic_id, tm.source_id FROM topic_member tm JOIN topic t ON t.id = tm.topic_id \
                 WHERE tm.origin != 'removed' AND t.merged_into IS NULL",
            )
            .unwrap();
        for r in stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
        {
            let (t, m) = r.unwrap();
            out.entry(t).or_default().insert(m);
        }
        out
    }

    /// Live topic overlapping `label` the most.
    fn topic_of(f: &Fixture, label: &str) -> i64 {
        let want = &f.labels[label];
        live(&f.store)
            .into_iter()
            .max_by_key(|(_, ms)| ms.intersection(want).count())
            .map(|(t, _)| t)
            .expect("no topic")
    }

    fn origin(s: &Store, t: i64, m: i64) -> Option<String> {
        s.conn
            .query_row(
                "SELECT origin FROM topic_member WHERE topic_id = ?1 AND source_id = ?2",
                [t, m],
                |r| r.get(0),
            )
            .ok()
    }

    /// (name, name_is_user, name_dirty)
    fn name(s: &Store, t: i64) -> (String, bool, bool) {
        s.conn
            .query_row(
                "SELECT name, name_is_user, name_dirty FROM topic WHERE id = ?1",
                [t],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    fn setup() -> (Fixture, i64, i64) {
        let f = build(true);
        run(&f.store);
        let (a, h) = (topic_of(&f, ACME), topic_of(&f, HOME));
        assert_ne!(a, h);
        (f, a, h)
    }

    #[test]
    fn rename_survives_renaming_job() {
        let (f, a, _) = setup();
        // A pending auto-name (name_dirty = 1) must lose to the user's rename.
        f.store
            .conn
            .execute("UPDATE topic SET name_dirty = 1 WHERE id = ?1", [a])
            .unwrap();
        f.store.rename_topic(a, "Mine").unwrap();
        f.store.apply_topic_name(a, "Auto", Some("status")).unwrap();
        assert_eq!(name(&f.store, a), ("Mine".into(), true, false));
        assert_eq!(name(&f.store, a), ("Mine".into(), true, false));
        recluster(&f.store);
        assert_eq!(name(&f.store, a), ("Mine".into(), true, false));
        assert!(f.store.rename_topic(9999, "x").is_err());
    }

    #[test]
    fn merge_moves_members_and_hides_absorbed() {
        let (f, a, h) = setup();
        let s = &f.store;
        let hm: BTreeSet<i64> = live(s)[&h].clone();
        let mut it = hm.iter();
        let (u, r) = (*it.next().unwrap(), *it.next().unwrap());
        // `u`: auto in keep, user in absorb => user. `r`: auto in keep, removed in absorb => removed.
        s.conn
            .execute(
                "INSERT OR REPLACE INTO topic_member VALUES (?1, ?2, 'auto', 1.0)",
                [a, u],
            )
            .unwrap();
        s.conn
            .execute(
                "UPDATE topic_member SET origin='user' WHERE topic_id=?1 AND source_id=?2",
                [h, u],
            )
            .unwrap();
        s.conn
            .execute(
                "INSERT OR REPLACE INTO topic_member VALUES (?1, ?2, 'auto', 1.0)",
                [a, r],
            )
            .unwrap();
        s.conn
            .execute(
                "UPDATE topic_member SET origin='removed' WHERE topic_id=?1 AND source_id=?2",
                [h, r],
            )
            .unwrap();
        s.conn
            .execute("INSERT INTO topic_idea VALUES (777, ?1)", [h])
            .unwrap();
        s.conn
            .execute("UPDATE topic SET name_dirty = 0 WHERE id = ?1", [a])
            .unwrap();

        s.merge_topics(a, h).unwrap();
        let l = live(s);
        assert!(!l.contains_key(&h));
        for m in hm.iter().filter(|m| **m != r) {
            assert!(l[&a].contains(m));
        }
        assert_eq!(origin(s, a, u).as_deref(), Some("user"));
        assert_eq!(origin(s, a, r).as_deref(), Some("removed"));
        let merged: i64 = s
            .conn
            .query_row("SELECT merged_into FROM topic WHERE id=?1", [h], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(merged, a);
        let idea: i64 = s
            .conn
            .query_row(
                "SELECT topic_id FROM topic_idea WHERE entity_id=777",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idea, a);
        assert!(name(s, a).2, "keep should be renamed");
        let vn: i64 = s
            .conn
            .query_row(
                "SELECT vec_n FROM topic_stats WHERE topic_id=?1",
                [a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vn, -1);
        assert!(s.merge_topics(a, a).is_err());
        assert!(s.merge_topics(a, h).is_err());
        assert!(s.merge_topics(h, a).is_err());
    }

    #[test]
    fn merge_chain_is_flattened() {
        let (f, a, h) = setup();
        let s = &f.store;
        let c = topic_of(&f, BOARD);
        s.merge_topics(a, h).unwrap();
        s.merge_topics(c, a).unwrap();
        let m: i64 = s.conn.query_row("SELECT merged_into FROM topic WHERE id=?1", [h], |r| r.get(0)).unwrap();
        assert_eq!(m, c);
    }

    #[test]
    fn split_creates_user_topic_and_tombstones() {
        let (f, a, _) = setup();
        let s = &f.store;
        let pick: Vec<i64> = live(s)[&a].iter().take(2).copied().collect();
        let n = s.split_topic(a, &pick, Some("Spun off")).unwrap();
        assert_eq!(name(s, n), ("Spun off".into(), true, false));
        for p in &pick {
            assert_eq!(origin(s, n, *p).as_deref(), Some("user"));
            assert_eq!(origin(s, a, *p).as_deref(), Some("removed"));
        }
        let rest = *live(s)[&a].iter().next().unwrap();
        let n2 = s.split_topic(a, &[rest], None).unwrap();
        assert_eq!((name(s, n2).1, name(s, n2).2), (false, true));
        // A removed or foreign source fails the whole split and changes nothing.
        let before = live(s);
        assert!(s.split_topic(a, &[pick[0]], None).is_err());
        let other = *before
            .values()
            .flatten()
            .find(|m| !before[&a].contains(m))
            .unwrap();
        let ok = *before[&a].iter().next().unwrap();
        assert!(s.split_topic(a, &[ok, other], None).is_err());
        assert_eq!(live(s), before);
        assert!(s.split_topic(a, &[], None).is_err());
    }

    #[test]
    fn remove_only_affects_that_topic() {
        let f = build(true);
        run(&f.store);
        let (a, b) = (topic_of(&f, ACME), topic_of(&f, BOARD));
        assert!(live(&f.store)[&a].contains(&f.weekly) && live(&f.store)[&b].contains(&f.weekly));
        f.store.remove_from_topic(a, f.weekly).unwrap();
        assert_eq!(origin(&f.store, a, f.weekly).as_deref(), Some("removed"));
        assert_eq!(origin(&f.store, b, f.weekly).as_deref(), Some("auto"));
        f.store.add_to_topic(a, f.weekly).unwrap();
        assert_eq!(origin(&f.store, a, f.weekly).as_deref(), Some("user"));
        assert!(f.store.remove_from_topic(a, -5).is_err());
        assert!(f.store.add_to_topic(a, -5).is_err());
    }

    #[test]
    fn removing_every_member_hides_topic_and_it_does_not_return() {
        let (f, _, h) = setup();
        let s = &f.store;
        let members: Vec<i64> = live(s)[&h].iter().copied().collect();
        for m in &members {
            s.remove_from_topic(h, *m).unwrap();
        }
        assert!(!live(s).contains_key(&h));
        run(s);
        recluster(s);
        assert!(!live(s).contains_key(&h));
        for m in &members {
            assert_eq!(origin(s, h, *m).as_deref(), Some("removed"));
        }
    }

    #[test]
    fn edits_survive_recluster() {
        let (f, a, h) = setup();
        let s = &f.store;
        let b = topic_of(&f, BOARD);
        let pick: Vec<i64> = live(s)[&a].iter().take(2).copied().collect();
        let n = s.split_topic(a, &pick, Some("Spun off")).unwrap();
        recluster(s);
        assert_eq!(name(s, n).0, "Spun off");
        for p in &pick {
            assert_eq!(origin(s, n, *p).as_deref(), Some("user"));
            assert_eq!(origin(s, a, *p).as_deref(), Some("removed"));
        }
        s.rename_topic(b, "Board!").unwrap();
        recluster(s);
        assert_eq!(name(s, b).0, "Board!");
        let victim = *live(s)[&h].iter().next().unwrap();
        s.remove_from_topic(h, victim).unwrap();
        recluster(s);
        assert_eq!(origin(s, h, victim).as_deref(), Some("removed"));
        s.merge_topics(b, h).unwrap();
        recluster(s);
        assert!(!live(s).contains_key(&h));
        assert_eq!(origin(s, b, victim).as_deref(), Some("removed"));
        let noise = *f.labels["noise"].iter().next().unwrap();
        s.add_to_topic(b, noise).unwrap();
        recluster(s);
        assert_eq!(origin(s, b, noise).as_deref(), Some("user"));
        assert_eq!(name(s, b).0, "Board!");
    }
}
