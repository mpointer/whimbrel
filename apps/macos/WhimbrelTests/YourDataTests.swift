import XCTest
import SwiftUI
@testable import Whimbrel

@MainActor
final class YourDataTests: XCTestCase {
    func test_site_exclusions_match_the_host_and_its_subdomains_only() {
        XCTAssertEqual(CaptureExclusions.normalizedDomain("  HTTPS://Example.com/path?x=1 "), "example.com")
        XCTAssertEqual(CaptureExclusions.normalizedDomain("user@mail.example.com:8080"), "mail.example.com")
        XCTAssertNil(CaptureExclusions.normalizedDomain("not a domain"))
        XCTAssertNil(CaptureExclusions.normalizedDomain("localhost"))
        XCTAssertTrue(CaptureExclusions.host("app.example.com", matches: "example.com"))
        XCTAssertTrue(CaptureExclusions.host("example.com", matches: "example.com"))
        XCTAssertFalse(CaptureExclusions.host("notexample.com", matches: "example.com"))
        XCTAssertEqual(CaptureExclusions.host(of: "https://Bank.Example.com/accounts"), "bank.example.com")
        XCTAssertNil(CaptureExclusions.host(of: "file:///Users/me/notes.txt"))
    }

    func test_an_excluded_site_is_skipped_by_url_and_removed_again() {
        let exclusions = CaptureExclusions.shared
        XCTAssertTrue(exclusions.addExcludedDomain("example.com"))
        XCTAssertTrue(exclusions.isExcluded(url: "https://mail.example.com/inbox"))
        XCTAssertFalse(exclusions.isExcluded(url: "https://example.org/"))
        XCTAssertFalse(exclusions.addExcludedDomain("???"))
        exclusions.removeExcludedDomain("example.com")
        XCTAssertFalse(exclusions.isExcluded(url: "https://mail.example.com/inbox"))
        exclusions.setExcludedDomains(["A.com", "b.com/x"])
        XCTAssertEqual(exclusions.excludedDomains, ["a.com", "b.com"])
        exclusions.setExcludedDomains([])
    }

    func test_providers_row_mentions_window_text_only_with_a_hosted_provider() {
        let sentence = "The text of the window you ask about is sent to the model you chose, as in Ask."
        func sees(_ hosted: [String], localOnly: Bool) -> String {
            YourData.outbound(YourData.Inputs(hostedProviders: hosted, localOnly: localOnly, syncOn: false, syncDevices: 0, googleAccounts: [], typesafeOn: false))
                .first { $0.channel == .providers }!.sees
        }
        XCTAssertTrue(sees(["Anthropic"], localOnly: false).contains(sentence))
        XCTAssertFalse(sees([], localOnly: false).contains(sentence))
        XCTAssertFalse(sees(["Anthropic"], localOnly: true).contains(sentence))
        XCTAssertTrue(sees(["Anthropic"], localOnly: false).hasPrefix("The provider whose key you added receives the text needed for that request, under its own terms. Known secrets are redacted first."))
    }

    func test_outbound_rows_report_each_channel_state() {
        let off = YourData.Inputs(hostedProviders: [], localOnly: false, syncOn: false, syncDevices: 0, googleAccounts: [], typesafeOn: false)
        let rows = YourData.outbound(off)
        XCTAssertEqual(rows.map(\.channel), YourData.Outbound.Channel.allCases)
        XCTAssertEqual(rows.filter(\.active).map(\.channel), [.updateCheck])

        let on = YourData.Inputs(hostedProviders: ["Anthropic"], localOnly: false, syncOn: true, syncDevices: 2, googleAccounts: ["me@example.test"], typesafeOn: true)
        XCTAssertTrue(YourData.outbound(on).allSatisfy(\.active))

        let local = YourData.Inputs(hostedProviders: ["Anthropic"], localOnly: true, syncOn: false, syncDevices: 0, googleAccounts: [], typesafeOn: false)
        XCTAssertFalse(YourData.outbound(local)[0].active)
    }

    func test_delete_everything_keeps_the_models_folder_and_removes_the_rest() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("yourdata-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("models"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("snapshots"), withIntermediateDirectories: true)
        try "x".write(to: root.appendingPathComponent("core-memory.sqlite"), atomically: true, encoding: .utf8)
        try "x".write(to: root.appendingPathComponent("system-audio-1.m4a"), atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: root) }

