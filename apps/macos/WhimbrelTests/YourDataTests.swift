import Security
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
        defer { try? fm.removeItem(at: root) }
        try fm.createDirectory(at: root.appendingPathComponent("models"), withIntermediateDirectories: true)
        try fm.createDirectory(at: root.appendingPathComponent("snapshots"), withIntermediateDirectories: true)
        try "x".write(to: root.appendingPathComponent("core-memory.sqlite"), atomically: true, encoding: .utf8)

        // Scheduling removes nothing: the running app still has the database open.
        let entries = YourData.entriesToDelete(in: root)
        try YourData.scheduleDelete(entries, in: root, pid: 4242)
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
        let log = StepLog()
        let steps = log.steps(removeDatabaseFiles: CoreDeleteEverything.removeDatabaseFiles)

        // The old instance has not quit in time: nothing is removed, all of it is reported,
        // and the note is gone so a later launch can't delete data made after this one. No key
        // and no flag is touched.
        let late = YourData.runPendingDelete(in: root, steps: steps, timeout: 0, isRunning: { _ in true })
        XCTAssertEqual(late?.notRemoved.map(\.path), entries.map(\.path))
        XCTAssertEqual(late?.keysDeleted, false)
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
        XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent(YourData.pendingDeleteName).path))

        // An unreadable note is reported, not treated as nothing pending, and is left.
        try "garbage".write(to: root.appendingPathComponent(YourData.pendingDeleteName), atomically: true, encoding: .utf8)
        XCTAssertEqual(YourData.runPendingDelete(in: root, steps: steps)?.notRemoved.map(\.lastPathComponent), [YourData.pendingDeleteName])
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent(YourData.pendingDeleteName).path))
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
        XCTAssertEqual(log.events, [], "no file, key or flag step ran")

        try YourData.scheduleDelete(entries, in: root, pid: 4242)
        let done = YourData.runPendingDelete(in: root, steps: steps, isRunning: { $0 != 4242 })
        XCTAssertEqual(done?.notRemoved, [])
        XCTAssertEqual(done?.keysDeleted, true)
        XCTAssertEqual(done?.incomplete, false)
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: root.path), ["models"])
        // Nothing pending on a normal launch.
        XCTAssertNil(YourData.runPendingDelete(in: root, steps: steps))
    }

    /// Records the order of Delete everything's steps; each answer is injectable.
    private final class StepLog {
        var events: [String] = []
        var remaining: [URL] = []
        var keysNotRemoved: [String] = []
        var markerCleared = true

        func steps(removeDatabaseFiles: ((URL) -> [URL])? = nil) -> YourData.DeleteEverythingSteps {
            YourData.DeleteEverythingSteps(
                removeDatabaseFiles: { db in
                    self.events.append("files:\(db.lastPathComponent)")
                    return removeDatabaseFiles?(db) ?? self.remaining
                },
                deleteKeys: { self.events.append("keys"); return self.keysNotRemoved },
                clearLaunchFlags: { self.events.append("flags") },
                clearStoreMarker: { self.events.append("marker"); return self.markerCleared },
                markFreshLibrary: { self.events.append("fresh") })
        }
    }

    private func pendingRoot() throws -> URL {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("yourdata-order-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: root.appendingPathComponent("models"), withIntermediateDirectories: true)
        for name in ["core-memory.sqlite", "core-memory.sqlite.plain.bak", "skins.sqlite"] {
            try "x".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8)
        }
        try YourData.scheduleDelete(YourData.entriesToDelete(in: root), in: root, pid: 4242)
        return root
    }

    /// X4 Task 12: the database files go BEFORE the keys, and the keys and launch flags go only
    /// once no database file is left.
    func test_delete_everything_removes_the_database_files_before_the_keys() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let log = StepLog()
        let result = YourData.runPendingDelete(in: root, steps: log.steps(), isRunning: { _ in false })
        XCTAssertEqual(log.events, ["files:core-memory.sqlite", "keys", "flags", "marker", "fresh"])
        XCTAssertEqual(result?.keysDeleted, true)
        XCTAssertEqual(result?.incomplete, false)
    }

    /// A database file that will not go: the keys and flags stay (an encrypted file whose key
    /// is gone can never be opened), the rest of the data still goes, and the result names the
    /// file so the user is told.
    func test_a_database_file_that_will_not_go_keeps_the_keys_and_is_named() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let log = StepLog()
        let stuck = root.appendingPathComponent("core-memory.sqlite.plain.bak")
        log.remaining = [stuck]
        let result = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps(), isRunning: { _ in false }))
        XCTAssertEqual(log.events, ["files:core-memory.sqlite"], "no key and no flag was touched")
        XCTAssertFalse(result.keysDeleted)
        XCTAssertEqual(result.databaseFilesRemaining, [stuck])
        XCTAssertTrue(result.notRemoved.contains(stuck))
        XCTAssertTrue(result.incomplete)
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("skins.sqlite").path), "the rest still goes")
        let message = try XCTUnwrap(YourData.deleteIncompleteMessage(result))
        XCTAssertTrue(message.contains(stuck.path), message)
    }

    /// Key slots that would not go are reported (by account name, never key bytes).
    func test_key_slots_that_will_not_go_are_reported() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let log = StepLog()
        log.keysNotRemoved = ["db-key-previous-2"]
        let result = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps(), isRunning: { _ in false }))
        XCTAssertEqual(log.events, ["files:core-memory.sqlite", "keys", "flags", "marker", "fresh"])
        XCTAssertEqual(result.keysNotRemoved, ["db-key-previous-2"])
        XCTAssertTrue(result.incomplete)
        let message = try XCTUnwrap(YourData.deleteIncompleteMessage(result))
        XCTAssertTrue(message.contains("db-key-previous-2"), message)
        XCTAssertNil(YourData.deleteIncompleteMessage(.init(notRemoved: [], databaseFilesRemaining: [], keysNotRemoved: [], keysDeleted: true)))
    }

    /// The whole flow on the live pieces (core's file removal, `DbKeyStore` on a throwaway
    /// service, a throwaway defaults suite): db-key, db-key-pending, every db-key-previous* slot
    /// and the launch flags are gone with the files.
    func test_delete_everything_removes_db_key() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let keys = DbKeyStore(service: "your-data-tests.\(UUID().uuidString)")
        _ = try keys.create()
        _ = try keys.createPending()
        for slot in [1, 2, DbKeyStore.maxPreviousSlots] {
            XCTAssertEqual(KeychainAccess.add([kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: keys.service,
                                               kSecAttrAccount as String: DbKeyStore.previousAccount(slot: slot),
                                               kSecValueData as String: Data(repeating: UInt8(slot), count: 32)] as CFDictionary,
                                              nil), errSecSuccess)
        }
        let suite = "your-data-tests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = UserDefaultsLaunchSettings(defaults: defaults)
        settings.migrateAtNextLaunch = true
        settings.goodLaunches = 2
        defaults.set(true, forKey: UserDefaultsLaunchSettings.fastSearchKey)

        let result = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: .live(keys: keys, defaults: defaults),
                                                            isRunning: { _ in false }))
        XCTAssertEqual(result.notRemoved, [])
        XCTAssertTrue(result.keysDeleted)
        XCTAssertEqual(keys.lookup(), .absent)
        XCTAssertEqual(keys.lookupPending(), .absent)
        XCTAssertEqual(keys.lookupPrevious(), [])
        XCTAssertFalse(settings.migrateAtNextLaunch)
        XCTAssertEqual(settings.goodLaunches, 0)
        XCTAssertTrue(defaults.bool(forKey: UserDefaultsLaunchSettings.fastSearchKey), "a preference, not a launch flag")
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["models"])
    }

    /// Review fix 1: a delete that never ran (the old instance still running, or the note
    /// unreadable) is said at once at the launch, not only in the pane; the keys and flags stay.
    func test_a_delete_that_never_ran_is_said_at_launch_and_keeps_the_keys() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let log = StepLog()
        let late = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps(), timeout: 0, isRunning: { _ in true }))
        XCTAssertEqual(log.events, [], "no file, key or flag step ran")
        XCTAssertTrue(late.didNotRun && !late.keysDeleted && late.incomplete)
        let message = try XCTUnwrap(YourData.deleteIncompleteMessage(late))
        XCTAssertTrue(message.contains(YourData.deleteDidNotRunMessage), message)
        try "garbage".write(to: root.appendingPathComponent(YourData.pendingDeleteName), atomically: true, encoding: .utf8)
        let unreadable = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps()))
        XCTAssertTrue(unreadable.didNotRun)
        XCTAssertNotNil(YourData.deleteIncompleteMessage(unreadable))
        XCTAssertEqual(log.events, [])
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
    }

    // MARK: The note's entries are confined (the note sits in the data folder; anything could write it)

    private func writeNote(_ paths: [String], in root: URL) throws {
        let json = try JSONSerialization.data(withJSONObject: ["pid": 4242, "paths": paths] as [String: Any])
        try json.write(to: root.appendingPathComponent(YourData.pendingDeleteName))
    }

    /// Entries outside the data root (absolute outside, a sibling sharing the root's prefix,
    /// `..`, relative, nested, through a link out) are never deleted and are named in the alert;
    /// the legitimate ones still go; a link in the root is removed itself, never followed.
    func test_note_entries_outside_the_data_folder_are_refused_and_named() throws {
        let fm = FileManager.default
        let base = fm.temporaryDirectory.appendingPathComponent("yourdata-confine-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: base) }
        let root = base.appendingPathComponent("data", isDirectory: true)
        let evil = base.appendingPathComponent("data-evil", isDirectory: true)
        let outside = base.appendingPathComponent("outside", isDirectory: true)
        for d in [root, evil, outside] { try fm.createDirectory(at: d, withIntermediateDirectories: true) }
        for f in [root.appendingPathComponent("core-memory.sqlite"), root.appendingPathComponent("skins.sqlite"),
                  evil.appendingPathComponent("victim"), outside.appendingPathComponent("victim")] {
            try "x".write(to: f, atomically: true, encoding: .utf8)
        }
        try fm.createSymbolicLink(at: root.appendingPathComponent("linkdir"), withDestinationURL: outside)
        let refused = [outside.appendingPathComponent("victim").path,
                       evil.appendingPathComponent("victim").path,
                       root.path + "/../outside/victim",
                       "skins.sqlite",
                       root.appendingPathComponent("linkdir/victim").path]
        try writeNote([root.appendingPathComponent("core-memory.sqlite").path, root.appendingPathComponent("skins.sqlite").path,
                       root.appendingPathComponent("linkdir").path] + refused, in: root)
        let log = StepLog()
        let r = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps(removeDatabaseFiles: CoreDeleteEverything.removeDatabaseFiles),
                                                        isRunning: { _ in false }))
        XCTAssertEqual(r.rejected, refused)
        XCTAssertTrue(fm.fileExists(atPath: outside.appendingPathComponent("victim").path), "never followed out")
        XCTAssertTrue(fm.fileExists(atPath: evil.appendingPathComponent("victim").path), "sibling prefix untouched")
        XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent("skins.sqlite").path), "the legitimate entries still go")
        XCTAssertNil(try? fm.destinationOfSymbolicLink(atPath: root.appendingPathComponent("linkdir").path), "the link itself goes")
        XCTAssertTrue(r.keysDeleted, "no database file is left, so the keys go as before")
        XCTAssertTrue(r.incomplete)
        let message = try XCTUnwrap(YourData.deleteIncompleteMessage(r))
        for p in refused { XCTAssertTrue(message.contains(p), "\(p) not named in \(message)") }
        XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent(YourData.pendingDeleteName).path))
    }

    /// The note carries data-folder entries only: one naming the media folder (or the note
    /// itself, in any case) is refused as outside, never deleted.
    func test_a_note_naming_the_media_folder_or_itself_is_refused() throws {
        let fm = FileManager.default
        let base = fm.temporaryDirectory.appendingPathComponent("yourdata-media-note-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: base) }
        let root = base.appendingPathComponent("data", isDirectory: true)
        let audio = base.appendingPathComponent("media/Audio/x", isDirectory: true)
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        try fm.createDirectory(at: audio, withIntermediateDirectories: true)
        let selfName = root.appendingPathComponent(YourData.pendingDeleteName.uppercased()).path
        try writeNote([audio.path, selfName], in: root)
        let r = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: StepLog().steps(), isRunning: { _ in false }))
        XCTAssertEqual(r.rejected, [audio.path, selfName])
        XCTAssertTrue(fm.fileExists(atPath: audio.path))
        XCTAssertNil(YourData.confine(root.appendingPathComponent(YourData.pendingDeleteName).path, root: root))
    }

    // MARK: Recordings and slides go at confirm time, in the app (never through the note)

    private struct MediaFixture {
        let base: URL, home: URL, media: URL, audio: URL, snapshots: URL, outside: URL
    }

    private func mediaFixture() throws -> MediaFixture {
        let fm = FileManager.default
        let base = fm.temporaryDirectory.appendingPathComponent("yourdata-media-now-\(UUID().uuidString)", isDirectory: true)
        let home = base.appendingPathComponent("home", isDirectory: true)
        let media = home.appendingPathComponent("Pictures/Whimbrel", isDirectory: true)
        let outside = home.appendingPathComponent("Documents", isDirectory: true)
        try fm.createDirectory(at: media, withIntermediateDirectories: true)
        try fm.createDirectory(at: outside.appendingPathComponent("this-mac"), withIntermediateDirectories: true)
        try "x".write(to: outside.appendingPathComponent("this-mac/keep.txt"), atomically: true, encoding: .utf8)
        return MediaFixture(base: base, home: home, media: media, audio: media.appendingPathComponent("Audio/this-mac", isDirectory: true),
                            snapshots: media.appendingPathComponent("Snapshots", isDirectory: true), outside: outside)
    }

    func test_media_goes_at_confirm_time_and_a_link_entry_goes_as_a_link() throws {
        let fm = FileManager.default
        let f = try mediaFixture()
        defer { try? fm.removeItem(at: f.base) }
        try fm.createDirectory(at: f.audio, withIntermediateDirectories: true)
        try "x".write(to: f.audio.appendingPathComponent("mic-audio-1.caf"), atomically: true, encoding: .utf8)
        let meeting = try SnapshotStorage.create("2026-11-04 10.00 Standup", under: f.snapshots)
        let unmarked = f.snapshots.appendingPathComponent("My own folder", isDirectory: true)
        try fm.createDirectory(at: unmarked, withIntermediateDirectories: true)
        // A folder whose marker is a link is not Whimbrel's.
        let fake = f.snapshots.appendingPathComponent("Fake", isDirectory: true)
        try fm.createDirectory(at: fake, withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: fake.appendingPathComponent(SnapshotStorage.markerName),
                                  withDestinationURL: meeting.appendingPathComponent(SnapshotStorage.markerName))
        let r = YourData.deleteMediaNow(audio: f.audio, snapshots: f.snapshots, home: f.home)
        XCTAssertNil(r.refused)
        XCTAssertEqual(r.notRemoved, [])
        XCTAssertFalse(fm.fileExists(atPath: f.audio.path))
        XCTAssertFalse(fm.fileExists(atPath: meeting.path))
        XCTAssertTrue(fm.fileExists(atPath: unmarked.path) && fm.fileExists(atPath: fake.path))
        // This Mac's audio folder as a link out: the link goes, never what it points to.
        try fm.createSymbolicLink(at: f.audio, withDestinationURL: f.outside.appendingPathComponent("this-mac"))
        let link = YourData.deleteMediaNow(audio: f.audio, snapshots: f.snapshots, home: f.home)
        XCTAssertNil(link.refused)
        XCTAssertNil(try? fm.destinationOfSymbolicLink(atPath: f.audio.path))
        XCTAssertTrue(fm.fileExists(atPath: f.outside.appendingPathComponent("this-mac/keep.txt").path))
    }

    func test_media_delete_refuses_a_symlinked_audio_folder() throws {
        let fm = FileManager.default
        let f = try mediaFixture()
        defer { try? fm.removeItem(at: f.base) }
        try fm.createSymbolicLink(at: f.media.appendingPathComponent("Audio"), withDestinationURL: f.outside)
        let r = YourData.deleteMediaNow(audio: f.audio, snapshots: f.snapshots, home: f.home)
        XCTAssertNotNil(r.refused)
        XCTAssertTrue(fm.fileExists(atPath: f.outside.appendingPathComponent("this-mac/keep.txt").path))
        XCTAssertNotNil(YourData.mediaDeleteMessage(r))
    }

    func test_media_delete_refuses_a_symlinked_snapshots_folder() throws {
        let fm = FileManager.default
        let f = try mediaFixture()
        defer { try? fm.removeItem(at: f.base) }
        let victim = f.outside.appendingPathComponent("Meeting", isDirectory: true)
        try fm.createDirectory(at: victim, withIntermediateDirectories: true)
        try "x".write(to: victim.appendingPathComponent(SnapshotStorage.markerName), atomically: true, encoding: .utf8)
        try fm.createSymbolicLink(at: f.snapshots, withDestinationURL: f.outside)
        let r = YourData.deleteMediaNow(audio: f.audio, snapshots: f.snapshots, home: f.home)
        XCTAssertNotNil(r.refused)
        XCTAssertTrue(fm.fileExists(atPath: victim.path))
        // The media folder itself as a link: refused too.
        let real = f.base.appendingPathComponent("real-media", isDirectory: true)
        try fm.createDirectory(at: real.appendingPathComponent("Audio/this-mac"), withIntermediateDirectories: true)
        let linked = f.home.appendingPathComponent("Linked", isDirectory: true)
        try fm.createSymbolicLink(at: linked, withDestinationURL: real)
        let viaLink = YourData.deleteMediaNow(audio: linked.appendingPathComponent("Audio/this-mac"),
                                              snapshots: linked.appendingPathComponent("Snapshots"), home: f.home)
        XCTAssertNotNil(viaLink.refused)
        XCTAssertTrue(fm.fileExists(atPath: real.appendingPathComponent("Audio/this-mac").path))
    }

    /// Removal is by file descriptor and never follows a link: a link nested inside a meeting
    /// folder goes as the link, its target stays.
    func test_media_delete_removes_a_nested_link_as_a_link() throws {
        let fm = FileManager.default
        let f = try mediaFixture()
        defer { try? fm.removeItem(at: f.base) }
        let meeting = try SnapshotStorage.create("2026-11-04 10.00 Standup", under: f.snapshots)
        try fm.createSymbolicLink(at: meeting.appendingPathComponent("evil"), withDestinationURL: f.outside)
        try fm.createDirectory(at: f.audio, withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: f.audio.appendingPathComponent("evil"), withDestinationURL: f.outside.appendingPathComponent("this-mac"))
        let r = YourData.deleteMediaNow(audio: f.audio, snapshots: f.snapshots, home: f.home)
        XCTAssertNil(r.refused)
        XCTAssertEqual(r.notRemoved, [])
        XCTAssertFalse(fm.fileExists(atPath: meeting.path))
        XCTAssertFalse(fm.fileExists(atPath: f.audio.path))
        XCTAssertTrue(fm.fileExists(atPath: f.outside.appendingPathComponent("this-mac/keep.txt").path))
    }

    /// The race the lstat checks alone could lose: after the folders are opened, Snapshots and
    /// Audio are swapped for links to ~/Documents. The removal works on the folders it opened,
    /// so the link targets survive.
    func test_media_delete_survives_folders_swapped_for_links_after_open() throws {
        let fm = FileManager.default
        let f = try mediaFixture()
        defer { try? fm.removeItem(at: f.base) }
        let meeting = try SnapshotStorage.create("Standup", under: f.snapshots)
        try fm.createDirectory(at: f.audio, withIntermediateDirectories: true)
        // In ~/Documents: a folder named like the meeting (marked) and like this Mac's audio.
        let decoy = f.outside.appendingPathComponent(meeting.lastPathComponent, isDirectory: true)
        try fm.createDirectory(at: decoy, withIntermediateDirectories: true)
        try "x".write(to: decoy.appendingPathComponent(SnapshotStorage.markerName), atomically: true, encoding: .utf8)
        var swapError: Error?
        let r = YourData.deleteMediaNow(audio: f.audio, snapshots: f.snapshots, home: f.home, beforeRemove: {
            do {
                try fm.moveItem(at: f.snapshots, to: f.media.appendingPathComponent("Snapshots-moved"))
                try fm.createSymbolicLink(at: f.snapshots, withDestinationURL: f.outside)
                try fm.moveItem(at: f.audio.deletingLastPathComponent(), to: f.media.appendingPathComponent("Audio-moved"))
                try fm.createSymbolicLink(at: f.audio.deletingLastPathComponent(), withDestinationURL: f.outside)
            } catch { swapError = error }
        })
        XCTAssertNil(swapError, "the swap must really happen")
        // The links are still in place after the delete (it never removed or followed them).
        for linked in [f.snapshots, f.audio.deletingLastPathComponent()] {
            XCTAssertEqual(try fm.destinationOfSymbolicLink(atPath: linked.path), f.outside.path)
        }
        XCTAssertNil(r.refused)
        XCTAssertTrue(fm.fileExists(atPath: decoy.appendingPathComponent(SnapshotStorage.markerName).path), "Documents untouched")
        XCTAssertTrue(fm.fileExists(atPath: f.outside.appendingPathComponent("this-mac/keep.txt").path), "Documents untouched")
        XCTAssertFalse(fm.fileExists(atPath: f.media.appendingPathComponent("Snapshots-moved/\(meeting.lastPathComponent)").path),
                       "the folder that was opened is the one emptied")
    }

    /// Recordings and slides are gone before the restart, so no message may say nothing was deleted.
    func test_restart_and_did_not_run_messages_say_recordings_and_slides_are_gone() {
        for m in [YourData.restartMessages.failed, YourData.restartMessages.stuck, YourData.deleteDidNotRunMessage] {
            XCTAssertFalse(m.contains("nothing has been deleted yet") || m.contains("nothing was deleted"), m)
            // True also when the media delete was refused (nothing deleted then).
            XCTAssertTrue(m.contains("Your recordings and slides were deleted unless Your data says otherwise."), m)
        }
    }

    /// An entry that cannot be examined (its folder cannot be searched) is not reported as gone.
    func test_remove_tree_reports_an_entry_it_cannot_examine_as_not_removed() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("yourdata-noexec-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try "x".write(to: dir.appendingPathComponent("f"), atomically: true, encoding: .utf8)
        try fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: dir.path) // readable, not searchable
        defer {
            try? fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
            try? fm.removeItem(at: dir)
        }
        let fd = open(dir.path, O_RDONLY | O_DIRECTORY)
        XCTAssertGreaterThanOrEqual(fd, 0)
        defer { close(fd) }
        XCTAssertFalse(YourData.removeTreeNoFollow(dirfd: fd, name: "f"))
    }

    /// A media folder that is `/`, the home folder or a standard user folder is never deleted from.
    func test_media_delete_refuses_home_and_standard_folders() throws {
        let fm = FileManager.default
        let f = try mediaFixture()
        defer { try? fm.removeItem(at: f.base) }
        for name in ["", "Documents", "Desktop", "Downloads", "Music", "Pictures", "Movies"] {
            let folder = name.isEmpty ? f.home : f.home.appendingPathComponent(name, isDirectory: true)
            try fm.createDirectory(at: folder.appendingPathComponent("Audio/this-mac"), withIntermediateDirectories: true)
            let r = YourData.deleteMediaNow(audio: folder.appendingPathComponent("Audio/this-mac"),
                                            snapshots: folder.appendingPathComponent("Snapshots"), home: f.home)
            XCTAssertNotNil(r.refused, "media folder \(name.isEmpty ? "home" : name)")
            XCTAssertTrue(fm.fileExists(atPath: folder.appendingPathComponent("Audio/this-mac").path))
        }
        let root = YourData.deleteMediaNow(audio: URL(fileURLWithPath: "/Audio/this-mac"), snapshots: URL(fileURLWithPath: "/Snapshots"), home: f.home)
        XCTAssertNotNil(root.refused)
    }

    /// Only a missing note means nothing pending. One that cannot be read or decoded is the
    /// did-not-run path: nothing deleted, the note left, the alert names it.
    func test_an_unreadable_note_deletes_nothing_and_is_left_and_named() throws {
        let fm = FileManager.default
        let root = try pendingRoot()
        defer { try? fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: root.appendingPathComponent(YourData.pendingDeleteName).path)
                try? fm.removeItem(at: root) }
        let note = root.appendingPathComponent(YourData.pendingDeleteName)
        let log = StepLog()
        try "garbage".write(to: note, atomically: true, encoding: .utf8)
        let garbage = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps()))
        XCTAssertTrue(garbage.didNotRun)
        XCTAssertEqual(garbage.noteLeft?.path, note.path)
        XCTAssertTrue(fm.fileExists(atPath: note.path), "left, not removed")
        XCTAssertTrue(try XCTUnwrap(YourData.deleteIncompleteMessage(garbage)).contains(note.path))
        // Unreadable (no permission): not "nothing pending".
        try YourData.scheduleDelete(YourData.entriesToDelete(in: root), in: root, pid: 4242)
        try fm.setAttributes([.posixPermissions: 0o000], ofItemAtPath: note.path)
        let unreadable = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps(), isRunning: { _ in false }))
        XCTAssertTrue(unreadable.didNotRun)
        XCTAssertEqual(log.events, [], "nothing deleted")
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
    }

    /// A note that cannot be removed at the end is said in the alert (it would run again).
    func test_a_note_that_cannot_be_removed_is_named() throws {
        final class StickyNote: FileManager {
            override func removeItem(at url: URL) throws {
                if url.lastPathComponent == YourData.pendingDeleteName { throw CocoaError(.fileWriteNoPermission) }
                try super.removeItem(at: url)
            }
        }
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let r = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: StepLog().steps(), isRunning: { _ in false },
                                                        fileManager: StickyNote()))
        let note = root.appendingPathComponent(YourData.pendingDeleteName)
        XCTAssertEqual(r.noteLeft?.path, note.path)
        XCTAssertTrue(r.incomplete)
        XCTAssertTrue(try XCTUnwrap(YourData.deleteIncompleteMessage(r)).contains(note.path))
    }

    /// Review fix 2: the database Delete everything removes is the one the launch opens.
    func test_delete_everything_removes_the_database_the_launch_opens() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        var received: URL?
        let steps = YourData.DeleteEverythingSteps(removeDatabaseFiles: { received = $0; return [] },
                                                   deleteKeys: { [] }, clearLaunchFlags: {})
        _ = YourData.runPendingDelete(in: root, steps: steps, isRunning: { _ in false })
        let suite = "your-data-tests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let env = LaunchEnvironment(dataRoot: root, modelsDir: nil,
                                    keys: DbKeyStore(service: "your-data-tests.\(UUID().uuidString)"), defaults: defaults)
        XCTAssertEqual(received, env.dbPath)
        XCTAssertEqual(received, LaunchEnvironment.dbPath(in: root))
    }

    /// Review fix 3: a Delete and restart that cannot start, or a stuck old instance, says
    /// so in Delete everything's words, not the encryption restart's.
    func test_delete_and_restart_uses_its_own_messages() {
        var messages: [String] = []
        AppRestart.relaunchAfterExit(waitingAlsoFor: nil,
                                     helperDir: FileManager.default.temporaryDirectory.appendingPathComponent("missing-\(UUID().uuidString)/dir"),
                                     messages: YourData.restartMessages,
                                     run: { _ in }, showMessage: { messages.append($0) }, terminate: {})
        XCTAssertEqual(messages, [YourData.restartMessages.failed])
        XCTAssertNotEqual(YourData.restartMessages.failed, EncryptionCopy.restartFailed)
        var arguments: [String] = []
        AppRestart.relaunchAfterExit(waitingAlsoFor: nil, messages: YourData.restartMessages,
                                     run: { arguments = $0.arguments ?? [] }, showMessage: { messages.append($0) }, terminate: {})
        XCTAssertTrue(arguments.contains(YourData.restartMessages.stuck), "\(arguments)")
        XCTAssertFalse(arguments.contains(EncryptionCopy.restartStuck))
    }

    /// The live pieces with a database file that really will not go (a locked folder under a
    /// database name): the key stays and the file is named.
    func test_delete_everything_keeps_db_key_when_a_database_file_will_not_go() throws {
        let fm = FileManager.default
        let root = try pendingRoot()
        let stuck = root.appendingPathComponent("core-memory.sqlite.foreign-1", isDirectory: true)
        try fm.createDirectory(at: stuck, withIntermediateDirectories: true)
        try "x".write(to: stuck.appendingPathComponent("inner"), atomically: true, encoding: .utf8)
        try fm.setAttributes([.posixPermissions: 0o500], ofItemAtPath: stuck.path)
        defer {
            try? fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: stuck.path)
            try? fm.removeItem(at: root)
        }
        let keys = DbKeyStore(service: "your-data-tests.\(UUID().uuidString)")
        let key = try keys.create()
        let suite = "your-data-tests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        UserDefaultsLaunchSettings(defaults: defaults).goodLaunches = 2
        // The note lists the folder too: the shell's own removal of it fails the same way.
        try YourData.scheduleDelete(YourData.entriesToDelete(in: root), in: root, pid: 4242)

        let result = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: .live(keys: keys, defaults: defaults),
                                                            isRunning: { _ in false }))
        XCTAssertFalse(result.keysDeleted)
        XCTAssertEqual(result.databaseFilesRemaining.map(\.lastPathComponent), ["core-memory.sqlite.foreign-1"])
        XCTAssertEqual(keys.lookup(), .found(key), "the key of a file that remains is kept")
        XCTAssertEqual(UserDefaultsLaunchSettings(defaults: defaults).goodLaunches, 2, "flags kept")
        XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent("core-memory.sqlite").path))
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
        // A recording keeps its red badge even with a problem; the problem gets its own mark.
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: true, meetingRecording: true, activityCapture: true), .red)
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: true, meetingRecording: false, activityCapture: true), .orange)
        XCTAssertTrue(StatusIcon.showsProblemMark(degraded: true, meetingRecording: true))
        XCTAssertFalse(StatusIcon.showsProblemMark(degraded: true, meetingRecording: false))
        XCTAssertFalse(StatusIcon.showsProblemMark(degraded: false, meetingRecording: true))
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: false, meetingRecording: true, activityCapture: true), .red)
        XCTAssertEqual(StatusIcon.badgeColor(active: true, degraded: false, meetingRecording: false, activityCapture: true), .secondary)
        XCTAssertNil(StatusIcon.badgeColor(active: true, degraded: false, meetingRecording: false, activityCapture: false))
    }

    // MARK: - Task 13c

    private func suite() throws -> (UserDefaults, () -> Void) {
        let name = "your-data-13c.\(UUID().uuidString)"
        let d = try XCTUnwrap(UserDefaults(suiteName: name))
        return (d, { d.removePersistentDomain(forName: name) })
    }

    /// Item 1: Delete everything's live steps leave the one-shot "fresh library" marker, which
    /// is read once and then gone. A normal install has none.
    func test_the_fresh_library_marker_is_left_by_delete_everything_and_read_once() throws {
        let (defaults, cleanup) = try suite()
        defer { cleanup() }
        XCTAssertFalse(FreshLibraryMarker.consume(defaults), "a normal new install shows no line")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("yourdata-13c-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let steps = YourData.DeleteEverythingSteps.live(root: root, keys: DbKeyStore(service: "yourdata-13c.\(UUID().uuidString)"),
                                                        defaults: defaults)
        steps.markFreshLibrary()
        XCTAssertTrue(FreshLibraryMarker.consume(defaults))
        XCTAssertFalse(FreshLibraryMarker.consume(defaults), "consumed")
    }

    /// Item 4: Delete everything's flag cleanup removes the "a store was opened here" marker
    /// with the other launch flags, so the next launch shows no "library not found" notice.
    func test_delete_everythings_flag_cleanup_removes_the_store_marker() throws {
        let (defaults, cleanup) = try suite()
        defer { cleanup() }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("yourdata-13c-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let db = LaunchEnvironment.dbPath(in: root)
        XCTAssertTrue(CoreLaunch.markStoreExisted(dbPath: db))
        let marker = root.appendingPathComponent("store_existed")
        XCTAssertTrue(FileManager.default.fileExists(atPath: marker.path))
        defaults.set(true, forKey: UserDefaultsLaunchSettings.migrateAtNextLaunchKey)
        defaults.set(2, forKey: UserDefaultsLaunchSettings.goodLaunchesKey)
        let steps = YourData.DeleteEverythingSteps.live(root: root, keys: DbKeyStore(service: "yourdata-13c.\(UUID().uuidString)"),
                                                        defaults: defaults)
        steps.clearLaunchFlags()
        XCTAssertNil(defaults.object(forKey: UserDefaultsLaunchSettings.migrateAtNextLaunchKey))
        XCTAssertNil(defaults.object(forKey: UserDefaultsLaunchSettings.goodLaunchesKey))
        XCTAssertTrue(steps.clearStoreMarker())
        XCTAssertFalse(FileManager.default.fileExists(atPath: marker.path))
        XCTAssertTrue(steps.clearStoreMarker(), "already gone counts as cleared")
    }

    /// Fix (D): a store marker that will not go is reported like the other leftovers: the
    /// result is incomplete and the launch's alert names the file (else the next launch
    /// would say the library could not be found right after Delete everything).
    func test_a_store_marker_that_will_not_go_is_reported() throws {
        let root = try pendingRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let log = StepLog()
        log.markerCleared = false
        let result = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: log.steps(), isRunning: { _ in false }))
        XCTAssertEqual(log.events, ["files:core-memory.sqlite", "keys", "flags", "marker", "fresh"])
        let marker = root.appendingPathComponent("store_existed")
        XCTAssertEqual(result.storeMarkerLeft?.path, marker.path)
        XCTAssertTrue(result.incomplete)
        let message = try XCTUnwrap(YourData.deleteIncompleteMessage(result))
        XCTAssertTrue(message.contains(EncryptionCopy.storeMarkerNotRemoved), message)
        XCTAssertTrue(message.contains(marker.path), message)
    }

    /// Item 3: the set-aside files over the real C ABI, and the row's text (hidden when none).
    func test_foreign_files_row_lists_and_deletes_only_the_set_aside_files() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("yourdata-13c-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? fm.removeItem(at: root) }
        let db = LaunchEnvironment.dbPath(in: root)
        XCTAssertEqual(CoreForeignFiles.list(dbPath: db), [])
        XCTAssertNil(YourData.foreignFilesSummary([]), "no files: the row is hidden")
        try Data(count: 3000).write(to: root.appendingPathComponent("core-memory.sqlite.foreign-1700000000000"))
        try Data(count: 100).write(to: root.appendingPathComponent("core-memory.sqlite.foreign-1700000000000-wal"))
        for keep in ["core-memory.sqlite", "core-memory.sqlite.plain-journal-5", "core-memory.sqlite.foreign-12x"] {
            try Data("x".utf8).write(to: root.appendingPathComponent(keep))
        }
        let files = CoreForeignFiles.list(dbPath: db)
        XCTAssertEqual(files.map(\.name), ["core-memory.sqlite.foreign-1700000000000", "core-memory.sqlite.foreign-1700000000000-wal"])
        XCTAssertEqual(files.map(\.size), [3000, 100])
        XCTAssertEqual(YourData.foreignFilesSummary(files),
                       EncryptionCopy.foreignFilesSummary(count: 2, size: YourData.formatBytes(3100)))
        XCTAssertEqual(YourData.foreignFilesConfirmBody(files),
                       EncryptionCopy.foreignFilesConfirmBody(count: 2, size: YourData.formatBytes(3100)))
        let result = try XCTUnwrap(CoreForeignFiles.deleteAll(dbPath: db))
        XCTAssertEqual(result.removed.count, 2)
        XCTAssertEqual(result.failed, [])
        XCTAssertNil(YourData.foreignFilesDeleteMessage(result))
        XCTAssertEqual(CoreForeignFiles.list(dbPath: db), [])
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: root.path).sorted(),
                       ["core-memory.sqlite", "core-memory.sqlite.foreign-12x", "core-memory.sqlite.plain-journal-5"])
        let partial = CoreForeignFiles.DeleteResult(removed: [], failed: ["core-memory.sqlite.foreign-2"])
        XCTAssertEqual(YourData.foreignFilesDeleteMessage(partial), EncryptionCopy.foreignFilesNotDeleted + "\ncore-memory.sqlite.foreign-2")
        XCTAssertEqual(YourData.foreignFilesDeleteMessage(nil), EncryptionCopy.foreignFilesUnreadable)
    }
}

