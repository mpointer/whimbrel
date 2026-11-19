import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

@Suite struct HomeTests {
    @Test func elementsFollowTheOldArchetypeWhenNoKeysAreSet() {
        #expect(WHomeElements.resolve(options: [:], archetype: "standard") == WHomeElements())
        #expect(WHomeElements.resolve(options: [:], archetype: "glass") == WHomeElements())
        #expect(WHomeElements.resolve(options: [:], archetype: "field") == WHomeElements(nextMeeting: .scoreboard))
        #expect(WHomeElements.resolve(options: [:], archetype: "terminal") == WHomeElements(labels: .bracketed, ask: .prompt))
        #expect(WHomeElements.resolve(options: [:], archetype: "almanac") == WHomeElements(timeline: .ruled, labels: .smallCaps))
        #expect(WHomeElements.resolve(options: [:], archetype: "reference") == WHomeElements(nextMeeting: .infobox, timeline: .ruled, actions: .links))
    }

    @Test func aKeyLeftOutKeepsTheArchetypeAndAnUnknownValueTakesTheDefault() {
        let r = WHomeElements.resolve(options: ["labels": "plain", "ask": "nonsense", "other": "x"], archetype: "reference")
        #expect(r.labels == .plain)
        #expect(r.ask == .field)
        #expect(r.nextMeeting == .infobox)
        #expect(r.timeline == .ruled)
        #expect(r.actions == .links)
    }

    @Test func keysListEveryValue() {
        #expect(WHomeElements.keys.map(\.key) == ["nextMeeting", "timeline", "labels", "ask", "actions"])
        #expect(WHomeElements.keys[2].values == ["plain", "bracketed", "smallCaps"])
    }

    @Test func dateRangesAreMidnightToMidnight() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        let now = cal.date(from: DateComponents(year: 2026, month: 11, day: 17, hour: 9, minute: 52))!
        func day(_ d: Int) -> Date { cal.date(from: DateComponents(year: 2026, month: 11, day: d))! }
        #expect(WHomeDateRange.today.bounds(now: now, calendar: cal) == (day(17), day(18)))
        #expect(WHomeDateRange.yesterday.bounds(now: now, calendar: cal) == (day(16), day(17)))
        #expect(WHomeDateRange.lastSevenDays.bounds(now: now, calendar: cal) == (day(11), day(18)))
        let custom = WHomeDateRange.custom(from: day(12), to: day(10)).bounds(now: now, calendar: cal)
        #expect(custom == (day(10), day(13)))
    }

    @Test func filterIsEmptyOnlyWithNothingChosen() {
        #expect(WHomeFilter().isEmpty)
        #expect(!WHomeFilter(appId: "a").isEmpty)
        #expect(!WHomeFilter(dates: .today).isEmpty)
    }

    @Test func anEmptyDayHasNoMeetingsBlocksOrSections() {
        var m = WHomeModel()
        m.timeline = [.now(time: "9:52 AM", label: "Memory on")]
        #expect(m.isEmptyDay)
        m.hero = .next(WHomeMeeting(id: "1", kind: .upcoming, time: "10 AM", title: "x"))
        #expect(!m.isEmptyDay)
        m.hero = .none
        m.sections = [WHomeSection(kind: .approvals, title: "t", rows: [])]
        #expect(!m.isEmptyDay)
    }

    @Test @MainActor func stopsHaveStableDistinctIds() {
        let stops = WHomeGalleryIds.stops
        #expect(Set(stops.map(\.id)).count == stops.count)
    }

    @Test func spokenStopNamesStartWithTheTime() {
        let s = WStrings.english
        let m = WHomeMeeting(id: "1", kind: .upcoming, time: "10:31 AM", title: "Northwind security review", eyebrow: "in 9 minutes")
        #expect(WHomeSpoken.meeting(m, strings: s) == "10:31 AM, Northwind security review, in 9 minutes")
        #expect(WHomeSpoken.now(time: "10:22 AM", strings: s) == "Now, 10:22 AM")
        let b = WHomeBlock(id: "b", timeRange: "9:30 to 9:52 AM", duration: "22 min", items: [WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing")], openTitle: "Open")
        #expect(WHomeSpoken.block(b, strings: s) == "9:30 to 9:52 AM, Memory, Pricing")
    }

    @Test @MainActor func theHeroIsDrawnJustAboveTheNowMarkerAndTheThreadListsPartsWithContent() {
        let m = WHomeGallery.model()
        let ids = WHomeLineRow.rows(m).map(\.id)
        let now = ids.firstIndex(of: "now")!
        #expect(ids[now - 1] == "hero")
        // Later meetings come before the hero, furthest ahead first.
        #expect(ids.firstIndex(of: "mm-hiring")! < ids.firstIndex(of: "mm-design")!)
        #expect(ids.firstIndex(of: "mm-design")! < ids.firstIndex(of: "hero")!)
        #expect(m.thread(todayTitle: "Today", nowTime: "9:52 AM").map(\.id) == ["today", "yesterday", "lastWeek"])
        var none = m
        none.timeline = none.timeline.filter { if case .history = $0 { return false } else { return true } }
        #expect(none.thread(todayTitle: "Today", nowTime: "9:52 AM").map(\.id) == ["today"])
        none.hero = .none
        #expect(!WHomeLineRow.rows(none).contains { $0.id == "hero" })
    }

    @Test func aPersonCountsAsAFilter() {
        #expect(!WHomeFilter(person: "Sam Okafor").isEmpty)
    }

    @Test func textLinksKeepA24PointTarget() {
        #expect(WTextLink.minimumTarget >= 24)
    }
}

@MainActor
private enum WHomeGalleryIds {
    static var stops: [WHomeStop] { WHomeGallery.model().timeline }
}
