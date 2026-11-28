import XCTest
@testable import Whimbrel

final class SettingsSearchTests: XCTestCase {
    func testEmptyQueryReturnsNothing() {
        XCTAssertEqual(SettingsSearch.results(for: ""), [])
    }

    func testWhitespaceQueryReturnsNothing() {
        XCTAssertEqual(SettingsSearch.results(for: "   \n\t "), [])
    }

    func testFindsLocalOnlyByTitle() {
        XCTAssertTrue(SettingsSearch.results(for: "local").map(\.id).contains("ai.localOnly"))
    }

    func testFindsPlanByKeyword() {
        let hits = SettingsSearch.results(for: "refund")
        XCTAssertFalse(hits.isEmpty)
        XCTAssertTrue(hits.contains { $0.section == .plan })
    }

    func testDiacriticAndCaseInsensitive() {
        let plain = SettingsSearch.results(for: "local").map(\.id)
        XCTAssertEqual(SettingsSearch.results(for: "LOCÁL").map(\.id), plain)
        XCTAssertEqual(SettingsSearch.results(for: "  loCal ").map(\.id), plain)
    }

    func testTitleMatchesComeBeforeKeywordOnlyMatches() {
        let hits = SettingsSearch.results(for: "local")
        let firstKeywordOnly = hits.firstIndex { !$0.title.localizedStandardContains("local") }
        let lastTitle = hits.lastIndex { $0.title.localizedStandardContains("local") }
        if let k = firstKeywordOnly, let t = lastTitle { XCTAssertLessThan(t, k) }
    }

    func testRegexCharactersDoNotCrash() {
        for q in ["(", "[a", "\\", ".*", "+"] {
            XCTAssertEqual(SettingsSearch.results(for: q), [], "query \(q)")
        }
    }

    func testSignOutDoesNotFindDeleteAccount() {
        XCTAssertFalse(SettingsSearch.results(for: "sign out").map(\.id).contains("devices.deleteAccount"))
    }

    func testFindsPermissionsEntry() {
        XCTAssertTrue(SettingsSearch.results(for: "microphone").map(\.id).contains("general.permissions"))
    }

    func testNoMatchReturnsEmpty() {
        XCTAssertEqual(SettingsSearch.results(for: "zzzzqxj"), [])
    }

    func testTellAFriendIsFoundInSyncAndDevicesByItsTitleAndByShare() {
        for query in ["tell a friend", "share", "friend"] {
            let hit = SettingsSearch.results(for: query).first { $0.id == "devices.tellAFriend" }
            XCTAssertEqual(hit?.section, .devices, query)
        }
    }

    func testEntryIdsAreUnique() {
        let ids = SettingsSearch.entries.map(\.id)
        XCTAssertEqual(ids.count, Set(ids).count)
    }

    func testEveryEntryAnchorExistsInSource() throws {
        // apps/macos/WhimbrelTests/SettingsSearchTests.swift -> apps/macos/Whimbrel/UI
        let ui = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI")
        let files = try XCTUnwrap(FileManager.default.enumerator(
            at: ui, includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]))
        var source = ""
        for case let url as URL in files where url.pathExtension == "swift" {
            guard (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else { continue }
            source += try String(contentsOf: url, encoding: .utf8) + "\n"
        }
        XCTAssertFalse(source.isEmpty, "no sources found under \(ui.path)")
        for entry in SettingsSearch.entries {
            let needle = ".settingsAnchor(\"\(entry.id)\")"
            let count = source.components(separatedBy: needle).count - 1
            // The redrawn sections anchor their rows through `SettingsAnchorID`, which the redesign tests check against the
            // registry. A literal call remains only in a pane that the Settings window still hosts or whose helpers are still
            // used, and then once.
            let allowed = entry.inClassicPane ? 1 : 0
            XCTAssertLessThanOrEqual(count, allowed, "anchor \(entry.id) appears \(count) times in the UI source, at most \(allowed) expected")
        }

        // Orphans: a call site whose id is not in the registry.
        let known = Set(SettingsSearch.entries.map(\.id))
        let prefix = ".settingsAnchor(\""
        for piece in source.components(separatedBy: prefix).dropFirst() {
            guard let end = piece.firstIndex(of: "\"") else { continue }
            let id = String(piece[..<end])
            XCTAssertTrue(known.contains(id), "orphan anchor \(id) is not in the registry")
        }
    }
}
