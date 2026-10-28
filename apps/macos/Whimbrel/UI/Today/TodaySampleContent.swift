import SwiftUI

/// Plain `TodayContent` values for the skin gallery previews and for tests (`TodayFixtures` builds on them).
/// They build `Core*` values directly: no database, no `AppState`. The clock is pinned so the output does not drift.
enum TodaySampleContent {
    /// 2026-10-20 10:30 in the current time zone.
    static let now = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 20, hour: 10, minute: 30))!

    static func at(dayOffset: Int = 0, hour: Int, minute: Int = 0) -> Date {
        let day = Calendar.current.date(byAdding: .day, value: dayOffset, to: Calendar.current.startOfDay(for: now))!
        return Calendar.current.date(bySettingHour: hour, minute: minute, second: 0, of: day)!
    }

    static func event(_ id: String, _ title: String, _ start: Date, minutes: Int = 30, attendees: [String] = []) -> QualifyingMeeting {
        QualifyingMeeting(
            id: id, title: title, startDate: start, endDate: start.addingTimeInterval(Double(minutes) * 60),
            attendees: attendees, conferenceLink: nil, source: .calendar, appBundleID: nil, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil
        )
    }

    static func meeting(_ id: Int64, _ title: String, _ start: Date, attendees: [String] = []) -> CoreMeeting {
        CoreMeeting(
            id: id, deviceId: "mac-1", startUnixMs: Int64(start.timeIntervalSince1970 * 1000),
            endUnixMs: Int64(start.addingTimeInterval(1800).timeIntervalSince1970 * 1000), title: title,
            attendees: attendees, trigger: .calendar, audioPath: nil, status: .completed, transcriptPath: nil,
            notes: nil, summaryTldr: nil, summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [],
            summaryProvider: nil, summaryModel: nil, estimatedSpeakerCount: nil
        )
    }

    static func thread(_ id: Int64, _ subject: String?) -> CoreThread {
        CoreThread(
            id: id, source: "gmail", externalThreadId: "t\(id)", accountEmail: "alex@example.com", subject: subject,
            messageCount: 4, lastActivityTsUnixMs: 0, lastMessageFrom: "sam@example.com", summary: nil,
            status: "open", summarizeStatus: "done"
        )
    }

    static func content(
        events: [QualifyingMeeting], meetings: [CoreMeeting], approvals: Int, threads: [CoreThread],
        digest: CoreDailyDigest?, calendarGranted: Bool = true, followups: [CoreMeetingFollowup] = []
    ) -> TodayContent {
        TodayContent(
            name: "Alex Rivera", now: now, pinnedNow: now, events: events, recentMeetings: meetings,
            calendarGranted: calendarGranted, activeMeetingId: nil, processingMeetingId: nil, nextPrep: nil,
            pendingApprovalCount: approvals, attentionThreads: threads, briefRecap: nil, digest: digest,
            digestError: nil, isGeneratingDigest: false, deviceId: "mac-1",
            deviceRows: [
                CoreDeviceActivity(deviceId: "mac-1", captures: 42, lastTsUnixMs: Int64(now.timeIntervalSince1970 * 1000)),
                CoreDeviceActivity(deviceId: "phone-1", captures: 7, lastTsUnixMs: 0),
            ],
            deviceLabels: ["mac-1": "This Mac", "phone-1": "iPhone"], followups: followups
        )
    }

    /// A realistic day: three events to come, two meetings already held, a few earlier ones.
    static let sample = content(
        events: [
            event("e1", "Design review", at(hour: 11), attendees: ["Priya Nair", "Tom Becker"]),
            event("e2", "Pipeline sync", at(hour: 13), minutes: 45, attendees: ["Dana Cole", "Lee Ortiz", "Max Wu"]),
            event("e3", "1:1 with Sam", at(hour: 15, minute: 30)),
        ],
        meetings: [
            meeting(1, "Standup", at(hour: 8, minute: 30), attendees: ["Priya Nair"]),
            meeting(2, "Customer call: Acme", at(hour: 9, minute: 15), attendees: ["Jo Park", "Rae Kim"]),
            meeting(3, "Roadmap planning", at(dayOffset: -1, hour: 14), attendees: ["Dana Cole"]),
            meeting(4, "Hiring debrief", at(dayOffset: -1, hour: 10)),
            meeting(5, "Weekly review", at(dayOffset: -3, hour: 16), attendees: ["Tom Becker"]),
        ],
        approvals: 3,
        threads: [thread(1, "Q4 budget questions"), thread(2, "Re: contract redlines"), thread(3, nil)],
        digest: CoreDailyDigest(
            summary: "Two meetings so far. The Acme call agreed on a pilot start date; design review is next.",
            openLoops: ["Send Acme the pilot plan"]
        ),
        followups: [CoreMeetingFollowup(
            id: 1, meetingId: 2, text: "Send Acme the pilot plan", routineRunId: nil,
            createdAtUnixMs: Int64(now.timeIntervalSince1970 * 1000), doneAtUnixMs: nil
        )]
    )

    /// Stands in for `TodayComposer`, which needs `AppState`: the same shell, static and not interactive. Computed, so it reads the
    /// skin in force each time (a static would keep the first skin's font for every later render).
    static var composer: AnyView { AnyView(
        Text("Ask Whimbrel about your meetings")
            .font(.whimbrelBody(.body))
            .foregroundStyle(Color.whimbrelTextSecondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Spacing.lg)
            .padding(.vertical, Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder, lineWidth: 1))
    ) }
}

extension TodayActions {
    static let noop = TodayActions(
        openMeeting: { _ in }, openThread: { _ in }, ask: { _ in }, openSettings: {}, generateDigest: {},
        reviewApprovals: {}, startMeeting: { _ in }, startNote: {}, completeFollowup: { _ in }
    )
}
