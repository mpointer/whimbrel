import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

private final class WidthBox { var width: CGFloat = 0 }
private struct WidthKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// A small seeded generator, so the property tests below are repeatable.
private struct Lcg: RandomNumberGenerator {
    var state: UInt64
    mutating func next() -> UInt64 {
        state = state &* 6364136223846793005 &+ 1442695040888963407
        return state
    }
}

@MainActor
final class ArchetypeFieldTests: XCTestCase {
    /// <repo>/skins, found from this file's path (apps/macos/WhimbrelTests/ArchetypeFieldTests.swift).
    private let skins = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("skins")

    private let paletteIds = ["navy-red", "black-gold", "green-white", "purple-gold", "orange-blue"]

    private func skin() throws -> Skin {
        let (skin, repairs) = try XCTUnwrap(SkinLoader.parse(try Data(contentsOf: skins.appendingPathComponent("game-day.json")), base: SkinDefaults.whimbrel))
        XCTAssertEqual(repairs, [])
        return skin
    }

    func test_game_day_skin_loads_without_repairs() throws {
        let skin = try skin()
        XCTAssertEqual(skin.id, "game-day")
        XCTAssertEqual(skin.paletteIds, paletteIds)
        XCTAssertEqual(skin.archetype, .field)
        XCTAssertEqual(skin.typography.numerals.family, .helveticaNeue)
    }

    private func palette(_ id: String) throws -> Theme {
        let data = try Data(contentsOf: skins.appendingPathComponent("palettes/\(id).json"))
        return try XCTUnwrap(ThemeLoader.parse(data), id)
    }

    func test_game_day_palettes_pass_contrast() throws {
        for id in paletteIds {
            let theme = try palette(id)
            XCTAssertNotEqual(theme.light, theme.dark, "\(id) needs its own light and dark variants")
            XCTAssertNotEqual(theme.dark, ThemeDefaults.theme.dark, "\(id) must not have fallen back to Default")
            for (mode, p) in [("light", theme.light), ("dark", theme.dark)] {
                XCTAssertEqual(Set(p.keys), Set(ThemeToken.allCases), "\(id) \(mode)")
                XCTAssertEqual(ThemeContrast.failures(p), [], "\(id) \(mode)")
            }
        }
    }

    /// The pitch is `backgroundCard` under accent stripes, with chalk lines in `border`. Labels sit on it, so text
    /// must pass 4.5:1 on the stripe-and-line composite (`ArchetypeContrast`'s Field surfaces), and the markers
    /// (`accentPrimary`) must stand out from the pitch by the 3:1 non-text minimum.
    func test_pitch_text_and_markers_have_contrast() throws {
        for id in paletteIds {
            let theme = try palette(id)
            for (mode, p) in [("light", theme.light), ("dark", theme.dark)] {
                XCTAssertEqual(ArchetypeContrast.failures(p, ArchetypeContrast.pairs(for: .field)), [], "\(id) \(mode)")
                let card = try XCTUnwrap(p[.backgroundCard]), accent = try XCTUnwrap(p[.accentPrimary]), border = try XCTUnwrap(p[.border])
                let striped = GlassStyle.composite(accent, alpha: FieldStyle.stripeOpacity, over: card)
                for (surface, name) in [(striped, "stripe"), (GlassStyle.composite(border, alpha: FieldStyle.lineOpacity, over: striped), "stripe+line")] {
                    let marker = ThemeContrast.ratio(accent, surface)
                    XCTAssertGreaterThanOrEqual(marker, 3, "\(id) \(mode) marker on pitch \(name) = \(marker)")
                }
            }
        }
    }

    func test_palette_names_are_colour_pairs_only() throws {
        let colours: Set<String> = ["navy", "red", "black", "gold", "green", "white", "purple", "orange", "blue"]
        for id in paletteIds {
            let name = try palette(id).name
            let words = name.split(separator: " ").map(String.init)
            XCTAssertEqual(words.count, 3, name)
            XCTAssertEqual(words[1], "and", name)
            XCTAssertTrue(colours.contains(words[0].lowercased()) && colours.contains(words[2]) && words[0] != words[2], "\(name) is not a colour pair")
        }
    }

    func test_field_places_expected_sections() {
        XCTAssertEqual(FieldTodayLayout.placedSections, [.greeting, .composer, .upNext, .schedule, .earlier])
    }

    // MARK: pitch geometry (pure)

    private func starts(_ minutes: [Int]) -> [Date] {
        minutes.map { TodayFixtures.now.addingTimeInterval(Double($0) * 60) }
    }

