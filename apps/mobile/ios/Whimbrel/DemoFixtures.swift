#if DEBUG
import Foundation

/// `-WhimbrelDemoSynced` read data: every screen can be shown without real data or a server.
/// Times are relative to launch, so Today always has "this morning's" brief and a next meeting.
extension DemoSyncStore: PhoneReadStore {
    private static let minute: Int64 = 60_000
    private static var now: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
    /// Today's bounds in the device calendar, as Today reads them. "This morning's" brief and
    /// the next meeting stay inside them, so the demo works just after midnight and late at night.
    private static var startOfToday: Int64 { Int64(Calendar.current.startOfDay(for: Date()).timeIntervalSince1970 * 1000) }
    private static var endOfToday: Int64 {
        let start = Calendar.current.startOfDay(for: Date())
        let end = Calendar.current.date(byAdding: .day, value: 1, to: start) ?? start.addingTimeInterval(86_400)
        return Int64(end.timeIntervalSince1970 * 1000)
    }

    private static func meetings() -> [PhoneMeeting] {
        let day: Int64 = 86_400_000
        let titles = [
            "Customer call: Acme pricing", "Design review", "Standup", "Planning", "1:1 with Sam", "Retro",
            "Partner sync with a very long title that has to wrap across several lines on a small phone screen", "Roadmap",
            "Hiring panel", "Board prep", "Weekly metrics", "Onboarding",
        ]
        return titles.enumerated().map { i, title in
            let start = now - Int64(i) * 5 * 3_600_000 - Int64(i / 3) * day / 2
            return PhoneMeeting(
                id: Int64(i + 1), deviceId: i % 3 == 1 ? "demo-mac-b" : "demo-mac-a", startUnixMs: start, endUnixMs: start + 30 * minute,
                title: title, attendees: i % 2 == 0 ? ["Sam Rivera", "Ann Lee", "Priya Nair", "Tom Webb"] : ["Sam Rivera"],
                summaryTldr: i == 2 ? nil : "We agreed to ship the pricing change on Friday and revisit the discount tiers next week.",
                summaryDecisions: i == 2 ? [] : ["Ship pricing change Friday", "Keep the annual discount at 15%"],
                summaryActionItems: i == 2 ? [] : ["Sam to update the pricing page", "Ann to brief support"],
                summaryOpenQuestions: i == 2 ? [] : ["Do we grandfather existing customers?"],
                alsoRecorded: i == 4 ? [PhoneMeetingCopy(id: 99, deviceId: "demo-mac-b", title: title)] : nil
            )
        }
    }

    func recentMeetings(limit: Int) -> [PhoneMeeting] { Array(Self.meetings().prefix(limit)) }
    func meeting(id: Int64) -> PhoneMeeting? { Self.meetings().first { $0.id == id } }

    func transcriptPage(meetingId: Int64, offset: Int, limit: Int) -> PhoneTranscriptPage? {
        guard meetingId == 1 else { return PhoneTranscriptPage(segments: [], total: 0) }
        let all = (0..<150).map { i in
            PhoneTranscriptSegment(id: Int64(i), startCs: Int64(i) * 900, speakerLabel: i % 2 == 0 ? "Speaker 1" : "Me",
                                   text: "Line \(i + 1) of the call: we walked through the pricing options and the open questions.")
        }
        return PhoneTranscriptPage(segments: Array(all.dropFirst(offset).prefix(limit)), total: all.count)
    }

    func actionItems(meetingId: Int64) -> [PhoneActionItem] {
        guard meetingId != 3 else { return [] } // the meeting with no summary has none
        return [PhoneActionItem(id: 1, text: "Sam to update the pricing page", completed: true, orderIndex: 0),
         PhoneActionItem(id: 2, text: "Ann to brief support", completed: false, orderIndex: 1)]
    }

    func routineRuns(limit: Int) -> [PhoneRoutineRun] {
        let now = Self.now
        return [
            PhoneRoutineRun(id: 1, routineName: "Daily Brief", ranAtUnixMs: max(now - 30 * Self.minute, Self.startOfToday), status: "success",
                            outputSummary: "Today you have 3 meetings. The Acme pricing call at 10:30 is the one that matters: Sam owes the revised tiers, and support has not been briefed. Two follow-ups from yesterday are still open, including the contract redline Ann sent on Monday.",
                            originDeviceId: "demo-mac-a"),
            PhoneRoutineRun(id: 2, routineName: "Meeting Prep", ranAtUnixMs: now - 45 * Self.minute, status: "success", outputSummary: "Prepared 1 meeting.", originDeviceId: nil),
            PhoneRoutineRun(id: 3, routineName: "Inbox triage", ranAtUnixMs: now - 3 * 3_600_000, status: "failed", outputSummary: "Gmail permission was revoked. Reconnect Gmail on your Mac.", originDeviceId: "demo-mac-a"),
            PhoneRoutineRun(id: 4, routineName: "Daily Brief", ranAtUnixMs: now - 86_400_000, status: "success", outputSummary: "Yesterday's brief.", originDeviceId: "demo-mac-a"),
        ].prefix(limit).map { $0 }
    }

    func prep(eventId: String) -> PhoneMeetingPrep? {
        guard eventId == "demo-next" else { return nil }
        let brief = #"{"summary":{"text":"Acme wants a quote for 200 seats; last call ended on discount tiers.","sources":[]},"attendees":[{"who":"Priya Nair","note":"Procurement lead, cares about annual terms","sources":[]}],"open_items":[{"text":"Send revised tier table","sources":[]}],"open_questions":[{"text":"Grandfather existing customers?","sources":[]}],"sources":[]}"#
        let json = try? JSONSerialization.data(withJSONObject: ["event_id": eventId, "title": "Customer call: Acme pricing", "start_unix_ms": Self.now, "brief_json": brief])
        return json.flatMap { try? JSONDecoder().decode(PhoneMeetingPrep.self, from: $0) }
    }

    func upcomingEvents(fromUnixMs: Int64, untilUnixMs: Int64, limit: Int) -> [PhoneCalendarEvent] {
        // 40 minutes out, or halfway to midnight when that is sooner.
        let start = Self.now + min(40 * Self.minute, (Self.endOfToday - Self.now) / 2)
        guard start >= fromUnixMs, start < untilUnixMs else { return [] }
        return [PhoneCalendarEvent(externalId: "demo-next", source: "google_calendar", title: "Customer call: Acme pricing", startUnixMs: start, attendees: ["Priya Nair", "Sam Rivera", "Ann Lee"])]
    }
}
#endif
