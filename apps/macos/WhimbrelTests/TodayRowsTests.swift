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
        content.nextPrep = CoreMeetingPrep(eventId: next.id, title: "x", startUnixMs: 0, generatedAtUnixMs: 0, briefJson: "{}", provider: "p", model: "m")
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
}