    func test_slots_are_in_start_order_and_never_closer_than_a_marker() {
        for width in [480, 560, 612, 700] as [CGFloat] {
            for minutes in [[], [0], [0, 1], [0, 0, 0], [0, 5, 10, 600], [0, 30, 60, 90, 120, 150, 180, 210], [0, 1, 2, 3, 4, 5, 6, 7], [0, 700, 701, 702, 703]] {
                let slots = FieldGeometry.slots(starts: starts(minutes), width: width)
                XCTAssertEqual(slots.count, minutes.count)
                for pair in zip(slots, slots.dropFirst()) {
                    XCTAssertGreaterThanOrEqual(pair.1.centerX - pair.0.centerX, FieldGeometry.minGap - 0.001, "\(minutes) at \(width)")
                }
                XCTAssertEqual(slots.map(\.centerX), slots.map(\.centerX).sorted())
            }
        }
    }

    func test_slots_follow_time_when_there_is_room() {
        let slots = FieldGeometry.slots(starts: starts([0, 60, 120]), width: 612)
        XCTAssertEqual(slots[1].centerX - slots[0].centerX, slots[2].centerX - slots[1].centerX, accuracy: 0.5, "equal hours sit equally far apart")
    }

    /// A label starts at its marker and ends before the next label in its row. Rows alternate, so the
    /// labels never share a row with a neighbour, and none leaves the pitch.
    func test_labels_never_collide_or_leave_the_pitch() {
        var rng = Lcg(state: 42)
        for _ in 0..<400 {
            let n = Int.random(in: 1...FieldDay.maxMarkers, using: &rng)
            let width = CGFloat.random(in: 420...760, using: &rng)
            let minutes = (0..<n).map { _ in Int.random(in: 0...(Bool.random(using: &rng) ? 8 : 900), using: &rng) }.sorted()
            let slots = FieldGeometry.slots(starts: starts(minutes), width: width)
            for (i, slot) in slots.enumerated() {
                XCTAssertGreaterThan(slot.labelWidth, 0)
                XCTAssertGreaterThanOrEqual(slot.labelX, 0)
                XCTAssertLessThanOrEqual(slot.labelX + slot.labelWidth, width + 0.001, "\(minutes) at \(width)")
                XCTAssertEqual(slot.row, i % 2)
                if i + 2 < slots.count {
                    XCTAssertLessThanOrEqual(slot.labelX + slot.labelWidth + FieldGeometry.labelGap, slots[i + 2].labelX + 0.001, "same-row labels collide: \(minutes) at \(width)")
                }
                // The marker is at its label's leading edge.
                XCTAssertEqual(slot.centerX - FieldGeometry.markerDiameter / 2, slot.labelX, accuracy: 0.001)
            }
        }
    }

    func test_every_label_has_room_for_a_time() {
        for n in 1...FieldDay.maxMarkers {
            let slots = FieldGeometry.slots(starts: starts(Array(repeating: 0, count: n)), width: 560)
            for slot in slots { XCTAssertGreaterThanOrEqual(slot.labelWidth, 60, "n=\(n)") }
        }
    }

    // MARK: marker model

    /// `sample` plus `count` more events later today, three of them at the very same time.
    private func busy(extra count: Int) -> TodayContent {
        var content = TodayContent.sample
        let calendar = Calendar.current
        for i in 0..<count {
            let hour = 16 + min(i, 6), minute = i > 6 ? 0 : (i % 2) * 30
            let start = calendar.date(bySettingHour: hour, minute: minute, second: 0, of: TodayFixtures.now)!
            content.events.append(QualifyingMeeting(
                id: "x\(i)", title: "Busy block \(i) with a title that is much too long to fit beside its neighbour",
                startDate: start, endDate: start.addingTimeInterval(1800), attendees: [], conferenceLink: nil, source: .calendar, appBundleID: nil, organizerEmail: nil, organizerName: nil, selfIsOrganizer: false, selfResponse: nil
            ))
        }
        return content
    }

    func test_markers_are_the_days_meetings_by_start_time() throws {
        let day = FieldDay.make(TodayContent.sample)
        XCTAssertEqual(day.markers.map(\.title), ["Standup", "Customer call: Acme", "Design review", "Pipeline sync", "1:1 with Sam"])
        XCTAssertEqual(day.markers.map(\.start), day.markers.map(\.start).sorted())
        XCTAssertEqual(day.markers.filter(\.isNext).map(\.title), ["Design review"])
        XCTAssertEqual(day.hiddenCount, 0)
        XCTAssertTrue(day.overflow.upcoming.isEmpty && day.overflow.held.isEmpty)
    }

