import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the redesign Home adapter (`UI/Redesign/HomeRedesign.swift`): `TodayContent` to the
/// `WHomeModel`, memory blocks, the capture state lines and the search filter. Uses the Today sample content
/// (made-up, a pinned clock). Nothing here touches the app's defaults, the store or a window.
final class RedesignHomeAdapterTests: XCTestCase {
    private let content = TodaySampleContent.sample
    private let now = TodaySampleContent.now

    private func model(memory: WMemoryState = .on, data: HomeRedesignData = HomeRedesignData()) -> WHomeModel {
        HomeRedesignAdapter.model(
            content: content, data: data, memory: memory, elapsedSeconds: nil,
            now: now, voice: SkinDefaults.whimbrel.voice, elements: WHomeElements())
    }

    func testHeroIsTheNextEventAndIsNotRepeatedInTheTimeLine() {
        let m = model()
        guard case let .next(hero) = m.hero else { return XCTFail("expected the next meeting") }
        XCTAssertEqual(hero.id, HomeRedesignAdapter.eventId("e1"))
        XCTAssertFalse(m.timelineMeetings.contains { $0.id == hero.id })
        XCTAssertTrue(m.timelineMeetings.contains { $0.id == HomeRedesignAdapter.eventId("e2") })
    }

    func testLineIsNewestFirstWithNowBetweenLaterAndEarlier() {
        let ids = model().timeline.map(\.id)
        let nowIndex = try! XCTUnwrap(ids.firstIndex(of: "now"))
        // Later meetings sit above the now marker, furthest ahead first (the hero, e1, is drawn between them and now).
        let e3 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.eventId("e3")))
        let e2 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.eventId("e2")))
        XCTAssertLessThan(e3, e2)
        XCTAssertLessThan(e2, nowIndex)
        // Held meetings sit below it, newest first.
        let h2 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.heldId(2)))
        let h1 = try! XCTUnwrap(ids.firstIndex(of: "m" + HomeRedesignAdapter.heldId(1)))
        XCTAssertGreaterThan(h2, nowIndex)
        XCTAssertLessThan(h2, h1)
    }

    func testMemoryBlocksAreNewestFirstBelowNow() {
        let m = model(data: HomeRedesignData(blocks: [
            CoreDayBlock(startUnixMs: Int64(now.addingTimeInterval(-7200).timeIntervalSince1970 * 1000), endUnixMs: Int64(now.addingTimeInterval(-6000).timeIntervalSince1970 * 1000), apps: [], moreApps: 0),
            CoreDayBlock(startUnixMs: Int64(now.addingTimeInterval(-1800).timeIntervalSince1970 * 1000), endUnixMs: Int64(now.addingTimeInterval(-600).timeIntervalSince1970 * 1000), apps: [], moreApps: 0),
        ]))
        let ids = m.timeline.map(\.id)
        let newer = ids.firstIndex { $0.hasPrefix("b") && $0.contains(String(Int64(now.addingTimeInterval(-1800).timeIntervalSince1970 * 1000))) }
        let older = ids.firstIndex { $0.hasPrefix("b") && $0.contains(String(Int64(now.addingTimeInterval(-7200).timeIntervalSince1970 * 1000))) }
        XCTAssertNotNil(newer)
        XCTAssertNotNil(older)
        XCTAssertLessThan(newer!, older!)
        XCTAssertGreaterThan(newer!, ids.firstIndex(of: "now")!)
    }

    func testYesterdayAndLastWeekRowsUseRecentMeetings() {
        let m = model()
        let histories = m.timeline.compactMap { stop -> WHomeHistory? in if case .history(let h) = stop { return h } else { return nil } }
        // The sample has meetings 3 and 4 yesterday and meeting 5 three days ago.
        XCTAssertEqual(histories.map(\.id), ["yesterday", "lastWeek"])
        XCTAssertEqual(histories[0].meetings.count, 2)
        XCTAssertEqual(histories[1].meetings.count, 1)
        XCTAssertEqual(m.thread(todayTitle: "Today", nowTime: "x").map(\.id), ["today", "yesterday", "lastWeek"])
    }

    func testPeopleComeFromAttendeesMostOftenFirstWithoutAddresses() {
        let people = HomeRedesignAdapter.people(content: content)
        XCTAssertTrue(people.contains("Priya Nair"))
        XCTAssertFalse(people.contains { $0.contains("@") })
        XCTAssertEqual(Set(people).count, people.count)
    }

    func testPausedOrOffMemoryAddsOneNoticeJustBelowNow() {
        let ids = model(memory: .pausedUntil("3:10 PM")).timeline.map(\.id)
        XCTAssertEqual(ids.filter { $0 == "notice" }.count, 1)
        XCTAssertEqual(ids.firstIndex(of: "notice"), ids.firstIndex(of: "now")! + 1)
        XCTAssertNil(model(memory: .on).timeline.first { if case .notice = $0 { return true } else { return false } })
        XCTAssertNotNil(HomeRedesignAdapter.notice(.off))
    }

    func testBelowTheLineKeepsTodaysOrder() {
        let kinds = model().sections.map(\.kind)
        XCTAssertEqual(kinds, [.approvals, .followups, .threads, .recap])
    }

    func testBlocksCarryTitlesAndAppsButNoText() {
        let core = CoreDayBlock(startUnixMs: 1_000_000, endUnixMs: 1_000_000 + 22 * 60_000, apps: [
            CoreDayBlock.App(appBundleId: "com.microsoft.Excel", items: ["Pricing v3.xlsx"], moreItems: 0),
        ], moreApps: 2)
        let block = HomeRedesignAdapter.block(core)
        XCTAssertEqual(block.id, "1000000")
        XCTAssertEqual(block.items.count, 1)
        XCTAssertEqual(block.items[0].glyph, "tablecells")
        XCTAssertNotNil(block.more)
    }

    func testMeetingIntervalsMergeOverlaps() {
        let day = Calendar.current.startOfDay(for: now)
        let spans = HomeRedesignAdapter.meetingIntervals(content: content, startOfDay: day, endOfDay: day.addingTimeInterval(86400))
        XCTAssertEqual(spans.map(\.start), spans.map(\.start).sorted())
        for (a, b) in zip(spans, spans.dropFirst()) { XCTAssertLessThan(a.end, b.start) }
    }

    func testFilterBecomesMillisecondBounds() {
        let f = HomeRedesignAdapter.libraryFilter(WHomeFilter(appId: "com.apple.mail", dates: .today), apps: [], strings: .english, now: now)
        XCTAssertEqual(f.appId, "com.apple.mail")
        XCTAssertEqual(f.endUnixMs - f.startUnixMs, 86_400_000, accuracy: 3_600_000)
        XCTAssertEqual(f.dateLabel, "Today")
        XCTAssertEqual(HomeRedesignAdapter.libraryFilter(WHomeFilter(person: "Sam"), apps: [], strings: .english, now: now).person, "Sam")
        let none = HomeRedesignAdapter.libraryFilter(WHomeFilter(appId: "x"), apps: [], strings: .english, now: now)
        XCTAssertEqual(none.startUnixMs, 0)
        XCTAssertEqual(none.endUnixMs, 0)
    }

    func testPersonTermIsQuotedAndJoinedToTheQuery() {
        XCTAssertEqual(TimelineView.searchTerm(query: "", person: "Priya Shah"), "\"Priya Shah\"")
        XCTAssertEqual(TimelineView.searchTerm(query: "pricing", person: "Priya \"P\" Shah"), "pricing \"Priya P Shah\"")
        XCTAssertEqual(TimelineView.searchTerm(query: "pricing", person: nil), "pricing")
    }

    func testAppDotColorIsSteady() {
        XCTAssertEqual(HomeRedesignAdapter.kind("com.apple.mail"), HomeRedesignAdapter.kind("com.apple.mail"))
    }
}
