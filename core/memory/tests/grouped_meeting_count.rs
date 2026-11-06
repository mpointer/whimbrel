use memory::{MeetingStatus, MeetingTrigger, NewMeeting, Store};

const MIN: i64 = 60_000;

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
    store.complete_meeting(id, start + minutes * MIN, None).unwrap();
}

#[test]
fn counts_every_meeting_not_just_the_newest_200_and_groups_copies() {
    let store = Store::open_in_memory().unwrap();
    // 250 distinct meetings, a day apart (so none overlap)...
    for i in 0..250 {
        record(&store, "mac-a", &format!("Meeting {i}"), i * 24 * 60 * MIN, 30);
    }
    // ...plus the oldest one recorded again on a second Mac: a copy, not a meeting.
    record(&store, "mac-b", "Meeting 0", MIN, 29);

    assert_eq!(store.recent_meetings(10_000).unwrap().len(), 251);
    assert_eq!(store.recent_meetings(200).unwrap().len(), 200, "the old cap");
    assert_eq!(store.grouped_meeting_count().unwrap(), 250);
}
