import XCTest
@testable import Whimbrel

/// The row and recap decisions that Terminal and Glass share: which rows, in what order, with
/// which action enabled.
final class TodayRowsTests: XCTestCase {
    private var sample: TodayContent { TodayFixtures.sample }

    private func run(_ summary: String, origin: String? = nil) -> CoreRoutineRunWithName {
        CoreRoutineRunWithName(id: 1, routineId: 1, routineName: "Daily Brief", ranAtUnixMs: 0, status: "ok", outputSummary: summary, outputJson: "{}", originDeviceId: origin)
    }

    func test_up_next_is_the_first_upcoming_event_and_is_excluded_from_the_schedule_once() throws {
        let content = sample
        let next = try XCTUnwrap(TodayRows.upNext(content))
        XCTAssertEqual(next.title, "Design review")
        XCTAssertTrue(next.isNext)
        let placed = TodayRows.schedule(content, upNextPlaced: true)
        XCTAssertEqual(placed.upcoming.map(\.title), ["Pipeline sync", "1:1 with Sam"])
        XCTAssertFalse(placed.upcoming.map(\.id).contains(next.id))
        XCTAssertFalse(placed.upcoming.contains(where: \.isNext))
        let all = TodayRows.schedule(content, upNextPlaced: false)
        XCTAssertEqual(all.upcoming.map(\.title), ["Design review", "Pipeline sync", "1:1 with Sam"])
    }

    func test_schedule_is_in_ascending_time_order() {
        let held = TodayRows.schedule(sample, upNextPlaced: true).held
        XCTAssertEqual(held.map(\.title), ["Standup", "Customer call: Acme"])
        let worst = TodayRows.schedule(.worstCase, upNextPlaced: true)
        let ms = worst.held.map(\.startUnixMs)
        XCTAssertEqual(ms, ms.sorted())
        XCTAssertFalse(worst.held.isEmpty)
        let starts = worst.upcoming.map(\.start)
        XCTAssertEqual(starts, starts.sorted())
    }

    /// Terminal, Glass and the leftovers all iterate this sequence, so the label is between the lists in each.
    func test_schedule_items_put_the_earlier_today_label_between_upcoming_and_held() {
        let items = TodayRows.scheduleItems(TodayRows.schedule(sample, upNextPlaced: true))
        let shape = items.map { item -> String in
            switch item {
            case let .event(e): return e.title
            case .separator: return "--"
            case let .held(m): return m.title
            }
        }
        XCTAssertEqual(shape, ["Pipeline sync", "1:1 with Sam", "--", "Standup", "Customer call: Acme"])
        XCTAssertEqual(TodayRows.heldSeparator, "Earlier today")
        XCTAssertEqual(Set(items.map(\.id)).count, items.count)
        // No label when either side is empty.
        var onlyHeld = sample
        onlyHeld.events = []
        XCTAssertFalse(TodayRows.scheduleItems(TodayRows.schedule(onlyHeld, upNextPlaced: false)).contains { if case .separator = $0 { return true } else { return false } })
    }

    func test_start_only_when_no_meeting_is_active() throws {
        var content = sample
        XCTAssertTrue(try XCTUnwrap(TodayRows.upNext(content)).showsStart)
        XCTAssertTrue(TodayRows.schedule(content, upNextPlaced: true).upcoming.allSatisfy(\.showsStart))
        content.activeMeetingId = 7
        XCTAssertFalse(try XCTUnwrap(TodayRows.upNext(content)).showsStart)
        XCTAssertTrue(TodayRows.schedule(content, upNextPlaced: true).upcoming.allSatisfy { !$0.showsStart })
    }

    func test_event_row_text_and_prep_state() throws {
        let next = try XCTUnwrap(TodayRows.upNext(sample))
        XCTAssertEqual(next.timeText, MeetingRowText.time(next.event.start))
        XCTAssertEqual(next.attendees, "Priya Nair, Tom Becker")
        XCTAssertEqual(next.prep, .missing, "no prep data: 'No prep yet'")
        var content = sample
        content.nextPrep = CoreMeetingPrep(
            eventId: TodayLogic.prepKey(eventId: next.id, start: next.start), title: "x",
            startUnixMs: Int64(next.start.timeIntervalSince1970 * 1000), generatedAtUnixMs: 0, briefJson: "{}", provider: "p", model: "m"
        )
        XCTAssertEqual(try XCTUnwrap(TodayRows.upNext(content)).prep, .missing, "a prep without a decodable brief is not ready")
        let later = TodayRows.schedule(sample, upNextPlaced: true).upcoming[0]
        XCTAssertNil(later.prep, "only the next event carries prep state")
    }

    func test_meeting_row_shows_transcribing_for_the_processing_meeting() throws {
        var content = sample
        content.processingMeetingId = 1
        let held = TodayRows.schedule(content, upNextPlaced: true).held
        XCTAssertEqual(held.first(where: { $0.id == 1 })?.subtitle, "Transcribing…")
        XCTAssertNotEqual(held.first(where: { $0.id == 2 })?.subtitle, "Transcribing…")
    }

    func test_earlier_rows_use_the_same_model() {
        XCTAssertEqual(TodayRows.earlier(sample).count, sample.agenda.earlier.count)
    }

