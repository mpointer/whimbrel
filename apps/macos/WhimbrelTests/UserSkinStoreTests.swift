import XCTest
@testable import Whimbrel

final class UserSkinStoreTests: XCTestCase {
    /// A store on a fresh temp folder, removed after the test.
    private func store() -> (UserSkinStore, URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("user-skins-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return (UserSkinStore(root: root), root)
    }

    private func defaults() -> UserDefaults {
        let suite = "user-skin-store-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        addTeardownBlock {
            d.removePersistentDomain(forName: suite)
            let plist = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/\(suite).plist")
            try? FileManager.default.removeItem(at: plist)
        }
        return d
    }

    private func manifest(_ id: String, name: String = "Mine", greeting: String? = nil) -> Data {
        var root: [String: Any] = ["id": id, "name": name, "palettes": ["\(id)-dark"]]
        if let greeting { root["voice"] = ["greetingMorning": greeting] }
        return try! JSONSerialization.data(withJSONObject: root, options: [.sortedKeys])
    }

    private let palette = Data(#"{"id":"p","name":"P","light":{}}"#.utf8)

    private func exists(_ url: URL) -> Bool { FileManager.default.fileExists(atPath: url.path) }

    func testSaveWritesFolderAndVersion() throws {
        let (s, root) = store()
        let version = try s.save(manifest: manifest("user-a-1"), palettes: [(id: "user-a-1-dark", data: palette)], for: "user-a-1")
        XCTAssertEqual(version, 1)
        let folder = root.appendingPathComponent("user-a-1")
        XCTAssertTrue(exists(folder.appendingPathComponent("skin.json")))
        XCTAssertTrue(exists(folder.appendingPathComponent("palettes/user-a-1-dark.json")))
        XCTAssertTrue(exists(folder.appendingPathComponent("history/1.json")))
        XCTAssertEqual(s.skinIds(), ["user-a-1"])
    }

    func testHistoryCappedAtTwenty() throws {
        let (s, root) = store()
        for i in 1...25 { _ = try s.save(manifest: manifest("user-a-1", name: "v\(i)"), palettes: [], for: "user-a-1") }
        let names = try FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent("user-a-1/history").path)
        let numbers = names.compactMap { Int(($0 as NSString).deletingPathExtension) }.sorted()
        XCTAssertEqual(numbers.count, 20)
        XCTAssertEqual(numbers.first, 6)
        XCTAssertEqual(numbers.last, 25)
    }

    func testUndoRestoresPreviousVersionExactly() throws {
        let (s, root) = store()
        let a = manifest("user-a-1", name: "A"), b = manifest("user-a-1", name: "B")
        _ = try s.save(manifest: a, palettes: [(id: "pa", data: palette)], for: "user-a-1")
        _ = try s.save(manifest: b, palettes: [(id: "pb", data: Data("{}".utf8))], for: "user-a-1")
        XCTAssertTrue(try s.undo(skinId: "user-a-1"))
        let folder = root.appendingPathComponent("user-a-1")
        XCTAssertEqual(try Data(contentsOf: folder.appendingPathComponent("skin.json")), a)
        XCTAssertEqual(try Data(contentsOf: folder.appendingPathComponent("palettes/pa.json")), palette)
        XCTAssertFalse(exists(folder.appendingPathComponent("palettes/pb.json")))
        XCTAssertFalse(try s.undo(skinId: "user-a-1"))
        XCTAssertEqual(try Data(contentsOf: folder.appendingPathComponent("skin.json")), a)
    }

    func testNewSkinIdIsUniqueAndPrefixed() throws {
        let (s, _) = store()
        let first = s.newSkinId(slug: "steelers")
        XCTAssertEqual(first, "user-steelers-1")
        _ = try s.save(manifest: manifest(first), palettes: [], for: first)
        XCTAssertEqual(s.newSkinId(slug: "steelers"), "user-steelers-2")
    }

    func testUnsafeIdsAreRefused() {
        let (s, _) = store()
        XCTAssertThrowsError(try s.save(manifest: manifest("user-a-1"), palettes: [], for: "../escape"))
        XCTAssertThrowsError(try s.save(manifest: manifest("user-a-1"), palettes: [(id: "../x", data: palette)], for: "user-a-1"))
        XCTAssertEqual(s.newSkinId(slug: "../Big Team"), "user-big-team-1")
        XCTAssertEqual(s.newSkinId(slug: "!!"), "user-skin-1")
        XCTAssertEqual(s.newSkinId(slug: String(repeating: "ab ", count: 30)), "user-\(String(repeating: "ab-", count: 13))a-1")
    }

