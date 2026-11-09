import SwiftUI
import XCTest
@testable import Whimbrel

@MainActor
final class TodayChromeTests: XCTestCase {
    @MainActor
    private struct PlacesSchedule: @MainActor TodayArchetypeView {
        init(content: TodayContent, actions: TodayActions, composer: AnyView?) {}
        static var placedSections: Set<TodaySection> { [.schedule] }
        var body: some View { EmptyView() }
    }

    func test_standard_has_no_separate_slots() {
        XCTAssertEqual(TodayChrome.order(for: .standard), [.archetype])
    }

    func test_other_archetypes_put_slots_first() {
        for archetype in SkinArchetype.allCases where archetype != .standard {
            XCTAssertEqual(TodayChrome.order(for: archetype), [.slots, .archetype, .leftovers], "\(archetype)")
        }
    }

    func test_leftovers_are_nonEmpty_minus_placed() {
        let all = TodayContent.sample.nonEmptySections()
        let left = TodayChrome.leftovers(of: .sample, placed: PlacesSchedule.placedSections)
        XCTAssertEqual(left, all.filter { $0 != .schedule })
        XCTAssertFalse(left.contains(.schedule))
        XCTAssertTrue(all.contains(.schedule))
    }

    func test_empty_content_has_no_leftovers() {
        // Empty still has a greeting, devices and the composer; an archetype placing those leaves nothing.
        let placed = Set(TodaySection.allCases)
        XCTAssertTrue(TodayChrome.leftovers(of: .empty, placed: placed).isEmpty)
        XCTAssertTrue(TodayChrome.leftovers(of: .sample, placed: placed).isEmpty)
        XCTAssertEqual(TodayChrome.leftovers(of: .empty, placed: PlaceholderArchetype.placedSections), [.greeting, .composer, .devices])
    }

    func test_nonEmptySections_sample() {
        XCTAssertEqual(
            TodayContent.sample.nonEmptySections(),
            [.greeting, .composer, .upNext, .schedule, .earlier, .threads, .followups, .recap, .devices])
    }

    func test_nonEmptySections_empty() {
        XCTAssertEqual(TodayContent.empty.nonEmptySections(), [.greeting, .composer, .devices])
    }

    func test_next_meeting_appears_once_in_leftovers() {
        let all = TodayContent.sample.agenda.upcomingToday
        XCTAssertGreaterThan(all.count, 1)
        // Leftovers use the shared row models: the next event is in `.upNext`, never repeated, and held meetings follow in ascending order.
        let schedule = TodayChrome.leftoverSchedule(of: .sample)
        XCTAssertEqual(schedule.upcoming.map(\.id), Array(all.dropFirst()))
        XCTAssertFalse(schedule.upcoming.map(\.id).contains(all[0]))
        XCTAssertEqual(schedule.held.map(\.title), ["Standup", "Customer call: Acme"])
        XCTAssertEqual(TodayRows.heldSeparator, "Earlier today")
    }

    func test_no_schedule_leftover_when_only_up_next_remains() {
        var content = TodayContent.sample
        content.events = Array(content.events.prefix(1))
        content.recentMeetings = content.recentMeetings.filter { !content.agenda.pastToday.contains($0.id) }
        XCTAssertTrue(content.nonEmptySections().contains(.schedule), "the section has up next to show")
        XCTAssertTrue(TodayChrome.leftoverSchedule(of: content).upcoming.isEmpty && TodayChrome.leftoverSchedule(of: content).held.isEmpty)
        XCTAssertFalse(TodayChrome.leftovers(of: content, placed: []).contains(.schedule), "up next lists it; an empty schedule block is not drawn")
        XCTAssertTrue(TodayChrome.leftovers(of: content, placed: []).contains(.upNext))
        XCTAssertTrue(TodayChrome.leftovers(of: .sample, placed: []).contains(.schedule))
    }

    func test_upcoming_groups_by_day_and_collapses_after_two() {
        typealias S = TodaySampleContent
        var content = TodayContent.sample
        XCTAssertFalse(content.nonEmptySections().contains(.upcoming), "an empty week draws nothing")
        content.upcomingWeek = [
            S.event("d3", "Review", S.at(dayOffset: 4, hour: 9)),
            S.event("d1b", "Lunch", S.at(dayOffset: 1, hour: 12), attendees: ["A", "B", "C"]),
            S.event("d1a", "Standup", S.at(dayOffset: 1, hour: 9)),
            S.event("d2", "Planning", S.at(dayOffset: 2, hour: 15), attendees: ["A"]),
        ]
        XCTAssertTrue(content.nonEmptySections().contains(.upcoming))
        let collapsed = TodayRows.upcomingDays(content, expanded: false)
        XCTAssertEqual(collapsed.map { $0.rows.map(\.title) }, [["Standup", "Lunch"], ["Planning"]])
        XCTAssertEqual(collapsed[0].heading, "Tomorrow")
        XCTAssertEqual(collapsed[1].heading, S.at(dayOffset: 2, hour: 0).formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()))
        XCTAssertEqual(collapsed[0].rows.map(\.invited), ["", "3 invited"])
        XCTAssertTrue(TodayRows.upcomingHasMore(content))
        XCTAssertEqual(TodayRows.upcomingDays(content, expanded: true).count, 3)
        content.upcomingWeek.removeFirst()
        XCTAssertFalse(TodayRows.upcomingHasMore(content), "two days fit without the toggle")
    }
}
