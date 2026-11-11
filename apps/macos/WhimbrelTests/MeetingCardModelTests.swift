import XCTest
@testable import Whimbrel

/// The one model every meeting card and row renders from: its click target, chips, people count
/// and duration, plus the Now marker position and the localized day headings.
final class MeetingCardModelTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)
    private var now: Date { start.addingTimeInterval(-3600) }
    private let topic = TopicRef(topicId: 7, key: "acme", name: "Acme")

    private func event(attendees: [String] = ["Ana", "Ben"]) -> QualifyingMeeting {
        QualifyingMeeting(id: "EVT-1", title: "Planning", startDate: start, endDate: start.addingTimeInterval(45 * 60),
                          attendees: attendees, conferenceLink: nil, source: .calendar, appBundleID: nil, organizerEmail: nil,
                          organizerName: nil, selfIsOrganizer: false, selfResponse: nil)
    }

    private func recorded(endMs: Int64?, tldr: String? = nil, actions: [String] = []) -> CoreMeeting {
        let startMs = Int64(start.timeIntervalSince1970 * 1000)
        return CoreMeeting(
            id: 42, deviceId: "d1", startUnixMs: startMs, endUnixMs: endMs, title: "Standup", attendees: ["a@x.com"],
            trigger: .manual, audioPath: nil, status: .completed, transcriptPath: nil, notes: nil, summaryTldr: tldr,
            summaryDecisions: [], summaryActionItems: actions, summaryOpenQuestions: [], summaryProvider: nil,
            summaryModel: nil, estimatedSpeakerCount: nil
        )
    }

    func testEventWithPrepTargetsBrief() {
        let e = event()
        let key = TodayLogic.prepKey(eventId: e.id, start: e.startDate)
        let model = MeetingCardModel.forEvent(e, prepKey: key, topic: topic, now: now)
        XCTAssertEqual(model.target, .prepBrief(key: key))
        XCTAssertEqual(model.chips, [.prep(key: key), .topic(id: 7, name: "Acme")])
        XCTAssertFalse(model.isPast)
    }

    func testEventWithTopicOnlyTargetsTopic() {
        let model = MeetingCardModel.forEvent(event(), prepKey: nil, topic: topic, now: now)
        XCTAssertEqual(model.target, .topic(id: 7))
        XCTAssertEqual(model.chips, [.topic(id: 7, name: "Acme")])
    }

    func testUpcomingWithoutPrepOrTopicIsNotClickable() {
        let model = MeetingCardModel.forEvent(event(), prepKey: nil, topic: nil, now: now)
        XCTAssertEqual(model.target, .none)
        XCTAssertTrue(model.chips.isEmpty)
    }

    func testRecordedTargetsMeetingWithSummaryAndActionChips() {
        let endMs = Int64(start.addingTimeInterval(1800).timeIntervalSince1970 * 1000)
        let model = MeetingCardModel.forRecorded(recorded(endMs: endMs, tldr: "We shipped.", actions: ["a", "b"]),
                                                 prepKey: nil, topic: topic, now: now)
        XCTAssertEqual(model.target, .meeting(id: 42))
        XCTAssertEqual(model.chips, [.summary, .actionItems(count: 2), .topic(id: 7, name: "Acme")])
        XCTAssertEqual(model.durationMinutes, 30)
        let bare = MeetingCardModel.forRecorded(recorded(endMs: endMs, tldr: ""), prepKey: nil, topic: nil, now: now)
        XCTAssertTrue(bare.chips.isEmpty)
        XCTAssertEqual(bare.target, .meeting(id: 42))
    }

    private func prepRow(eventId: String) -> CoreMeetingPrep {
        CoreMeetingPrep(eventId: eventId, title: "x", startUnixMs: 1_700_000_000_000, generatedAtUnixMs: 0, briefJson: "{}", provider: "p", model: "m")
    }

    func testPrepKeyIsTheStoredEventId() {
        XCTAssertEqual(MeetingsListView.prepKey(for: prepRow(eventId: "evt@1700000000000")), "evt@1700000000000")
        XCTAssertEqual(MeetingsListView.prepKey(for: prepRow(eventId: "evt")), "evt")
    }

    func testChipsFromBatchLookups() {
        let withData = recorded(endMs: nil)
        let other = CoreMeeting(
            id: 43, deviceId: "d1", startUnixMs: withData.startUnixMs, endUnixMs: nil, title: "Solo", attendees: [],
            trigger: .manual, audioPath: nil, status: .completed, transcriptPath: nil, notes: nil, summaryTldr: nil,
            summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil,
            summaryModel: nil, estimatedSpeakerCount: nil
        )
        let prepKey = MeetingsListView.prepKey(for: prepRow(eventId: "EVT-1@1700000000000"))
        let models = MeetingsListView.rowModels(meetings: [withData, other], topics: [42: topic], preps: [42: prepKey], now: now)
        XCTAssertEqual(models.map(\.title), ["Standup", "Solo"])
        XCTAssertEqual(models[0].chips, [.prep(key: "EVT-1@1700000000000"), .topic(id: 7, name: "Acme")])
        XCTAssertTrue(models[1].chips.isEmpty)
        XCTAssertEqual(models[1].target, .meeting(id: 43))
        let early = MeetingsListView.rowModels(meetings: [withData], topics: [:], preps: [:], now: now)
        XCTAssertTrue(early[0].chips.isEmpty)
    }

    func testPeopleCountIncludesYou() {
        XCTAssertEqual(MeetingCardModel.forEvent(event(attendees: ["Ana", "Ben"]), prepKey: nil, topic: nil, now: now).peopleCount, 3)
        XCTAssertEqual(MeetingCardModel.forEvent(event(attendees: []), prepKey: nil, topic: nil, now: now).peopleCount, 1)
    }

    func testNoEndTimeHasNoDuration() {
        let model = MeetingCardModel.forRecorded(recorded(endMs: nil), prepKey: nil, topic: nil, now: now)
        XCTAssertNil(model.durationMinutes)
        XCTAssertEqual(model.timeRange, MeetingRowText.time(start))
        XCTAssertEqual(MeetingCardModel.forEvent(event(), prepKey: nil, topic: nil, now: now).durationMinutes, 45)
    }

    func testNowMarkerPositionAtEdges() {
        let now = self.now
        let ago: (Double) -> Date = { now.addingTimeInterval(-$0) }
        XCTAssertEqual(MeetingCardModel.nowMarkerIndex(ends: [], now: now), 0)
        XCTAssertEqual(MeetingCardModel.nowMarkerIndex(ends: [ago(60), ago(30)], now: now), 2, "everything ended: marker last")
        XCTAssertEqual(MeetingCardModel.nowMarkerIndex(ends: [ago(-60), ago(-120)], now: now), 0, "everything ahead: marker first")
        XCTAssertEqual(MeetingCardModel.nowMarkerIndex(ends: [now, ago(-60)], now: now), 1, "ending exactly now has ended")
        XCTAssertEqual(MeetingCardModel.nowMarkerIndex(ends: [ago(60), ago(-60)], now: now), 1)
        XCTAssertEqual(MeetingCardModel.nowMarkerIndex(ends: [ago(60), ago(-60), ago(30)], now: now), 1, "counts leading ended items only")
    }

    func testRunningMeetingHasInProgressState() {
        let e = event()
        let during = start.addingTimeInterval(10 * 60)
        let running = MeetingCardModel.forEvent(e, prepKey: nil, topic: nil, now: during)
        XCTAssertTrue(running.isRunning)
        XCTAssertFalse(running.isPast)
        let before = MeetingCardModel.forEvent(e, prepKey: nil, topic: nil, now: now)
        XCTAssertFalse(before.isRunning)
        XCTAssertFalse(before.isPast)
        let after = MeetingCardModel.forEvent(e, prepKey: nil, topic: nil, now: start.addingTimeInterval(45 * 60))
        XCTAssertFalse(after.isRunning)
        XCTAssertTrue(after.isPast)
        XCTAssertFalse(MeetingCardModel.forRecorded(recorded(endMs: nil), prepKey: nil, topic: nil, now: during).isRunning)
    }

    func testDayHeadingsAreLocalized() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/New_York")!
        let nowDate = calendar.date(from: DateComponents(year: 2026, month: 10, day: 6, hour: 12))!
        let ms: (Int) -> Int64 = { day in
            Int64(calendar.date(from: DateComponents(year: 2026, month: 10, day: day, hour: 9))!.timeIntervalSince1970 * 1000)
        }
        let groups = MeetingDayGrouping.groups([(id: 1, startUnixMs: ms(6)), (id: 2, startUnixMs: ms(5))],
                                               now: nowDate, calendar: calendar, locale: Locale(identifier: "en_US"))
        // Pins the catalog keys, not the translations: the Today tab's "Today" is a product name
        // (shouldTranslate false), so the day heading must use its own key.
        XCTAssertEqual(groups.map(\.title), [
            String(localized: "day.today", defaultValue: "Today", comment: "Day heading in the Meetings list: the current day"),
            String(localized: "day.yesterday", defaultValue: "Yesterday", comment: "Day heading in the Meetings list: the previous day"),
        ])
    }

    func testEndBeforeStartShowsStartOnly() {
        let startOnly = MeetingRowText.time(start)
        XCTAssertEqual(MeetingCardModel.timeRange(start: start, end: start.addingTimeInterval(-900)), startOnly)
        XCTAssertEqual(MeetingCardModel.timeRange(start: start, end: start), startOnly)
        let endMs = Int64(start.addingTimeInterval(-900).timeIntervalSince1970 * 1000)
        let model = MeetingCardModel.forRecorded(recorded(endMs: endMs), prepKey: nil, topic: nil, now: now)
        XCTAssertEqual(model.timeRange, startOnly)
        XCTAssertNil(model.durationMinutes)
    }

    func testRecordedChipOrderIsPrepSummaryActionsTopic() {
        let endMs = Int64(start.addingTimeInterval(1800).timeIntervalSince1970 * 1000)
        let model = MeetingCardModel.forRecorded(recorded(endMs: endMs, tldr: "x", actions: ["a"]), prepKey: "k@1", topic: topic, now: now)
        XCTAssertEqual(model.chips, [.prep(key: "k@1"), .summary, .actionItems(count: 1), .topic(id: 7, name: "Acme")])
    }

    func testAwayOutcomeShowsOnRecordedCard() {
        var away = recorded(endMs: nil)
        away = CoreMeeting(
            id: 42, deviceId: "d1", startUnixMs: away.startUnixMs, endUnixMs: nil, title: "Standup", attendees: [],
            trigger: .away, audioPath: nil, status: .completed, transcriptPath: nil, notes: nil, summaryTldr: nil,
            summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil,
            summaryModel: nil, estimatedSpeakerCount: nil, awayOutcome: .waitingRoom
        )
        let model = MeetingCardModel.forRecorded(away, prepKey: nil, topic: nil, now: now)
        XCTAssertEqual(model.statusLine, AwayOutcomeLabel.text(.waitingRoom, at: nil))
        // An away meeting with no stored outcome was attended.
        let attended = CoreMeeting(
            id: 43, deviceId: "d1", startUnixMs: away.startUnixMs, endUnixMs: nil, title: "x", attendees: [],
            trigger: .away, audioPath: nil, status: .completed, transcriptPath: nil, notes: nil, summaryTldr: nil,
            summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil,
            summaryModel: nil, estimatedSpeakerCount: nil
        )
        XCTAssertEqual(MeetingCardModel.forRecorded(attended, prepKey: nil, topic: nil, now: now).statusLine,
                       AwayOutcomeLabel.text(.attended, at: nil))
        XCTAssertNil(MeetingCardModel.forRecorded(recorded(endMs: nil), prepKey: nil, topic: nil, now: now).statusLine)
    }

    func testCopiesNoteShowsOnRecordedCard() {
        var m = recorded(endMs: nil)
        m.alsoRecorded = [CoreMeetingCopy(id: 50, deviceId: "d2", title: "Standup"), CoreMeetingCopy(id: 51, deviceId: "d3", title: "Standup")]
        XCTAssertEqual(MeetingCardModel.forRecorded(m, prepKey: nil, topic: nil, now: now).statusLine, String(localized: "+\(2) copies"))
        m.alsoRecorded = []
        XCTAssertNil(MeetingCardModel.forRecorded(m, prepKey: nil, topic: nil, now: now).statusLine)
    }

    func testChipBatchSkipsWhenIdsUnchanged() {
        XCTAssertTrue(MeetingsListView.needsChipBatch(ids: [1, 2, 3], lastBatched: nil, trigger: .appeared), "first load always batches")
        XCTAssertFalse(MeetingsListView.needsChipBatch(ids: [1, 2, 3], lastBatched: [1, 2, 3], trigger: .appeared))
        XCTAssertFalse(MeetingsListView.needsChipBatch(ids: [3, 1, 2], lastBatched: [1, 2, 3], trigger: .appeared), "order does not matter")
        XCTAssertTrue(MeetingsListView.needsChipBatch(ids: [1, 2, 3, 4], lastBatched: [1, 2, 3], trigger: .appeared), "a new meeting batches")
        XCTAssertTrue(MeetingsListView.needsChipBatch(ids: [1, 2], lastBatched: [1, 2, 3], trigger: .appeared), "a deleted meeting batches")
        XCTAssertTrue(MeetingsListView.needsChipBatch(ids: [], lastBatched: nil, trigger: .appeared))
        XCTAssertFalse(MeetingsListView.needsChipBatch(ids: [], lastBatched: [], trigger: .appeared))
    }

    func testChipBatchRerunsAfterMeetingsChange() {
        XCTAssertTrue(MeetingsListView.needsChipBatch(ids: [1, 2, 3], lastBatched: [1, 2, 3], trigger: .meetingsChanged),
                      "a processed meeting gains its topic with the same ids")
        XCTAssertTrue(MeetingsListView.needsChipBatch(ids: [1], lastBatched: nil, trigger: .meetingsChanged))
    }
}