    func testLoadedUserSkinsGoThroughValidator() throws {
        let (s, _) = store()
        let p = Data(#"{"id":"user-a-1-dark","name":"Dark","light":{}}"#.utf8)
        _ = try s.save(manifest: manifest("user-a-1", greeting: String(repeating: "x", count: 200)),
                       palettes: [(id: "user-a-1-dark", data: p)], for: "user-a-1")
        let loaded = s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel)
        XCTAssertEqual(loaded.skins.map(\.id), ["user-a-1"])
        XCTAssertEqual(loaded.skins[0].voice.values[.greetingMorning], SkinDefaults.whimbrel.voice.values[.greetingMorning])
        XCTAssertEqual(loaded.palettes.map(\.id), ["user-a-1-dark"])
    }

    func testDeletingSelectedSkinFallsBackToWhimbrel() throws {
        let (users, root) = store()
        let d = defaults()
        let skins = SkinStore(defaults: d, themes: ThemeStore(defaults: d), userSkins: users)
        _ = try users.save(manifest: manifest("user-a-1"), palettes: [], for: "user-a-1")
        skins.reloadUserSkins()
        skins.select(skinId: "user-a-1", paletteId: nil)
        XCTAssertEqual(skins.selected.id, "user-a-1")
        skins.reloadUserSkins()
        XCTAssertEqual(skins.selected.id, "user-a-1", "a reload keeps a selection that still exists")
        try users.delete(skinId: "user-a-1")
        skins.reloadUserSkins()
        XCTAssertEqual(skins.selected.id, "whimbrel")
        XCTAssertEqual(skins.selectedPaletteId, "default")
        XCTAssertFalse(exists(root.appendingPathComponent("user-a-1")))
    }

    func testChatRoundTrip() throws {
        let (s, _) = store()
        let one = SkinChatTurn(role: "user", text: "make it gold", date: Date(timeIntervalSince1970: 1_000))
        let two = SkinChatTurn(role: "whimbrel", text: "Gold it is.", date: Date(timeIntervalSince1970: 1_001.5))
        XCTAssertThrowsError(try s.appendChat(one, to: "user-a-1"), "an unsaved skin gets no chat-only folder")
        _ = try s.save(manifest: manifest("user-a-1"), palettes: [], for: "user-a-1")
        try s.appendChat(one, to: "user-a-1")
        try s.appendChat(two, to: "user-a-1")
        XCTAssertEqual(s.chat(for: "user-a-1"), [one, two])
        XCTAssertEqual(s.chat(for: "user-b-1"), [])
    }

    func testAddImageFlattensAndDeduplicatesNames() throws {
        let (s, _) = store()
        let name = try s.addImage(Data([1]), preferredName: "../dir/Logo Big.png", to: "user-a-1")
        XCTAssertEqual(name, "logo-big.png")
        XCTAssertEqual(try s.addImage(Data([2]), preferredName: "logo-big.png", to: "user-a-1"), "logo-big-2.png")
        XCTAssertEqual(try Data(contentsOf: s.imageURL(asset: name, skinId: "user-a-1")), Data([1]))
        XCTAssertEqual(try s.addImage(Data([3]), preferredName: "https://x.test/a/Photo.JPG?w=1&h=2#top", to: "user-a-1"), "photo.jpg")
        let long = try s.addImage(Data([4]), preferredName: String(repeating: "x", count: 90) + ".png", to: "user-a-1")
        XCTAssertEqual(long, String(repeating: "x", count: 36) + ".png")
    }

    func testPaletteNotNamedForItsSkinIsDropped() throws {
        let (s, _) = store()
        let own = Data(#"{"id":"user-a-1-p1","name":"Own","light":{}}"#.utf8)
        let clash = Data(#"{"id":"default","name":"Clash","light":{}}"#.utf8)
        _ = try s.save(manifest: manifest("user-a-1"), palettes: [(id: "user-a-1-p1", data: own), (id: "default", data: clash)], for: "user-a-1")
        XCTAssertEqual(s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel).palettes.map(\.id), ["user-a-1-p1"])
    }

    func testRenameRewritesNameAsNewVersion() throws {
        let (s, _) = store()
        _ = try s.save(manifest: manifest("user-a-1"), palettes: [(id: "user-a-1-dark", data: palette)], for: "user-a-1")
        try s.rename(skinId: "user-a-1", to: "Renamed")
        XCTAssertEqual(s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel).skins.first?.name, "Renamed")
        XCTAssertTrue(try s.undo(skinId: "user-a-1"))
        XCTAssertEqual(s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel).skins.first?.name, "Mine")
    }

    /// I3: a name is at most `nameLimit` (40) characters, whoever writes it.
    func testRenameCapsName() throws {
        let (s, _) = store()
        _ = try s.save(manifest: manifest("user-a-1"), palettes: [(id: "user-a-1-dark", data: palette)], for: "user-a-1")
        try s.rename(skinId: "user-a-1", to: "  " + String(repeating: "Long name ", count: 10))
        let name = try XCTUnwrap(s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel).skins.first?.name)
        XCTAssertTrue((UserSkinStore.nameLimit - 1...UserSkinStore.nameLimit).contains(name.count), "cut at 40 (a trailing space trimmed): \(name.count)")
        XCTAssertTrue(name.hasPrefix("Long name"))
    }
}
