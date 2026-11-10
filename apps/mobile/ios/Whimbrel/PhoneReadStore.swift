import Foundation

/// What the screens read from the phone's replica. Calls block; `SyncModel.read` runs them
/// on the store queue, never on the main thread.
protocol PhoneReadStore: AnyObject {
    /// Grouped meetings, newest first (copies of one call collapse into one entry).
    func recentMeetings(limit: Int) -> [PhoneMeeting]
    func meeting(id: Int64) -> PhoneMeeting?
    func transcriptPage(meetingId: Int64, offset: Int, limit: Int) -> PhoneTranscriptPage?
    func actionItems(meetingId: Int64) -> [PhoneActionItem]
    func routineRuns(limit: Int) -> [PhoneRoutineRun]
    func prep(eventId: String) -> PhoneMeetingPrep?
    func upcomingEvents(fromUnixMs: Int64, untilUnixMs: Int64, limit: Int) -> [PhoneCalendarEvent]
    /// Topic pages the computer built, in the store's order; deleted ones are left out.
    func topicSnapshots() -> [PhoneTopicSnapshot]
    /// One topic page; nil when it is absent or deleted.
    func topicSnapshot(key: String) -> PhoneTopicSnapshot?
    /// This replica's meeting id for a meeting recorded as `sourceMeetingId` on `deviceId`.
    func meetingId(deviceId: String, sourceMeetingId: Int64) -> Int64?
}

#if os(iOS)
extension CoreMemoryStore: PhoneReadStore {
    private func decode<T: Decodable>(_ json: UnsafeMutablePointer<CChar>?) -> T? {
        guard let json else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(T.self, from: Data(String(cString: json).utf8))
    }

    func recentMeetings(limit: Int) -> [PhoneMeeting] {
        decode(whimbrel_meetings_recent_grouped(handle, limit)) ?? []
    }

    func meeting(id: Int64) -> PhoneMeeting? {
        decode(whimbrel_memory_get_meeting(handle, id))
    }

    func transcriptPage(meetingId: Int64, offset: Int, limit: Int) -> PhoneTranscriptPage? {
        decode(whimbrel_memory_get_transcript_segments_page(handle, meetingId, offset, limit))
    }

    func actionItems(meetingId: Int64) -> [PhoneActionItem] {
        decode(whimbrel_memory_list_meeting_action_items(handle, meetingId)) ?? []
    }

    func routineRuns(limit: Int) -> [PhoneRoutineRun] {
        decode(whimbrel_routines_recent_runs(handle, UInt64(limit))) ?? []
    }

    func prep(eventId: String) -> PhoneMeetingPrep? {
        decode(eventId.withCString { whimbrel_meeting_prep_get(handle, $0) })
    }

    func upcomingEvents(fromUnixMs: Int64, untilUnixMs: Int64, limit: Int) -> [PhoneCalendarEvent] {
        decode(whimbrel_memory_upcoming_calendar_events(handle, fromUnixMs, untilUnixMs, UInt64(limit))) ?? []
    }

    func topicSnapshots() -> [PhoneTopicSnapshot] {
        decode(whimbrel_topic_snapshots_json(handle)) ?? []
    }

    func topicSnapshot(key: String) -> PhoneTopicSnapshot? {
        // Core answers the JSON text `null` for a missing page, which fails to decode: nil.
        decode(key.withCString { whimbrel_topic_snapshot_json(handle, $0) })
    }

    func meetingId(deviceId: String, sourceMeetingId: Int64) -> Int64? {
        let id = deviceId.withCString { whimbrel_meeting_id_for_source(handle, $0, sourceMeetingId) }
        return id < 0 ? nil : id
    }
}
#endif
