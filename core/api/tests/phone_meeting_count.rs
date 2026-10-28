//! M4.0 exit gate, without Google or a real Turso: two Macs push their meetings to a
//! relay (a local libsql file, as sync-engine's own tests do), one Mac pulls the other's,
//! and a fresh phone store does the pull-only round the iPhone runs (30-day capture
//! window). The phone's grouped meeting count (what it shows) must equal the Mac's
//! (what its Meetings list shows: `whimbrel_meetings_recent_grouped(200)`).

use api::meetings::copies::group_copies;
use memory::{MeetingStatus, MeetingTrigger, NewMeeting, Store};
use sync::GroupKey;

const MIN: i64 = 60_000;
const DAY: i64 = 24 * 60 * MIN;
/// The Mac's Meetings list limit (MeetingsListView.refresh); the phone uses the same.
const LIST_LIMIT: usize = 200;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn record(store: &Store, device: &str, title: &str, start: i64, minutes: i64) {
    let id = store
        .insert_meeting(&NewMeeting {
            device_id: device.to_string(),
            start_unix_ms: start,
            end_unix_ms: None,
            title: title.to_string(),
            attendees: vec![],
            trigger: MeetingTrigger::Calendar,
            audio_path: None,
            status: MeetingStatus::Recording,
        })
        .unwrap();
    store
        .complete_meeting(id, start + minutes * MIN, None)
        .unwrap();
}

fn grouped_count(store: &Store) -> usize {
    group_copies(store.recent_meetings(LIST_LIMIT).unwrap()).len()
}

/// One Mac content-sync round (`sync_engine::sync_content` after connecting).
async fn mac_round(conn: &libsql::Connection, key: &GroupKey, store: &Store, device: &str) {
    sync_engine::ensure_content_schema(conn).await.unwrap();
    sync_engine::push_content(conn, key, store, device)
        .await
        .unwrap();
    sync_engine::pull_content(conn, key, store, device)
        .await
        .unwrap();
    sync_engine::dispatch_outbox(conn, key, store, device)
        .await
        .unwrap();
}

#[tokio::test]
async fn phone_pull_only_shows_the_same_grouped_meeting_count_as_the_mac() {
    let dir = tempfile::tempdir().unwrap();
    let relay = libsql::Builder::new_local(dir.path().join("relay.db"))
        .build()
        .await
        .unwrap();
    let conn = relay.connect().unwrap();
    let key = GroupKey::generate();
    let now = now_ms();

    let mac_a = Store::open_in_memory().unwrap();
    mac_a.set_own_device_id("mac-a");
    record(&mac_a, "mac-a", "Standup", now - 2 * DAY, 15);
    record(&mac_a, "mac-a", "Design review", now - DAY, 45);
    // Older than the phone's 30-day capture window: meetings are never windowed.
    record(&mac_a, "mac-a", "Quarterly planning", now - 60 * DAY, 60);
    // The same call recorded on a second Mac in the room: one meeting in both lists.
    let one_on_one = now - 3 * DAY;
    record(&mac_a, "mac-a", "1:1", one_on_one, 30);
    let mac_b = Store::open_in_memory().unwrap();
    mac_b.set_own_device_id("mac-b");
    record(&mac_b, "mac-b", "1:1", one_on_one + MIN, 29);

    // A call still recording when it first syncs, completed before a later round.
    let live = mac_a
        .insert_meeting(&NewMeeting {
            device_id: "mac-a".to_string(),
            start_unix_ms: now - 30 * MIN,
            end_unix_ms: None,
            title: "Live call".to_string(),
            attendees: vec![],
            trigger: MeetingTrigger::Detected,
            audio_path: None,
            status: MeetingStatus::Recording,
        })
        .unwrap();

    mac_round(&conn, &key, &mac_a, "mac-a").await;
    mac_round(&conn, &key, &mac_b, "mac-b").await;
    mac_round(&conn, &key, &mac_a, "mac-a").await;

    // The phone: a fresh on-disk store opened the way whimbrel_memory_open(path, NULL) does.
    let phone = api::store::open(&dir.path().join("phone.db"), std::path::Path::new("")).unwrap();
    phone.set_own_device_id("phone");
    let options = sync_engine::PullOptions {
        capture_window_ms: Some(30 * DAY),
    };
    sync_engine::pull_only_with_connection(&conn, &key, &phone, "phone", &options)
        .await
        .unwrap();
    // By design a meeting syncs only once it stops recording (push_content and the
    // outbox hold it back), so while the Mac records, its list has one meeting more.
    assert_eq!(
        grouped_count(&phone),
        grouped_count(&mac_a) - 1,
        "while the live call records"
    );

    mac_a.complete_meeting(live, now - MIN, None).unwrap();
    mac_round(&conn, &key, &mac_a, "mac-a").await;
    sync_engine::pull_only_with_connection(&conn, &key, &phone, "phone", &options)
        .await
        .unwrap();

    assert_eq!(
        mac_a.recent_meetings(LIST_LIMIT).unwrap().len(),
        6,
        "Mac A holds both copies"
    );
    assert_eq!(
        phone.recent_meetings(LIST_LIMIT).unwrap().len(),
        6,
        "the phone pulled every row, once"
    );
    assert_eq!(
        grouped_count(&mac_a),
        5,
        "the two 1:1 copies show as one meeting"
    );
    assert_eq!(grouped_count(&phone), grouped_count(&mac_a));
    assert!(
        phone
            .recent_meetings(LIST_LIMIT)
            .unwrap()
            .iter()
            .any(|m| m.title == "Live call" && m.status == MeetingStatus::Completed),
        "the completion reached the phone as an update, not a second row"
    );

    // Another round (the 60 s cadence) is idempotent: no duplicates.
    sync_engine::pull_only_with_connection(&conn, &key, &phone, "phone", &options)
        .await
        .unwrap();
    assert_eq!(phone.recent_meetings(LIST_LIMIT).unwrap().len(), 6);
    assert_eq!(grouped_count(&phone), 5);
}