        let entries = YourData.entriesToDelete(in: root)
        XCTAssertEqual(entries.map(\.lastPathComponent), ["core-memory.sqlite", "snapshots", "system-audio-1.m4a"])
        XCTAssertTrue(YourData.folderBytes(root) > 0)
        XCTAssertEqual(YourData.delete(entries), [])
        let left = try FileManager.default.contentsOfDirectory(atPath: root.path)
        XCTAssertEqual(left, ["models"])
    }

    func test_delete_everything_takes_this_macs_recordings_and_slides_from_the_media_folder() throws {
        let fm = FileManager.default
        let media = fm.temporaryDirectory.appendingPathComponent("yourdata-media-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: media) }
        let mine = media.appendingPathComponent("Audio/this-mac", isDirectory: true)
        let other = media.appendingPathComponent("Audio/other-mac", isDirectory: true)
        let snapshots = media.appendingPathComponent("Snapshots", isDirectory: true)
        for dir in [mine, other] { try fm.createDirectory(at: dir, withIntermediateDirectories: true) }
        try "x".write(to: mine.appendingPathComponent("mic-audio-1.caf"), atomically: true, encoding: .utf8)
        let meeting = try SnapshotStorage.create("2026-11-04 10.00 Standup", under: snapshots)
        try fm.createDirectory(at: snapshots.appendingPathComponent("My own folder"), withIntermediateDirectories: true)

        let entries = YourData.mediaEntriesToDelete(audio: mine, snapshots: snapshots)
        XCTAssertEqual(entries.map(\.lastPathComponent), ["this-mac", meeting.lastPathComponent])
        XCTAssertEqual(YourData.delete(entries), [])
        XCTAssertFalse(fm.fileExists(atPath: mine.path))
        XCTAssertTrue(fm.fileExists(atPath: other.path))
        XCTAssertTrue(fm.fileExists(atPath: snapshots.appendingPathComponent("My own folder").path))
        // A missing media folder (an unmounted drive) lists nothing.
        XCTAssertEqual(YourData.mediaEntriesToDelete(audio: media.appendingPathComponent("gone/Audio/x"), snapshots: media.appendingPathComponent("gone/Snapshots")), [])
    }

    func test_delete_everything_runs_on_the_next_launch_once_the_old_instance_has_quit() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("yourdata-pending-\(UUID().uuidString)", isDirectory: true)
        let media = fm.temporaryDirectory.appendingPathComponent("yourdata-pending-media-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: root); try? fm.removeItem(at: media) }
        try fm.createDirectory(at: root.appendingPathComponent("models"), withIntermediateDirectories: true)
        try fm.createDirectory(at: media, withIntermediateDirectories: true)
        try "x".write(to: root.appendingPathComponent("core-memory.sqlite"), atomically: true, encoding: .utf8)
        try "x".write(to: media.appendingPathComponent("mic-audio-1.caf"), atomically: true, encoding: .utf8)

        // Scheduling removes nothing: the running app still has the database open.
        let entries = YourData.entriesToDelete(in: root) + [media]
        try YourData.scheduleDelete(entries, in: root, pid: 4242)
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))

        // The old instance has not quit in time: nothing is removed, all of it is reported,
        // and the note is gone so a later launch can't delete data made after this one.
        XCTAssertEqual(YourData.runPendingDelete(in: root, timeout: 0, isRunning: { _ in true })?.map(\.path), entries.map(\.path))
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
        XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent(YourData.pendingDeleteName).path))

        // An unreadable note is reported, not treated as nothing pending.
        try "garbage".write(to: root.appendingPathComponent(YourData.pendingDeleteName), atomically: true, encoding: .utf8)
        XCTAssertEqual(YourData.runPendingDelete(in: root)?.map(\.lastPathComponent), [YourData.pendingDeleteName])
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))

        try YourData.scheduleDelete(entries, in: root, pid: 4242)
        XCTAssertEqual(YourData.runPendingDelete(in: root, isRunning: { $0 != 4242 }), [])
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: root.path), ["models"])
        XCTAssertFalse(fm.fileExists(atPath: media.path))
        // Nothing pending on a normal launch.
        XCTAssertNil(YourData.runPendingDelete(in: root))
    }

    func test_spend_line_lists_providers_largest_first_and_hides_at_zero() {
        XCTAssertNil(YourData.spendLine(nil))
        XCTAssertNil(YourData.spendLine(CoreUsageTotals(todayCents: 0, last30DaysCents: 0, monthToDateCents: 0, last30DaysByProvider: [])))
        let totals = CoreUsageTotals(todayCents: 5, last30DaysCents: 150, monthToDateCents: 150, last30DaysByProvider: [
            .init(provider: "openai", cents: 25), .init(provider: "anthropic", cents: 125), .init(provider: "ollama", cents: 0),
        ])
        XCTAssertEqual(YourData.spendLine(totals), "Last 30 days: $1.50 in hosted model calls (Anthropic $1.25, OpenAI $0.25).")
    }

    func test_status_badge_precedence_and_the_capture_dot() {
        XCTAssertNil(StatusIcon.badgeColor(active: false, degraded: true, meetingRecording: true, activityCapture: true))
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: true, meetingRecording: true, activityCapture: true), .orange)
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: false, meetingRecording: true, activityCapture: true), .red)
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: false, meetingRecording: false, activityCapture: true), .secondary)
        XCTAssertNil(StatusIcon.badgeColor(active: true, degraded: false, meetingRecording: false, activityCapture: false))
    }
}