    func test_a_marker_has_the_same_action_as_its_schedule_row() throws {
        var content = TodayContent.sample
        var day = FieldDay.make(content)
        let sync = try XCTUnwrap(day.markers.first { $0.title == "Pipeline sync" })
        let standup = try XCTUnwrap(day.markers.first { $0.title == "Standup" })
        guard case let .start(event)? = sync.action else { return XCTFail("an upcoming event starts") }
        XCTAssertEqual(event.title, "Pipeline sync")
        guard case let .open(id)? = standup.action else { return XCTFail("a held meeting opens") }
        XCTAssertEqual(id, 1)
        XCTAssertEqual(sync.accessibilityLabel, "Start Pipeline sync, \(sync.timeText)")
        XCTAssertEqual(standup.accessibilityLabel, "Open Standup, \(standup.timeText)")
        // TodayRows' gating: Start is offered only while no meeting is active.
        content.activeMeetingId = 7
        day = FieldDay.make(content)
        let gated = try XCTUnwrap(day.markers.first { $0.title == "Pipeline sync" })
        XCTAssertNil(gated.action)
        XCTAssertEqual(gated.accessibilityLabel, "Pipeline sync, \(gated.timeText)")
        XCTAssertNotNil(day.markers.first { $0.title == "Standup" }?.action, "Open does not depend on an active meeting")
    }

    func test_more_than_eight_meetings_collapse_to_a_list_under_the_pitch() throws {
        let content = busy(extra: 9)
        let day = FieldDay.make(content)
        XCTAssertEqual(day.markers.count, FieldDay.maxMarkers)
        XCTAssertEqual(day.markers.map(\.start), day.markers.map(\.start).sorted())
        XCTAssertEqual(day.hiddenCount, 14 - 8)
        XCTAssertEqual(day.markers.filter(\.isHeld).count, 0, "upcoming meetings take the pitch first")
        XCTAssertEqual(day.hiddenCount, day.overflow.upcoming.count + day.overflow.held.count)
        // Every meeting is on the pitch or in the list, once.
        let shown = day.markers.map(\.id) + day.overflow.upcoming.map(\.id) + day.overflow.held.map { "held-\($0.id)" }
        XCTAssertEqual(Set(shown).count, shown.count)
        XCTAssertEqual(shown.count, 14)
        XCTAssertEqual(day.overflow.upcoming.map(\.start), day.overflow.upcoming.map(\.start).sorted())
        XCTAssertEqual(day.moreLabel, "+6 more")
        // Eight fit; nine do not.
        XCTAssertEqual(FieldDay.make(busy(extra: 3)).hiddenCount, 0)
        XCTAssertEqual(FieldDay.make(busy(extra: 4)).hiddenCount, 1)
    }

    /// A busy day: upcoming meetings come first (from up next on), the latest held ones fill what is left,
    /// and the older held ones go to the list.
    func test_busy_day_puts_up_next_and_the_upcoming_meetings_on_the_pitch() throws {
        var content = busy(extra: 3)
        content.recentMeetings += (100..<110).map { i in
            let day = Calendar.current.startOfDay(for: TodayFixtures.now)
            let start = Calendar.current.date(byAdding: .minute, value: 10 * Int(i - 100), to: day)!
            return CoreMeeting(
                id: Int64(i), deviceId: "mac-1", startUnixMs: Int64(start.timeIntervalSince1970 * 1000), endUnixMs: 0, title: "Early \(i)",
                attendees: [], trigger: .calendar, audioPath: nil, status: .completed, transcriptPath: nil, notes: nil, summaryTldr: nil,
                summaryDecisions: [], summaryActionItems: [], summaryOpenQuestions: [], summaryProvider: nil, summaryModel: nil, estimatedSpeakerCount: nil
            )
        }
        let day = FieldDay.make(content)
        let upcoming = TodayRows.schedule(content, upNextPlaced: false).upcoming
        XCTAssertEqual(upcoming.count, 6)
        XCTAssertEqual(day.markers.count, FieldDay.maxMarkers)
        XCTAssertTrue(day.markers.contains { $0.isNext }, "up next is on the pitch")
        for event in upcoming { XCTAssertTrue(day.markers.contains { $0.id == event.id }, "\(event.title) is a marker") }
        // The two slots left go to the latest held meetings, and the pitch stays in time order.
        XCTAssertEqual(day.markers.filter(\.isHeld).count, 2)
        XCTAssertEqual(day.markers.map(\.start), day.markers.map(\.start).sorted())
        XCTAssertEqual(day.markers.prefix(2).map(\.title), ["Standup", "Customer call: Acme"], "the latest held ones, not the earliest")
        XCTAssertTrue(day.overflow.upcoming.isEmpty)
        XCTAssertEqual(day.overflow.held.count, 10 + 2 - 2)
        XCTAssertEqual(day.hiddenCount, 10)
        XCTAssertFalse(day.overflow.held.contains { $0.title == "Customer call: Acme" || $0.title == "Standup" })
    }

    func test_more_than_eight_upcoming_fill_the_pitch_and_the_rest_list() {
        let day = FieldDay.make(busy(extra: 9))
        XCTAssertEqual(day.markers.filter(\.isHeld).count, 0)
        XCTAssertEqual(day.overflow.upcoming.count, 4)
        XCTAssertEqual(day.overflow.held.map(\.title), ["Standup", "Customer call: Acme"])
    }