/// Task 15 (Mike 2026-11-17): connected Google accounts stay connected through Delete
/// everything. The confirm step writes `.kept-google-accounts.json`, the delete keeps it, and
/// the first open of the new database re-adds each account whose sign-in is still stored.
@MainActor
final class KeptGoogleAccountsTests: XCTestCase {
    private let fm = FileManager.default
    private var root: URL!
    private var defaults: UserDefaults!
    private let suiteName = "KeptGoogleAccountsTests"

    override func setUp() {
        super.setUp()
        root = fm.temporaryDirectory.appendingPathComponent("kept-\(UUID().uuidString)", isDirectory: true)
        try! fm.createDirectory(at: root, withIntermediateDirectories: true)
        defaults = UserDefaults(suiteName: suiteName)
        defaults.removePersistentDomain(forName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        try? fm.removeItem(at: root)
        super.tearDown()
    }

    private var file: URL { root.appendingPathComponent(KeptGoogleAccounts.fileName) }

    private func makeStore() -> CoreMemoryStore {
        let dir = root.appendingPathComponent("store-\(UUID().uuidString)", isDirectory: true)
        try! fm.createDirectory(at: dir, withIntermediateDirectories: true)
        return CoreMemoryStore(path: dir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    private let sample: [String: KeptGoogleAccounts.Connector] = [
        "gmail": .init(emails: ["a@example.test", "b@example.test"], defaultEmail: "b@example.test"),
        "calendar": .init(emails: ["c@example.test", "d@example.test"], defaultEmail: "d@example.test"),
    ]

    private func mode(_ url: URL) throws -> Int {
        try XCTUnwrap(fm.attributesOfItem(atPath: url.path)[.posixPermissions] as? NSNumber).intValue & 0o777
    }

    func test_write_captures_both_connectors_emails_and_defaults_at_mode_0600() throws {
        try KeptGoogleAccounts.write(sample, in: root)
        let contents = try JSONDecoder().decode(KeptGoogleAccounts.Contents.self, from: Data(contentsOf: file))
        XCTAssertEqual(contents.connectors, sample)
        XCTAssertEqual(contents.version, 1)
        XCTAssertEqual(try mode(file), 0o600)
        XCTAssertEqual(try fm.contentsOfDirectory(atPath: root.path), [KeptGoogleAccounts.fileName], "no temporary file left")
        // Overwriting keeps 0600 (a fresh 0600 file renamed over the old one).
        try KeptGoogleAccounts.write(["gmail": .init(emails: ["a@example.test"], defaultEmail: "a@example.test")], in: root)
        XCTAssertEqual(try mode(file), 0o600)
    }

    func test_write_with_no_google_account_removes_an_earlier_list_and_ignores_outlook() throws {
        try KeptGoogleAccounts.write(sample, in: root)
        try KeptGoogleAccounts.write(["gmail": .init(emails: [], defaultEmail: nil),
                                      "outlook365": .init(emails: ["o@example.test"], defaultEmail: "o@example.test")], in: root)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
    }

    /// Fix round 1 (Mike 2026-11-17): a list that cannot be written stops the delete
    /// (`.failed`, the pane returns before anything is scheduled).
    func test_a_failed_write_is_reported_as_failed() throws {
        struct Boom: Error {}
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(sample, in: root, write: { _, _ in throw Boom() }), .failed)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
        // A real failure: the folder is not there.
        let missing = root.appendingPathComponent("missing", isDirectory: true)
        XCTAssertThrowsError(try KeptGoogleAccounts.write(sample, in: missing))
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(sample, in: missing), .failed)
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(sample, in: root), .wrote)
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        // With a store open and really no Google account, an earlier list goes.
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(["gmail": .init(emails: [], defaultEmail: nil)], in: root), .nothingToKeep)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
    }

    /// No store open: an earlier list (its restore has not run) is never overwritten or
    /// removed, whatever the Keychain holds.
    func test_without_a_store_an_existing_list_is_left_untouched() throws {
        try KeptGoogleAccounts.write(sample, in: root)
        let before = try Data(contentsOf: file)
        var asked = false
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: { asked = true; return [] }), .keptExisting)
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: { ["gmail-connector:z@example.test"] }), .keptExisting)
        XCTAssertFalse(asked)
        XCTAssertEqual(try Data(contentsOf: file), before)
        XCTAssertEqual(try mode(file), 0o600)
    }

    /// No store open and no earlier list: rebuilt from the Keychain's Google OAuth account
    /// names (attributes only), without a default; nothing in the Keychain writes nothing.
    func test_without_a_store_the_list_is_rebuilt_from_the_keychain() throws {
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: { [] }), .nothingToKeep)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
        let names = ["gmail-connector:b@example.test", "gmail-connector:a@example.test", "gmail-connector:a@example.test#issuer",
                     "gmail-connector", "google-calendar-connector", "google-calendar-connector:c@example.test",
                     "sync-sign-in:d@example.test"]
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: { names }), .wrote)
        let contents = try JSONDecoder().decode(KeptGoogleAccounts.Contents.self, from: Data(contentsOf: file))
        XCTAssertEqual(contents.connectors, ["gmail": .init(emails: ["a@example.test", "b@example.test"], defaultEmail: nil),
                                             "calendar": .init(emails: ["c@example.test"], defaultEmail: nil)])
        XCTAssertEqual(try mode(file), 0o600)
        // The restore makes the first account the default.
        try fm.removeItem(at: file)
        try KeptGoogleAccounts.write(contents.connectors, in: root)
        let store = makeStore()
        XCTAssertNotNil(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults))
        XCTAssertEqual(ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults).defaultEmail, "a@example.test")
    }

    /// The live listing goes through `KeychainAccess` (the test host's in-memory Keychain)
    /// with attributes only: the in-memory store answers a listing only when no data is asked.
    func test_the_keychain_listing_reads_account_names_only() throws {
        let email = "list-\(UUID().uuidString)@example.test"
        var queries: [[String: Any]] = []
        for account in ["gmail-connector:\(email)", "gmail-connector:\(email)#issuer", "google-calendar-connector:\(email)"] {
            let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                    kSecAttrService as String: GoogleOAuthHelper.keychainService,
                                    kSecAttrAccount as String: account]
            XCTAssertEqual(KeychainAccess.upsert(q, data: Data("secret-token".utf8)), errSecSuccess)
            queries.append(q)
        }
        defer { for q in queries { KeychainAccess.delete(q as CFDictionary) } }
        let names = try XCTUnwrap(KeptGoogleAccounts.googleOAuthAccountNames())
        XCTAssertTrue(names.contains("gmail-connector:\(email)"))
        XCTAssertTrue(names.contains("google-calendar-connector:\(email)"))
        XCTAssertFalse(names.contains { $0.contains("secret-token") })
        let rebuilt = KeptGoogleAccounts.connectors(fromKeychainAccounts: names)
        XCTAssertTrue(rebuilt["gmail"]?.emails.contains(email) == true)
        XCTAssertTrue(rebuilt["calendar"]?.emails.contains(email) == true)
        XCTAssertFalse(rebuilt.values.contains { $0.emails.contains { $0.hasSuffix("#issuer") } })
    }

    /// A Keychain that cannot be listed is a failed keep (the delete stops, nothing is
    /// written or deleted); no Google item at all is nothing to keep.
    func test_a_keychain_listing_error_stops_the_delete() throws {
        try "x".write(to: root.appendingPathComponent("skins.sqlite"), atomically: true, encoding: .utf8)
        let refused = { (status: OSStatus) in
            { KeptGoogleAccounts.googleOAuthAccountNames(copyMatching: { _, _ in status }) }
        }
        XCTAssertNil(refused(errSecInteractionNotAllowed)())
        XCTAssertEqual(refused(errSecItemNotFound)(), [])
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: refused(errSecInteractionNotAllowed)), .failed)
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: refused(errSecAuthFailed)), .failed)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
        XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent(YourData.pendingDeleteName).path), "nothing scheduled")
        XCTAssertTrue(fm.fileExists(atPath: root.appendingPathComponent("skins.sqlite").path), "nothing deleted")
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: refused(errSecItemNotFound)), .nothingToKeep)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
    }

    /// A second Delete everything before the restore ran (no store opened in between) keeps
    /// the first list.
    func test_a_second_delete_everything_before_the_restore_keeps_the_list() throws {
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(sample, in: root), .wrote)
        let steps = YourData.DeleteEverythingSteps(removeDatabaseFiles: CoreDeleteEverything.removeDatabaseFiles,
                                                   deleteKeys: { [] }, clearLaunchFlags: {})
        try YourData.scheduleDelete(YourData.entriesToDelete(in: root), in: root, pid: 4242)
        XCTAssertEqual(YourData.runPendingDelete(in: root, steps: steps, isRunning: { _ in false })?.incomplete, false)
        // The next launch opened no store; Delete everything again.
        XCTAssertEqual(KeptGoogleAccounts.keepForDelete(nil, in: root, keychainAccounts: { [] }), .keptExisting)
        try YourData.scheduleDelete(YourData.entriesToDelete(in: root), in: root, pid: 4242)
        XCTAssertEqual(YourData.runPendingDelete(in: root, steps: steps, isRunning: { _ in false })?.incomplete, false)
        XCTAssertEqual(try JSONDecoder().decode(KeptGoogleAccounts.Contents.self, from: Data(contentsOf: file)).connectors, sample)
    }

    /// Fix round 1 C4: accounts already there, one more kept: the kept default wins.
    func test_restore_sets_the_kept_default_over_an_existing_one() throws {
        let store = makeStore()
        let gmail = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        gmail.addDuringInit("a@example.test"); gmail.addDuringInit("b@example.test")
        XCTAssertEqual(gmail.defaultEmail, "a@example.test")
        try KeptGoogleAccounts.write(["gmail": .init(emails: ["a@example.test", "b@example.test", "c@example.test"],
                                                     defaultEmail: "b@example.test")], in: root)
        XCTAssertNotNil(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults))
        let after = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(after.emails, ["a@example.test", "b@example.test", "c@example.test"])
        XCTAssertEqual(after.defaultEmail, "b@example.test")
    }

    /// Fix round 1 C4 (coordinator ruling): every account already there (a crash after the
    /// adds, before the default was set): the kept default is still applied.
    func test_restore_applies_the_kept_default_when_every_account_is_already_there() throws {
        let store = makeStore()
        let gmail = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        gmail.addDuringInit("a@example.test"); gmail.addDuringInit("b@example.test")
        XCTAssertEqual(gmail.defaultEmail, "a@example.test")
        try KeptGoogleAccounts.write(["gmail": .init(emails: ["a@example.test", "b@example.test"], defaultEmail: "b@example.test")], in: root)
        let r = try XCTUnwrap(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults))
        XCTAssertEqual(r.restored, 2)
        let after = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(after.emails, ["a@example.test", "b@example.test"])
        XCTAssertEqual(after.defaultEmail, "b@example.test")
    }

    /// The file is not in the delete set, survives the launch delete even when a note names
    /// it, and is neither a database file nor a set-aside (foreign) file.
    func test_the_list_survives_delete_everything() throws {
        try fm.createDirectory(at: root.appendingPathComponent("models"), withIntermediateDirectories: true)
        for name in ["core-memory.sqlite", "core-memory.sqlite-wal", "skins.sqlite"] {
            try "x".write(to: root.appendingPathComponent(name), atomically: true, encoding: .utf8)
        }
        try KeptGoogleAccounts.write(sample, in: root)
        let entries = YourData.entriesToDelete(in: root)
        XCTAssertFalse(entries.map(\.lastPathComponent).contains(KeptGoogleAccounts.fileName))
        XCTAssertTrue(entries.map(\.lastPathComponent).contains("skins.sqlite"))
        XCTAssertEqual(CoreForeignFiles.list(dbPath: LaunchEnvironment.dbPath(in: root)), [])
        // A note that names the list anyway (any case) still leaves it, and does not report it.
        try YourData.scheduleDelete(entries + [file, root.appendingPathComponent(KeptGoogleAccounts.fileName.uppercased())],
                                    in: root, pid: 4242)
        let steps = YourData.DeleteEverythingSteps(removeDatabaseFiles: CoreDeleteEverything.removeDatabaseFiles,
                                                   deleteKeys: { [] }, clearLaunchFlags: {})
        let r = try XCTUnwrap(YourData.runPendingDelete(in: root, steps: steps, isRunning: { _ in false }))
        XCTAssertFalse(r.incomplete, "\(r)")
        XCTAssertEqual(r.rejected, [])
        XCTAssertTrue(fm.fileExists(atPath: file.path))
        for gone in ["core-memory.sqlite", "core-memory.sqlite-wal", "skins.sqlite"] {
            XCTAssertFalse(fm.fileExists(atPath: root.appendingPathComponent(gone).path), gone)
        }
        XCTAssertEqual(try JSONDecoder().decode(KeptGoogleAccounts.Contents.self, from: Data(contentsOf: file)).connectors, sample)
        XCTAssertEqual(try mode(file), 0o600)
    }

    func test_restore_re_adds_only_accounts_with_a_stored_sign_in_and_sets_the_default() throws {
        try KeptGoogleAccounts.write(sample, in: root)
        let store = makeStore()
        let signedIn: Set<String> = ["gmail a@example.test", "gmail b@example.test", "calendar c@example.test"]
        let r = try XCTUnwrap(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { signedIn.contains("\($0) \($1)") },
                                                         defaults: defaults))
        XCTAssertEqual(r, .init(restored: 3, dropped: 1, unreadable: false, fileLeft: false))
        let gmail = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(gmail.emails, ["a@example.test", "b@example.test"])
        XCTAssertEqual(gmail.defaultEmail, "b@example.test", "the kept default, not the first added")
        let calendar = ConnectedAccountStore(connectorId: "calendar", store: store, defaults: defaults)
        XCTAssertEqual(calendar.emails, ["c@example.test"])
        XCTAssertEqual(calendar.defaultEmail, "c@example.test", "the kept default has no sign-in: the first restored one")
        XCTAssertFalse(fm.fileExists(atPath: file.path), "removed after the restore")
    }

    /// A crash between the adds and the file's removal repeats the restore: nothing doubles.
    func test_restore_twice_is_a_no_op() throws {
        let store = makeStore()
        try KeptGoogleAccounts.write(sample, in: root)
        XCTAssertNotNil(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults))
        try KeptGoogleAccounts.write(sample, in: root)
        let again = try XCTUnwrap(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults))
        XCTAssertEqual(again.restored, 4)
        XCTAssertNil(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults), "no file: nothing to do")
        let gmail = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(gmail.emails, ["a@example.test", "b@example.test"])
        XCTAssertEqual(gmail.defaultEmail, "b@example.test")
        XCTAssertEqual(ConnectedAccountStore(connectorId: "calendar", store: store, defaults: defaults).emails,
                       ["c@example.test", "d@example.test"])
    }

    func test_a_corrupt_list_is_removed_and_the_launch_goes_on() throws {
        try "{not json".write(to: file, atomically: true, encoding: .utf8)
        let store = makeStore()
        let r = try XCTUnwrap(KeptGoogleAccounts.restore(in: root, store: store, hasSignIn: { _, _ in true }, defaults: defaults))
        XCTAssertTrue(r.unreadable)
        XCTAssertFalse(r.fileLeft)
        XCTAssertFalse(fm.fileExists(atPath: file.path))
        XCTAssertEqual(ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults).emails, [])
    }

    func test_without_an_open_store_the_list_waits() throws {
        try KeptGoogleAccounts.write(sample, in: root)
        XCTAssertNil(KeptGoogleAccounts.restore(in: root, store: nil, hasSignIn: { _, _ in true }, defaults: defaults))
        XCTAssertTrue(fm.fileExists(atPath: file.path))
    }

    /// The live sign-in check asks the Keychain (the test host's in-memory one) for the item the
    /// connector's `GoogleOAuthHelper` writes, by the same account name; existence only.
    func test_the_sign_in_check_uses_the_connectors_keychain_items() throws {
        let email = "kept-\(UUID().uuidString)@example.test"
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                    kSecAttrService as String: GoogleOAuthHelper.keychainService,
                                    kSecAttrAccount as String: "gmail-connector:\(email)"]
        XCTAssertEqual(KeychainAccess.upsert(query, data: Data("t".utf8)), errSecSuccess)
        defer { KeychainAccess.delete(query as CFDictionary) }
        XCTAssertTrue(KeptGoogleAccounts.hasStoredSignIn(connectorId: "gmail", email: email))
        XCTAssertFalse(KeptGoogleAccounts.hasStoredSignIn(connectorId: "calendar", email: email))
        XCTAssertFalse(KeptGoogleAccounts.hasStoredSignIn(connectorId: "outlook365", email: email))
        XCTAssertEqual(KeptGoogleAccounts.keychainAccount(connectorId: "calendar", email: email), "google-calendar-connector:\(email)")
    }

    /// Log lines carry counts only, never an email address.
    func test_log_lines_carry_no_email() {
        let lines = [KeptGoogleAccounts.restoreLogMessage(.init(restored: 3, dropped: 1)),
                     KeptGoogleAccounts.restoreLogMessage(.init(unreadable: true)),
                     KeptGoogleAccounts.restoreLogMessage(.init(unreadable: true, fileLeft: true)),
                     KeptGoogleAccounts.restoreLogMessage(.init(restored: 1, fileLeft: true)),
                     KeptGoogleAccounts.writeFailedLogMessage(accounts: 4, code: 13),
                     KeptGoogleAccounts.rebuildLogMessage(accounts: 2)]
        for line in lines {
            XCTAssertFalse(line.contains("@"), line)
            XCTAssertFalse(line.contains("example"), line)
        }
        XCTAssertTrue(lines[0].contains("restored 3") && lines[0].contains("dropped 1"), lines[0])
    }
}
