//! Gmail: decoding the connector's `sync_messages` result and landing it in `items`.
//! No UI-facing output: its only job is rows for Thread distillation to read.

use memory::{NewEmailItem, Store};

use super::ToolCall;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodedMessage {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub internal_date: String,
    pub body_text: String,
    // Defaulted so output from a connector predating these fields still decodes.
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub list_unsubscribe: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncMessagesResponse {
    messages: Option<Vec<DecodedMessage>>,
    deleted_external_ids: Option<Vec<String>>,
    next_history_id: Option<String>,
    needs_full_resync: Option<bool>,
}

/// The result of one `sync_messages` call.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SyncResult {
    pub messages: Vec<DecodedMessage>,
    pub deleted_external_ids: Vec<String>,
    pub next_history_id: Option<String>,
    pub needs_full_resync: bool,
}

/// Malformed JSON degrades to an empty, non-resync result (a background poll never
/// crashes over a bad response).
pub fn sync_result_from_json(json: &str) -> SyncResult {
    let Ok(response) = serde_json::from_str::<SyncMessagesResponse>(json) else {
        return SyncResult::default();
    };
    if response.needs_full_resync == Some(true) {
        return SyncResult { needs_full_resync: true, ..Default::default() };
    }
    SyncResult {
        messages: response.messages.unwrap_or_default(),
        deleted_external_ids: response.deleted_external_ids.unwrap_or_default(),
        next_history_id: response.next_history_id,
        needs_full_resync: false,
    }
}

