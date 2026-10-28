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
}
#endif
