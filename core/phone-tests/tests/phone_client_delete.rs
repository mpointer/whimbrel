//! X6 Task 11, the phone's side of a client delete, without a real Turso: a Mac deletes a client through
//! the same call the shells make (`api::client_controls::delete`), and a phone store applies it with the
//! pull-only round the iPhone and Android apps run (`sync_engine::pull_only_with_connection`). The phone
//! never starts a delete; it applies the tombstones through the shared pull and hands the files of what it
//! removed to the app (`take_forgotten_meeting_files`), which the apps drain.

use api::client_controls::{delete, inventory, DeleteRequest, Folders};
use memory::{MeetingStatus, MeetingTrigger, NewMeeting, Store};
use rusqlite::params;
use sync::GroupKey;

const NAME: &str = "Zorblax";

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// One Mac content-sync round (`sync_engine::sync_content` after connecting).
async fn mac_round(conn: &libsql::Connection, key: &GroupKey, store: &Store, device: &str) {
    sync_engine::ensure_content_schema(conn).await.unwrap();
    sync_engine::push_content(conn, key, store, device).await.unwrap();
    sync_engine::pull_content(conn, key, store, device).await.unwrap();
    sync_engine::dispatch_outbox(conn, key, store, device).await.unwrap();
}

/// Files source `kind`/`id` under the client's topic, as the topic index does.
fn file(db: &rusqlite::Connection, topic: i64, kind: &str, id: i64) {
    db.execute(
        "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, indexed_at_unix_ms) VALUES (?1, ?2, 1000, 1)",
        params![kind, id],
    )
    .unwrap();
    let source = db.last_insert_rowid();
    db.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![topic, source])
        .unwrap();
}

fn calendar_events(store: &Store) -> usize {
    store
        .export_items_page("calendar_event", 0, 1_000)
        .unwrap()
        .into_iter()
        .filter(|r| r.title.as_deref().unwrap_or("").contains(NAME))
        .count()
}

fn has_meeting_titled(store: &Store, title: &str) -> bool {
    store.recent_meetings(100).unwrap().iter().any(|m| m.title == title)
}

/// A phone that synced a client's meeting (with audio on the Mac) and calendar event applies the Mac's delete
/// on its next pull: the meeting, the event and the client's seed are gone, and the removed meeting is reported
/// to the app on the existing files channel (with no paths: the phone holds no meeting files). The Mac's own
/// audio file is removed by the delete itself.
#[tokio::test]
async fn phone_pull_applies_client_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let relay = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
    let conn = relay.connect().unwrap();
    let key = GroupKey::generate();

    // The Mac: a client with a recorded meeting (audio in the media folder) and a calendar event.
    let mac_db = dir.path().join("mac.db");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    let mac = Store::open(&mac_db, dir.path()).unwrap();
    mac.set_own_device_id("mac-a");
    let seed = mac.create_seed_topic(NAME, &[], None).unwrap();
    let audio = media.join("system-audio-1.m4a");
    std::fs::write(&audio, b"audio").unwrap();
    let start = now_ms() - 3_600_000;
    let meeting = mac
        .insert_meeting(&NewMeeting {
            device_id: "mac-a".into(),
            start_unix_ms: start,
            end_unix_ms: None,
            title: format!("{NAME} kickoff"),
            attendees: vec![],
            trigger: MeetingTrigger::Calendar,
            audio_path: Some(audio.to_string_lossy().into_owned()),
            status: MeetingStatus::Recording,
        })
        .unwrap();
    let audio_path = audio.to_string_lossy().into_owned();
    mac.complete_meeting(meeting, start + 1_800_000, Some(&audio_path)).unwrap();
    mac.upsert_calendar_event_items(&[memory::NewCalendarEventItem {
        device_id: "mac-a".into(),
        source: "google_calendar".into(),
        external_id: "ev-1#1700000000".into(),
        ts_unix_ms: start,
        title: format!("{NAME} kickoff"),
        actors: vec![],
    }])
    .unwrap();
    let db = rusqlite::Connection::open(&mac_db).unwrap();
    let event: i64 = db
        .query_row("SELECT id FROM items WHERE kind = 'calendar_event'", [], |r| r.get(0))
        .unwrap();
    file(&db, seed.topic_id, "meeting", meeting);
    file(&db, seed.topic_id, "calendar_event", event);
    // The page the phone shows for the client (it names the client).
    assert!(mac.publish_topic_page_snapshots("mac-a", now_ms()).unwrap() > 0);
    mac_round(&conn, &key, &mac, "mac-a").await;

    // The phone pulls it all.
    let phone = Store::open_in_memory().unwrap();
    phone.set_own_device_id("phone-1");
    let options = sync_engine::PullOptions::default();
    let pull = || sync_engine::pull_only_with_connection(&conn, &key, &phone, "phone-1", &options);
    pull().await.unwrap();
    assert!(has_meeting_titled(&phone, &format!("{NAME} kickoff")), "the phone has the meeting");
    assert_eq!(calendar_events(&phone), 1, "the phone has the event");
    assert!(phone.list_seed_topics().unwrap().iter().any(|s| s.key == seed.key), "the phone has the seed");
    assert!(phone.topic_page_snapshot(&seed.key).unwrap().is_some(), "the phone has the client's page");

    // The Mac deletes the client, as the shells do.
    let folders = Folders { media_roots: vec![media.clone()], vault: None, vault_sync_off: false };
    let inv = inventory(&mac, &seed.key, &folders).unwrap();
    assert!(inv.linked.len() >= 2, "{inv:?}");
    let req = DeleteRequest { key: seed.key.clone(), include_possible: vec![], delete_exports: false, folders };
    let done = delete(&mac, &req, Some("mac-a"), Some(&key)).unwrap();
    assert!(done.complete, "{done:?}");
    assert!(!audio.exists(), "the Mac's own audio file is removed");
    mac_round(&conn, &key, &mac, "mac-a").await;
    let mut marker = conn
        .query(
            "SELECT 1 FROM sync_row WHERE device_id = 'phone-1' AND local_seq = 0 AND row_type = 'client_controls_capability'",
            (),
        )
        .await
        .unwrap();
    assert!(marker.next().await.unwrap().is_some(), "the phone announced the marker");
    assert_eq!(mac.pending_tombstone_devices().unwrap(), Some(0), "the updated phone is counted as current");

    // The phone applies it on its next pull and reports the meeting's files to the app.
    sync_engine::take_forgotten_meeting_files();
    pull().await.unwrap();
    assert!(!has_meeting_titled(&phone, &format!("{NAME} kickoff")), "the meeting is gone on the phone");
    assert_eq!(calendar_events(&phone), 0, "the event is gone on the phone");
    assert!(phone.list_seed_topics().unwrap().is_empty(), "the seed is purged on the phone");
    assert!(phone.topic_page_snapshot(&seed.key).unwrap().is_none(), "the client's page is gone on the phone");
    let files = sync_engine::take_forgotten_meeting_files();
    // One entry for the removed meeting, on the channel both apps drain. It names no paths: the phone's copy of
    // a Mac's meeting holds no audio, transcript file or slides (the Mac removed its own above).
    assert_eq!(files.len(), 1, "the removed meeting is reported to the app: {files:?}");
    let f = &files[0];
    assert!(f.audio_path.is_none() && f.audio_paths.is_empty() && f.transcript_paths.is_empty() && f.snapshots_dir.is_none(), "{f:?}");
}