/// Phase 1: the `sync_messages` call for `account_email`, carrying its stored
/// historyId (or none for a baseline).
pub fn sync_request(store: &Store, account_email: &str) -> rusqlite::Result<ToolCall> {
    let args = match store.get_gmail_history_id(account_email)? {
        Some(id) => serde_json::json!({ "history_id": id }).to_string(),
        None => "{}".to_string(),
    };
    Ok(ToolCall { tool: "sync_messages".into(), args_json: args })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApplyOutcome {
    pub needs_full_resync: bool,
    pub deleted: usize,
    pub upserted: usize,
    /// Both the deletions and the upserts succeeded.
    pub applied: bool,
    pub history_saved: bool,
}

/// Phase 2: apply one `sync_messages` result. The cursor only advances if both the
/// deletion and the upsert succeed: Gmail never re-reports a change against a newer
/// historyId, so a premature save would permanently lose whatever failed to apply.
pub fn apply(store: &Store, account_email: &str, device_id: &str, result_json: &str) -> rusqlite::Result<ApplyOutcome> {
    let result = sync_result_from_json(result_json);
    if result.needs_full_resync {
        store.clear_gmail_history_id(account_email)?;
        return Ok(ApplyOutcome { needs_full_resync: true, deleted: 0, upserted: 0, applied: false, history_saved: false });
    }
    let inputs: Vec<NewEmailItem> = result
        .messages
        .iter()
        .map(|m| NewEmailItem {
            device_id: device_id.to_string(),
            external_id: m.id.clone(),
            ts_unix_ms: m.internal_date.parse().unwrap_or(0),
            from: m.from.clone(),
            to: m.to.clone(),
            subject: m.subject.clone(),
            body_text: m.body_text.clone(),
            gmail_thread_id: m.thread_id.clone(),
            account_email: account_email.to_string(),
            gmail_labels: m.label_ids.clone(),
            list_unsubscribe: m.list_unsubscribe,
        })
        .collect();
    let mut applied = true;
    let mut deleted = 0;
    if !result.deleted_external_ids.is_empty() {
        match store.delete_email_items(&result.deleted_external_ids) {
            Ok(n) => deleted = n,
            Err(_) => applied = false,
        }
    }
    let mut upserted = 0;
    if applied && !inputs.is_empty() {
        match store.upsert_email_items(&inputs) {
            Ok(()) => upserted = inputs.len(),
            Err(_) => applied = false,
        }
    }
    let mut history_saved = false;
    if applied {
        if let Some(next) = &result.next_history_id {
            history_saved = store.set_gmail_history_id(account_email, next).is_ok();
        }
    }
    Ok(ApplyOutcome { needs_full_resync: false, deleted, upserted, applied, history_saved })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- ported from GmailSyncDecodingTests.swift ----

    #[test]
    fn sync_result_decodes_messages_and_deletions() {
        let json = r#"{"messages": [{"id": "m1", "threadId": "t1", "from": "a@example.com", "to": "me@example.com", "subject": "Hi", "date": "Mon, 1 Jan 2026 10:00:00 -0500", "internalDate": "1700000000000", "bodyText": "Hello", "attachments": []}], "deletedExternalIds": ["m2"], "nextHistoryId": "105", "needsFullResync": false}"#;
        let result = sync_result_from_json(json);
        assert_eq!(result.messages, [DecodedMessage { id: "m1".into(), thread_id: "t1".into(), from: "a@example.com".into(), to: "me@example.com".into(), subject: "Hi".into(), internal_date: "1700000000000".into(), body_text: "Hello".into(), label_ids: vec![], list_unsubscribe: false }]);
        assert_eq!(result.deleted_external_ids, ["m2"]);
        assert_eq!(result.next_history_id.as_deref(), Some("105"));
        assert!(!result.needs_full_resync);
    }

    #[test]
    fn sync_result_decodes_label_ids_and_list_unsubscribe() {
        let json = r#"{"messages": [{"id": "m1", "threadId": "t1", "from": "a@x.com", "to": "me@x.com", "subject": "Sale", "internalDate": "1700000000000", "bodyText": "Hi", "labelIds": ["INBOX", "CATEGORY_PROMOTIONS"], "listUnsubscribe": true}]}"#;
        let m = &sync_result_from_json(json).messages[0];
        assert_eq!(m.label_ids, ["INBOX", "CATEGORY_PROMOTIONS"]);
        assert!(m.list_unsubscribe);
    }

    #[test]
    fn sync_result_detects_needs_full_resync_degrades_on_malformed_and_handles_empty_baseline() {
        let r = sync_result_from_json(r#"{"needsFullResync": true}"#);
        assert!(r.needs_full_resync && r.messages.is_empty() && r.deleted_external_ids.is_empty());
        let r = sync_result_from_json("not json");
        assert!(!r.needs_full_resync && r.messages.is_empty() && r.deleted_external_ids.is_empty());
        let r = sync_result_from_json(r#"{"messages": [], "deletedExternalIds": [], "nextHistoryId": "50", "needsFullResync": false}"#);
        assert!(r.messages.is_empty() && !r.needs_full_resync);
        assert_eq!(r.next_history_id.as_deref(), Some("50"));
    }

    #[test]
    fn request_carries_the_stored_history_id_and_apply_lands_rows_then_advances() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(sync_request(&store, "me@x.com").unwrap(), ToolCall { tool: "sync_messages".into(), args_json: "{}".into() });
        let json = r#"{"messages": [{"id": "m1", "threadId": "t1", "from": "a@x.com", "to": "me@x.com", "subject": "Hi", "internalDate": "1700000000000", "bodyText": "Hello"}], "deletedExternalIds": ["m-old"], "nextHistoryId": "105"}"#;
        let out = apply(&store, "me@x.com", "dev-1", json).unwrap();
        assert_eq!(out, ApplyOutcome { needs_full_resync: false, deleted: 0, upserted: 1, applied: true, history_saved: true });
        let req = sync_request(&store, "me@x.com").unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&req.args_json).unwrap()["history_id"], "105");
        // Deleting what was just upserted counts.
        let out = apply(&store, "me@x.com", "dev-1", r#"{"deletedExternalIds": ["m1"], "nextHistoryId": "106"}"#).unwrap();
        assert_eq!((out.deleted, out.history_saved), (1, true));
        // Resync clears the cursor.
        let out = apply(&store, "me@x.com", "dev-1", r#"{"needsFullResync": true}"#).unwrap();
        assert!(out.needs_full_resync);
        assert_eq!(sync_request(&store, "me@x.com").unwrap().args_json, "{}");
    }
}
