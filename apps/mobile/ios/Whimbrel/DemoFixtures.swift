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

    private static func isoDay(_ offsetDays: Int) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f.string(from: Date().addingTimeInterval(Double(offsetDays) * 86_400))
    }

    /// Three topic pages: a full one, a short one and one with only a name. The resolvable
    /// meeting ref sits on a step, not in `recent`, so Today and Meetings show no topic chips.
    private static func topics() -> [PhoneTopicSnapshot] {
        let day: Int64 = 86_400_000
        let call = PhoneMeetingRef(deviceId: "demo-mac-a", sourceMeetingId: 1)
        return [
            PhoneTopicSnapshot(
                topicKey: "acme", name: "Acme renewal", kind: "customer",
                lead: .date(label: "Quote due", date: isoDay(3)),
                role: .init(text: "You own the quote and the discount decision.", edited: true),
                nextMeeting: .init(title: "Customer call: Acme pricing", startUnixMs: now + 2 * day,
                                   attendeesByOrg: [.init(domain: "acme.com", count: 3)], me: 1),
                youOwe: [
                    .init(text: "Send the revised tier table to Priya", sourceKind: "meeting", sourceTitle: "Customer call: Acme pricing",
                          askedAtUnixMs: now - 2 * day, dueDate: isoDay(-1), meeting: call),
                    .init(text: "Confirm whether existing customers keep their price", sourceKind: "email", sourceTitle: "Re: renewal terms",
                          askedAtUnixMs: now - day, dueDate: isoDay(3)),
                ],
                owedToYou: [.init(text: "Priya to send the signed order form", sourceKind: "email", sourceTitle: "Order form", askedAtUnixMs: now - 3 * day)],
                done: [.init(text: "Share the pricing deck", sourceKind: "meeting", sourceTitle: "Design review")],
                recent: [
                    .init(kind: "email", title: "Re: renewal terms", sender: "Priya Nair", tsUnixMs: now - day),
                    .init(kind: "meeting", title: "Customer call: Acme pricing", tsUnixMs: now - 2 * day),
                    .init(kind: "document", title: "Pricing v3.xlsx", tsUnixMs: now - 4 * day),
                ],
                recentTotal: 9, truncated: true, builtAtUnixMs: now - 5 * minute, builtBy: "Demo MacBook"),
            PhoneTopicSnapshot(
                topicKey: "hiring", name: "Hiring: senior designer", kind: "project",
                lead: .step(text: "Send feedback on the two finalists", dueDate: isoDay(1)),
                youOwe: [.init(text: "Send feedback on the two finalists", sourceKind: "meeting", sourceTitle: "Hiring panel", dueDate: isoDay(1))],
                recent: [.init(kind: "meeting", title: "Hiring panel", tsUnixMs: now - 3 * day)],
                recentTotal: 1, builtAtUnixMs: now - 2 * 3_600_000, builtBy: "Demo MacBook"),
            PhoneTopicSnapshot(topicKey: "offsite", name: "Team offsite", builtAtUnixMs: now - day, builtBy: "Demo MacBook"),
        ]
    }

    /// `-WhimbrelDemoEmpty`: no topics and no hosted key, so the first-run states can be shown.
    static var isEmptyDemo: Bool { ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoEmpty") }

    func topicSnapshots() -> [PhoneTopicSnapshot] { Self.isEmptyDemo ? [] : Self.topics() }
    func topicSnapshot(key: String) -> PhoneTopicSnapshot? { topicSnapshots().first { $0.topicKey == key } }
    func meetingId(deviceId: String, sourceMeetingId: Int64) -> Int64? { deviceId == "demo-mac-a" && sourceMeetingId == 1 ? 1 : nil }
}

/// `-WhimbrelDemoSynced` Ask: nothing here reaches a provider. A typed question fails at
/// `prepare`, so no prompt is built and no stream starts.
struct DemoAskBackend: AskBackend {
    func prepare(query: String, historyJSON: String, routingJSON: String) async -> PrepareResult { .failed }
    func finalize(prepareId: String, text: String, model: String, inputTokens: Int64, outputTokens: Int64) async -> [CitationCard]? { nil }
    func discard(prepareId: String) {}
    func resolve(refs: [CitationRef]) async -> [CitationCard] { [] }
    func stream(prompt: String, model: String, provider: AskProvider, key: String) -> AsyncStream<InferenceEvent> { AsyncStream { $0.finish() } }

    @MainActor static func model() -> AskModel {
        let model = AskModel(backend: DemoAskBackend(), prefs: PhoneRoutingPrefs(defaults: UserDefaults(suiteName: "WhimbrelDemoAsk") ?? .standard),
                             keys: { $0 == .anthropic && !DemoSyncStore.isEmptyDemo ? "demo" : nil })
        if DemoSyncStore.isEmptyDemo { return model }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let line = "claude-sonnet · your key"
        model.seedDemo([
            AskTurn(question: "What did we decide about Acme pricing?",
                    answer: "You agreed to ship the pricing change on Friday and keep the annual discount at 15%.\n\nOne thing is still open: whether existing customers keep their current price. Sam owes the updated pricing page, and Ann is briefing support.",
                    state: .done,
                    citations: [
                        CitationCard(kind: "transcript", id: 1, title: "Customer call: Acme pricing", subtitle: "Meeting", tsUnixMs: now - 3_600_000, meetingId: 1, deviceId: "demo-mac-a", found: true),
                        CitationCard(kind: "capture", id: 7, title: "Pricing v3.xlsx", subtitle: "Screen", tsUnixMs: now - 2 * 86_400_000, meetingId: nil, deviceId: "demo-mac-a", found: true),
                        CitationCard(kind: "capture", id: 8, title: "", subtitle: "Screen", tsUnixMs: nil, meetingId: nil, deviceId: nil, found: false),
                    ], modelLine: line),
            AskTurn(question: "Who is briefing support?", answer: "Ann is briefing support before", state: .stopped, modelLine: line),
            AskTurn(question: "And the hiring panel?", state: .failed, modelLine: line),
        ])
        return model
    }
}
#endif