    func test_up_next_marker_is_announced_as_up_next() throws {
        var content = TodayContent.sample
        var next = try XCTUnwrap(FieldDay.make(content).markers.first { $0.isNext })
        XCTAssertEqual(next.accessibilityLabel, "Start Design review, \(next.timeText), up next")
        content.activeMeetingId = 7
        next = try XCTUnwrap(FieldDay.make(content).markers.first { $0.isNext })
        XCTAssertEqual(next.accessibilityLabel, "Design review, \(next.timeText), up next")
    }

    func test_empty_day_has_no_markers() {
        let day = FieldDay.make(TodayContent.empty)
        XCTAssertTrue(day.markers.isEmpty)
        XCTAssertEqual(day.hiddenCount, 0)
        XCTAssertNil(day.moreLabel)
    }

    // MARK: rendering

    /// Runs `body` with Game Day and `palette` in force. The palettes are installed in the
    /// theme store for the duration, since the test host does not load `skins/palettes`.
    private func withField<T>(palette: String = "navy-red", _ body: () throws -> T) throws -> T {
        let store = ThemeStore.shared
        let before = store.themes
        store.install(before + ThemeLoader.loadDirectory(skins.appendingPathComponent("palettes")))
        defer { store.install(before) }
        return try SkinStore.shared.withOverride(skin: try skin(), paletteId: palette, body)
    }

    private func layout(_ content: TodayContent) -> FieldTodayLayout {
        FieldTodayLayout(content: content, actions: .noop, composer: TodayFixtures.composer)
    }

    /// Content widths: 672 / 692 at the 900pt minimum window (sidebar 180 / 160 plus 24 padding each
    /// side), 660 the brief's stricter figure, 760 the widest `TodayChrome` allows.
    private let widths: [CGFloat] = [660, 672, 692, 760]

    func test_field_no_horizontal_overflow() throws {
        try withField {
            for content in [TodayContent.worstCase, .sample, .empty, busy(extra: 9)] {
                for width in widths {
                    let measured = renderedWidth(layout(content), proposing: width)
                    XCTAssertLessThanOrEqual(measured, width + 0.5, "content wider than \(width)")
                }
            }
        }
    }

    /// The width the layout takes when offered `width`: a layout that cannot shrink to it reports more.
    /// (A plain `fittingSize` is no use: wrapping text's ideal width is its unwrapped width.)
    private func renderedWidth<V: View>(_ view: V, proposing width: CGFloat) -> CGFloat {
        let box = WidthBox()
        let probe = view
            .fixedSize(horizontal: false, vertical: true)
            .background(GeometryReader { Color.clear.preference(key: WidthKey.self, value: $0.size.width) })
            .frame(width: width, alignment: .leading)
            .onPreferenceChange(WidthKey.self) { box.width = $0 }
        let host = NSHostingView(rootView: probe)
        host.frame = CGRect(x: 0, y: 0, width: width, height: 6000)
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        host.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(box.width, 0, "the probe measured nothing")
        return box.width
    }

    /// The probe must see an overflow, or the test above proves nothing.
    func test_overflow_probe_detects_a_rigid_wide_view() {
        XCTAssertGreaterThan(renderedWidth(Text(String(repeating: "x", count: 200)).fixedSize(), proposing: 660), 660)
        XCTAssertLessThanOrEqual(renderedWidth(Text(String(repeating: "x ", count: 200)), proposing: 660), 660.5)
    }

    // MARK: snapshots (local only)

    private func page(_ content: TodayContent) -> some View {
        ScrollView { layout(content).padding(Spacing.xl).frame(maxWidth: 760, alignment: .leading).frame(maxWidth: .infinity) }
            .background(Color.whimbrelBackgroundPage)
    }

    func test_field_sample_light() throws {
        try withField { try assertSnapshot(page(.sample), named: "field-sample-light", size: CGSize(width: 900, height: 780), colorScheme: .light) }
    }

    func test_field_sample_dark() throws {
        try withField(palette: "black-gold") { try assertSnapshot(page(.sample), named: "field-sample-dark", size: CGSize(width: 900, height: 780), colorScheme: .dark) }
    }

    func test_field_busy_light() throws {
        try withField(palette: "green-white") { try assertSnapshot(page(busy(extra: 9)), named: "field-busy-light", size: CGSize(width: 900, height: 1500), colorScheme: .light) }
    }

    func test_field_worstcase_dark() throws {
        try withField(palette: "orange-blue") { try assertSnapshot(page(.worstCase), named: "field-worstcase-dark", size: CGSize(width: 900, height: 2000), colorScheme: .dark) }
    }
}