    func test_recap_fallback_order_brief_then_digest_then_error() {
        var content = sample
        content.briefRecap = run("brief text", origin: "phone-1")
        XCTAssertEqual(TodayRows.recap(content), .brief(text: "brief text", origin: "iPhone"))
        content.briefRecap = run("brief text")
        XCTAssertEqual(TodayRows.recap(content), .brief(text: "brief text", origin: nil))
        content.briefRecap = nil
        XCTAssertEqual(TodayRows.recap(content), .digest(text: content.digest!.summary))
        content.digest = nil
        content.digestError = "boom"
        XCTAssertEqual(TodayRows.recap(content), .failed(message: "boom", retryEnabled: true))
        content.isGeneratingDigest = true
        XCTAssertEqual(TodayRows.recap(content), .failed(message: "boom", retryEnabled: false), "Try again is disabled while generating")
        content.digestError = nil
        XCTAssertEqual(TodayRows.recap(content), .none)
    }

    /// The Today timeline is one start-ordered list of cards; the Now marker sits where past ends.
    func testScheduleItemsBuildCardModelsInStartOrder() throws {
        var content = sample
        let entries = TodayRows.timeline(content, now: content.now)
        XCTAssertEqual(entries.map(\.model.title), ["Standup", "Customer call: Acme", "Design review", "Pipeline sync", "1:1 with Sam"])
        XCTAssertEqual(entries.map(\.start), entries.map(\.start).sorted())
        XCTAssertEqual(entries.map(\.model.isPast), [true, true, false, false, false])
        XCTAssertEqual(Set(entries.map(\.id)).count, entries.count)
        XCTAssertEqual(TodayRows.timelineNowIndex(entries, now: content.now), 2)
        XCTAssertEqual(entries[0].model.target, .meeting(id: 1))
        XCTAssertEqual(entries[2].model.target, .none, "no brief and no topic: not clickable")
        XCTAssertEqual(entries.filter(\.isNext).map(\.model.title), ["Design review"])

        // A topic on an event makes it open the topic; a brief wins over the topic.
        content.topicsByEvent["e2"] = TopicRef(topicId: 7, key: "pipeline", name: "Pipeline")
        let withTopic = TodayRows.timeline(content, now: content.now)
        XCTAssertEqual(withTopic[3].model.target, .topic(id: 7))
        XCTAssertEqual(withTopic[3].model.chips, [.topic(id: 7, name: "Pipeline")])

        // Edges: every meeting upcoming puts the marker first, every meeting past puts it last.
        XCTAssertEqual(TodayRows.timelineNowIndex(entries, now: entries[0].start.addingTimeInterval(-60)), 0)
        XCTAssertEqual(TodayRows.timelineNowIndex(entries, now: entries[4].start.addingTimeInterval(60)), entries.count)
    }

    /// A meeting that has started but not ended sits after the Now marker, drawn as running; only the next one carries a hint.
    func testRunningMeetingSitsAfterTheNowMarker() throws {
        var content = sample
        content.events.append(TodaySampleContent.event("run", "Running sync", TodaySampleContent.at(hour: 10, minute: 15), minutes: 45))
        let entries = TodayRows.timeline(content, now: content.now)
        XCTAssertEqual(entries.map(\.model.title), ["Standup", "Customer call: Acme", "Running sync", "Design review", "Pipeline sync", "1:1 with Sam"])
        let marker = TodayRows.timelineNowIndex(entries, now: content.now)
        XCTAssertEqual(marker, 2)
        XCTAssertEqual(entries[marker].model.title, "Running sync")
        XCTAssertTrue(entries[marker].model.isRunning)
        XCTAssertEqual(entries.map(\.model.isRunning), [false, false, true, false, false, false])
        XCTAssertEqual(entries.map(\.model.isPast), [true, true, false, false, false, false])
        XCTAssertEqual(entries.filter(\.isNext).map(\.model.title), ["Running sync"])
        XCTAssertNil(entries[marker].model.hint, "a running meeting needs no 'in 10 min'")
    }

    func testOnlyTheNextUpcomingCardHasARelativeHint() throws {
        let entries = TodayRows.timeline(sample, now: sample.now)
        XCTAssertEqual(entries.map { $0.model.hint != nil }, [false, false, true, false, false])
        XCTAssertEqual(entries[2].model.hint, String(localized: "In \(30)m"))
    }

    /// A held meeting gets the same Prep chip on Today that the Meetings list gives it.
    func testHeldMeetingCardGetsItsPrepChip() throws {
        var content = sample
        let before = TodayRows.timeline(content, now: content.now)
        XCTAssertFalse(before[0].model.chips.contains(.prep(key: "e-standup@1")))
        content.prepKeyByMeetingId = [1: "e-standup@1"]
        let entries = TodayRows.timeline(content, now: content.now)
        XCTAssertEqual(entries[0].model.target, .meeting(id: 1))
        XCTAssertEqual(entries[0].model.chips.first, .prep(key: "e-standup@1"))
        XCTAssertFalse(entries[1].model.chips.contains(.prep(key: "e-standup@1")))
    }

    func testNextMeetingHintShowsWhileAnotherIsRunning() throws {
        var content = sample
        content.events.append(TodaySampleContent.event("run", "Running sync", TodaySampleContent.at(hour: 10, minute: 15), minutes: 45))
        let entries = TodayRows.timeline(content, now: content.now)
        XCTAssertEqual(entries.map(\.model.title), ["Standup", "Customer call: Acme", "Running sync", "Design review", "Pipeline sync", "1:1 with Sam"])
        XCTAssertEqual(entries.map { $0.model.hint != nil }, [false, false, false, true, false, false])
    }
}
