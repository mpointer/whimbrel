//! Connected-account registry per connector (M3.0 part 2, Task 9), moved from
//! apps/macos `ConnectedAccountStore.swift` (a UserDefaults list) to the
//! `connector_account` table in `core/memory`. Tracks identity and default only: the
//! OAuth tokens stay in the shell's secret store, keyed by `(connector, email)`.
//! Accounts are device-local (a secret-store item never crosses devices), so this table
//! is deliberately outside settings sync.

use memory::Store;

/// One connector's registry, in connection order. Serializes as
/// `{"emails":[…],"default_email":…}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Accounts {
    pub emails: Vec<String>,
    pub default_email: Option<String>,
}

pub fn list(store: &Store, connector: &str) -> rusqlite::Result<Accounts> {
    let rows = store.list_connector_accounts(connector)?;
    Ok(Accounts {
        default_email: rows.iter().find(|r| r.is_default).map(|r| r.email.clone()),
        emails: rows.into_iter().map(|r| r.email).collect(),
    })
}

/// Adds `email` if it isn't already connected, marking it default automatically if it
/// is the first account for this connector. Returns `false` (no-op) if already present:
/// connecting the same account twice must never create a duplicate entry.
pub fn add(store: &Store, connector: &str, email: &str, now_unix_ms: i64) -> rusqlite::Result<bool> {
    let existing = store.list_connector_accounts(connector)?;
    if existing.iter().any(|r| r.email == email) {
        return Ok(false);
    }
    let is_default = !existing.iter().any(|r| r.is_default);
    let position = existing.iter().map(|r| r.position).max().map_or(0, |p| p + 1);
    store.insert_connector_account(connector, email, is_default, position, now_unix_ms)?;
    Ok(true)
}

/// Removes `email`. If it was the default and other accounts remain, the next remaining
/// one in connection order becomes default; if none remain, there is no default.
pub fn remove(store: &Store, connector: &str, email: &str) -> rusqlite::Result<()> {
    let existing = store.list_connector_accounts(connector)?;
    let Some(removed) = existing.iter().find(|r| r.email == email) else {
        return Ok(());
    };
    let was_default = removed.is_default;
    store.delete_connector_account(connector, email)?;
    if was_default {
        if let Some(next) = existing.iter().find(|r| r.email != email) {
            store.set_default_connector_account(connector, &next.email)?;
        }
    }
    Ok(())
}

/// No-op if `email` isn't connected: the default must always be a connected account
/// (or none), regardless of caller.
pub fn set_default(store: &Store, connector: &str, email: &str) -> rusqlite::Result<()> {
    if store.list_connector_accounts(connector)?.iter().any(|r| r.email == email) {
        store.set_default_connector_account(connector, email)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    // ---- ported from ConnectedAccountStoreTests.swift ----

    #[test]
    fn adding_first_account_marks_it_default() {
        let s = store();
        assert!(add(&s, "calendar", "alice@personal.com", 1).unwrap());
        assert_eq!(list(&s, "calendar").unwrap(), Accounts { emails: vec!["alice@personal.com".into()], default_email: Some("alice@personal.com".into()) });
    }

    #[test]
    fn adding_second_account_does_not_change_default() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        assert!(add(&s, "calendar", "alice@genesys.com", 2).unwrap());
        let a = list(&s, "calendar").unwrap();
        assert_eq!(a.emails, ["alice@personal.com", "alice@genesys.com"]);
        assert_eq!(a.default_email.as_deref(), Some("alice@personal.com"));
    }

    #[test]
    fn adding_already_connected_email_is_a_no_op() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        assert!(!add(&s, "calendar", "alice@personal.com", 2).unwrap());
        assert_eq!(list(&s, "calendar").unwrap().emails, ["alice@personal.com"]);
    }

    #[test]
    fn removing_non_default_account_leaves_default_unchanged() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        add(&s, "calendar", "alice@genesys.com", 2).unwrap();
        remove(&s, "calendar", "alice@genesys.com").unwrap();
        let a = list(&s, "calendar").unwrap();
        assert_eq!(a.emails, ["alice@personal.com"]);
        assert_eq!(a.default_email.as_deref(), Some("alice@personal.com"));
    }

    #[test]
    fn removing_default_account_promotes_the_next_remaining_one() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        add(&s, "calendar", "alice@genesys.com", 2).unwrap();
        remove(&s, "calendar", "alice@personal.com").unwrap();
        let a = list(&s, "calendar").unwrap();
        assert_eq!(a.emails, ["alice@genesys.com"]);
        assert_eq!(a.default_email.as_deref(), Some("alice@genesys.com"));
    }

    #[test]
    fn removing_the_only_account_clears_the_default() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        remove(&s, "calendar", "alice@personal.com").unwrap();
        assert_eq!(list(&s, "calendar").unwrap(), Accounts { emails: vec![], default_email: None });
        remove(&s, "calendar", "nobody@example.com").unwrap();
    }

    #[test]
    fn make_default_switches_among_connected_and_ignores_unconnected() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        add(&s, "calendar", "alice@genesys.com", 2).unwrap();
        set_default(&s, "calendar", "alice@genesys.com").unwrap();
        assert_eq!(list(&s, "calendar").unwrap().default_email.as_deref(), Some("alice@genesys.com"));
        set_default(&s, "calendar", "nobody@example.com").unwrap();
        assert_eq!(list(&s, "calendar").unwrap().default_email.as_deref(), Some("alice@genesys.com"));
        assert_eq!(list(&s, "calendar").unwrap().emails, ["alice@personal.com", "alice@genesys.com"], "order is connection order");
    }

    #[test]
    fn calendar_and_gmail_registries_are_independent() {
        let s = store();
        add(&s, "calendar", "alice@personal.com", 1).unwrap();
        add(&s, "gmail", "alice@genesys.com", 2).unwrap();
        assert_eq!(list(&s, "calendar").unwrap().emails, ["alice@personal.com"]);
        assert_eq!(list(&s, "gmail").unwrap().emails, ["alice@genesys.com"]);
        assert_eq!(list(&s, "outlook365").unwrap(), Accounts { emails: vec![], default_email: None });
    }

    #[test]
    fn removal_then_re_add_appends_at_the_end() {
        let s = store();
        add(&s, "gmail", "a@x.com", 1).unwrap();
        add(&s, "gmail", "b@x.com", 2).unwrap();
        remove(&s, "gmail", "a@x.com").unwrap();
        add(&s, "gmail", "a@x.com", 3).unwrap();
        let a = list(&s, "gmail").unwrap();
        assert_eq!(a.emails, ["b@x.com", "a@x.com"]);
        assert_eq!(a.default_email.as_deref(), Some("b@x.com"));
    }
}
