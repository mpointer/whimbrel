import AppKit
import XCTest
@testable import Whimbrel

/// Holds a fake call until the test releases it.
private final class Gate: @unchecked Sendable {
    private let lock = NSLock()
    private var open = false
    private var waiter: CheckedContinuation<Void, Never>?

    func wait() async {
        await withCheckedContinuation { (k: CheckedContinuation<Void, Never>) in
            lock.lock()
            if open { lock.unlock(); k.resume() } else { waiter = k; lock.unlock() }
        }
    }

    func release() {
        lock.lock()
        open = true
        let k = waiter
        waiter = nil
        lock.unlock()
        k?.resume()
    }
}

private class FakeBackend: SkinTurnBackend, @unchecked Sendable {
    var reply: Result<SkinTurnReplyDTO, Error>
    var gate: Gate?
    var searchResults: (String) -> [ImageCandidateDTO] = { _ in [] }
    var image = FakeBackend.png
    private(set) var turnCalls = 0
    private(set) var lastInput: Data?
    private(set) var searches: [String] = []
    private(set) var generations: [String] = []
    private(set) var fetches: [String] = []

    init(_ reply: SkinTurnReplyDTO) { self.reply = .success(reply) }

    func turn(_ input: Data, provider: RoutingSettings.Provider, model: String) async throws -> SkinTurnReplyDTO {
        turnCalls += 1
        lastInput = input
        if let gate { await gate.wait() }
        return try reply.get()
    }

    func search(_ query: String) async throws -> [ImageCandidateDTO] {
        searches.append(query)
        return searchResults(query)
    }

    func generate(_ prompt: String) async throws -> Data {
        generations.append(prompt)
        return image
    }

    func fetch(_ url: String) async throws -> Data {
        fetches.append(url)
        return image
    }

    /// A 2×2 PNG.
    static let png: Data = {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        return rep.representation(using: .png, properties: [:])!
    }()
}

@MainActor
final class SkinChatSessionTests: XCTestCase {
    private var root: URL!
    private var users: UserSkinStore!
    private var skins: SkinStore!
    private var access = SkinChatAccess(provider: .ollama, model: "gemma", localOnly: false, hasKey: true, hasSearchKey: false, hasImageKey: false)

    override func setUp() async throws {
        let suite = "skin-chat-session-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        root = FileManager.default.temporaryDirectory.appendingPathComponent("skin-chat-\(UUID().uuidString)")
        let folder = root!
        addTeardownBlock {
            d.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/\(suite).plist"))
            try? FileManager.default.removeItem(at: folder)
        }
        users = UserSkinStore(root: root)
        skins = SkinStore(defaults: d, themes: ThemeStore(defaults: d), userSkins: users)
        skins.installBundledAndUserSkins()
    }

    private func session(_ backend: FakeBackend) -> SkinChatSession {
        SkinChatSession(skinStore: skins, userSkins: users, backend: backend, routing: RoutingSettings(), access: { [unowned self] in self.access })
    }

    private func manifest(name: String = "Beach", palettes: [String] = ["sand"], extra: [String: Any] = [:]) -> Data {
        var m: [String: Any] = ["id": "whatever", "name": name, "palettes": palettes, "today": ["archetype": "glass"]]
        m.merge(extra) { $1 }
        return try! JSONSerialization.data(withJSONObject: m)
    }

    /// Passes every archetype's pairs (`ArchetypeContrast`), so saving it repairs nothing.
    private func palette(_ id: String = "sand", light: [String: String] = ["backgroundPage": "#fdf6e3", "textPrimary": "#222222", "textSecondary": "#444444"]) -> Data {
        try! JSONSerialization.data(withJSONObject: ["id": id, "name": "Sand", "light": light])
    }

    private func reply(_ intent: SkinTurnReplyDTO.Intent, name: String = "Beach", queries: [String] = [], prompts: [String] = []) -> SkinTurnReplyDTO {
        SkinTurnReplyDTO(intent: intent, reply: "Done.", notes: [.init(step: "palette", text: "Sandy page, sea accents")],
                         skin: manifest(name: name), palettes: [palette()], imageQueries: queries, imagePrompts: prompts)
    }

    private func run(_ s: SkinChatSession, _ text: String) async {
        s.send(text)
        await s.current?.value
    }

    private func folderNames() -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []).sorted()
    }

    private func candidate(_ n: Int, _ prefix: String = "") -> ImageCandidateDTO {
        ImageCandidateDTO(url: "https://img.test/\(prefix)\(n).png?size=big", thumbnailUrl: "https://img.test/t\(prefix)\(n).png", title: "\(n)", source: "web")
    }

    // MARK: turns

    func testNewTurnCreatesSelectsAndSavesSkin() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertEqual(skins.selected.id, "user-beach-1")
        XCTAssertEqual(skins.selected.name, "Beach")
        XCTAssertEqual(skins.selected.archetype, .glass)
        XCTAssertEqual(skins.selected.paletteIds, ["user-beach-1-p1"])
        XCTAssertEqual(skins.selectedPaletteId, "user-beach-1-p1")
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent("user-beach-1/history/1.json").path))
        XCTAssertTrue(s.status.contains("Sandy page, sea accents"))
        XCTAssertEqual(s.turns.map(\.role), ["user", "whimbrel"])
        XCTAssertEqual(users.chat(for: "user-beach-1").map(\.text), ["a beach skin", "Done."])
        XCTAssertFalse(s.isRunning)

        // The wire shape of core/llm's SkinTurnInput.
        let input = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(backend.lastInput)) as? [String: Any])
        XCTAssertEqual(Set(input.keys), ["request", "history", "currentManifest", "catalogue"])
        XCTAssertEqual(input["request"] as? String, "a beach skin")
        XCTAssertEqual((input["currentManifest"] as? [String: Any])?["id"] as? String, "whimbrel", "Whimbrel's bundled manifest")
        XCTAssertNotNil((input["catalogue"] as? [String: Any])?["palette"])
        XCTAssertEqual((input["history"] as? [Any])?.count, 0)
    }

    func testRefiningBuiltInForksAndLeavesBuiltInUntouched() async throws {
        skins.select(skinId: "game-day", paletteId: nil)
        let before = skins.skins.first { $0.id == "game-day" }
        var r = reply(.refine, name: "Game Day")
        r.skin = manifest(name: "Game Day", extra: ["today": ["archetype": "field"]])  // its own archetype, so its palettes pass
        r.palettes = []
        let s = session(FakeBackend(r))
        await run(s, "make the greeting louder")
        XCTAssertEqual(skins.selected.id, "user-game-day-1")
        XCTAssertEqual(skins.selected.name, "Game Day (yours)")
        XCTAssertEqual(skins.selected.paletteIds, before?.paletteIds, "a fork without new colours keeps the built-in's")
        XCTAssertEqual(skins.skins.first { $0.id == "game-day" }, before)
        XCTAssertEqual(folderNames(), ["user-game-day-1"])
    }

    func testNewIntentOnUserSkinMakesSecondSkin() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        backend.reply = .success(reply(.new, name: "Forest"))
        await run(s, "now a forest one")
        XCTAssertEqual(folderNames(), ["user-beach-1", "user-forest-1"])
        XCTAssertEqual(skins.selected.id, "user-forest-1")
        XCTAssertEqual(users.chat(for: "user-beach-1").count, 2, "the first skin's chat is untouched")
    }

    func testGarbageReplyChangesNothing() async throws {
        let backend = FakeBackend(reply(.new))
        backend.reply = .failure(CoreLLMError.providerFailure)
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertEqual(skins.selected.id, "whimbrel")
        XCTAssertEqual(folderNames(), [])
        XCTAssertEqual(s.turns.filter { $0.role == "whimbrel" }.count, 1)

        // A reply with no manifest is garbage too.
        var empty = reply(.new)
        empty.skin = nil
        backend.reply = .success(empty)
        await run(s, "a beach skin")
        XCTAssertEqual(skins.selected.id, "whimbrel")
        XCTAssertEqual(folderNames(), [])
    }

    func testFailingPaletteRepairedAndExplained() async throws {
        var r = reply(.new)
        r.palettes = [palette(light: ["backgroundPage": "#ffffff", "backgroundCard": "#ffffff", "textPrimary": "#cccccc"])]
        let s = session(FakeBackend(r))
        await run(s, "a pale skin")
        let theme = try XCTUnwrap(skins.palettes(for: skins.selected).first)
        XCTAssertEqual(theme.id, "user-beach-1-p1")
        XCTAssertTrue(ThemeContrast.passes(theme.light))
        XCTAssertNotEqual(theme.light, ThemeDefaults.theme.light, "repaired, not replaced by Default")
        XCTAssertEqual(theme.light[.backgroundPage], 0xFFFFFF)
        XCTAssertNotEqual(theme.light[.textPrimary], 0xCCCCCC)
        XCTAssertTrue(s.turns.contains { $0.role == "whimbrel" && $0.text.contains("text primary") && $0.text.contains("readable") })
    }

    /// Gold accent text on a white page passes `requiredPairs` but not what the archetypes draw: it is darkened and said so.
    func testGoldAccentOnWhiteIsRepairedAndExplained() async throws {
        var r = reply(.new)
        r.palettes = [palette(light: ["backgroundPage": "#ffffff", "backgroundCard": "#ffffff", "textPrimary": "#222222", "accentPrimary": "#ffb612"])]
        let s = session(FakeBackend(r))
        await run(s, "a steelers skin")
        let theme = try XCTUnwrap(skins.palettes(for: skins.selected).first)
        XCTAssertEqual(theme.id, "user-beach-1-p1")
        XCTAssertEqual(ArchetypeContrast.failures(theme.light), [])
        XCTAssertNotEqual(theme.light[.accentPrimary], 0xFFB612)
        XCTAssertTrue(s.turns.contains { $0.role == "whimbrel" && $0.text.contains("accent primary") })
    }

    /// I3: the model-written name is capped at `nameLimit` (40) characters, on a new skin and on a refine of yours.
    func testModelSkinNameIsCapped() async throws {
        let long = String(repeating: "Beach ", count: 20)
        let backend = FakeBackend(reply(.new, name: long))
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertTrue((UserSkinStore.nameLimit - 1...UserSkinStore.nameLimit).contains(skins.selected.name.count), "cut at 40 (a trailing space trimmed): \(skins.selected.name.count)")
        backend.reply = .success(reply(.refine, name: "Sea " + long))
        await run(s, "darker")
        XCTAssertTrue((UserSkinStore.nameLimit - 1...UserSkinStore.nameLimit).contains(skins.selected.name.count), "cut at 40 (a trailing space trimmed): \(skins.selected.name.count)")
        XCTAssertTrue(skins.selected.name.hasPrefix("Sea Beach"))
    }

    /// M3: top-level keys the loader doesn't know are dropped before saving (so never re-sent or exported).
    func testUnknownTopLevelKeysAreDropped() async throws {
        var r = reply(.new)
        r.skin = manifest(extra: ["junk": ["deep": "x"], "author": "Model", "archetypeOptions": ["a": "b"]])
        let s = session(FakeBackend(r))
        await run(s, "a beach skin")
        let saved = try XCTUnwrap(users.manifest(for: "user-beach-1"))
        let keys = Set(try XCTUnwrap(JSONSerialization.jsonObject(with: saved) as? [String: Any]).keys)
        XCTAssertTrue(keys.isSubset(of: SkinLoader.topLevelKeys), "\(keys.subtracting(SkinLoader.topLevelKeys))")
        XCTAssertFalse(keys.contains("junk"))
        XCTAssertTrue(keys.contains("today"))
    }

    /// M1: the status line names everything sent: the request, this skin's chat, its design and the allowed values.
    func testStatusNamesEverythingSent() async throws {
        access = SkinChatAccess(provider: .anthropic, model: "claude", localOnly: false, hasKey: true, hasSearchKey: false, hasImageKey: false)
        let s = session(FakeBackend(reply(.new)))
        await run(s, "a beach skin")
        XCTAssertTrue(s.status.contains("Sent to Anthropic: your request, this skin's chat, its design and the list of allowed values"), "\(s.status)")
    }

    /// I7: the blocking core calls run on a GCD queue, not Swift's cooperative pool, and errors come back.
    func testBlockingCallsRunOffTheCooperativePool() async throws {
        let label = try await CoreSkinTurnBackend.offPool { () -> Result<String, CoreLLMError> in
            .success(String(cString: __dispatch_queue_get_label(nil)))
        }
        XCTAssertEqual(label, CoreSkinTurnBackend.queue.label)
        do {
            _ = try await CoreSkinTurnBackend.offPool { () -> Result<Int, CoreLLMError> in .failure(.providerFailure) }
            XCTFail("the error must come back")
        } catch {
            XCTAssertEqual(error as? CoreLLMError, .providerFailure)
        }
    }

    /// N1: "make Whimbrel glassy" with no new colours forks Whimbrel as Glass; its kept built-in palettes fail Glass's
    /// glow, so repaired copies are saved as the fork's own and used.
    func testForkIntoGlassRepairsKeptBuiltInPalettes() async throws {
        var r = reply(.refine, name: "Whimbrel")
        r.palettes = []
        let s = session(FakeBackend(r))
        await run(s, "make Whimbrel glassy")
        XCTAssertEqual(skins.selected.id, "user-whimbrel-1")
        XCTAssertEqual(skins.selected.archetype, .glass)
        let palettes = skins.palettes(for: skins.selected)
        XCTAssertFalse(palettes.isEmpty)
        for theme in palettes {
            XCTAssertTrue(theme.id.hasPrefix("user-whimbrel-1-p"), theme.id)
            XCTAssertEqual(ArchetypeContrast.failures(theme.light, ArchetypeContrast.pairs(for: .glass)), [], "\(theme.id) light")
            XCTAssertEqual(ArchetypeContrast.failures(theme.dark, ArchetypeContrast.pairs(for: .glass)), [], "\(theme.id) dark")
        }
        XCTAssertTrue(s.turns.contains { $0.text.hasPrefix("I adjusted") })
    }

    /// N1: a Glass skin whose palettes are all rejected falls back to Whimbrel's colours, repaired for Glass.
    func testGlassSkinWithEveryPaletteRejectedStillPasses() async throws {
        var r = reply(.new)
        r.palettes = [palette(light: ["textPrimary": "red"])]
        let s = session(FakeBackend(r))
        await run(s, "a glass skin")
        XCTAssertEqual(skins.selected.archetype, .glass)
        let theme = try XCTUnwrap(skins.palettes(for: skins.selected).first)
        XCTAssertEqual(ArchetypeContrast.failures(theme.light, ArchetypeContrast.pairs(for: .glass)), [])
        XCTAssertEqual(ArchetypeContrast.failures(theme.dark, ArchetypeContrast.pairs(for: .glass)), [])
        XCTAssertTrue(s.turns.contains { $0.text.contains("Whimbrel's colours") })
    }

    /// N5: a name of only whitespace falls back to "My skin" (it was capped to "" after the check, and the save failed).
    func testWhitespaceNameFallsBackToMySkin() async throws {
        let s = session(FakeBackend(reply(.new, name: "   ")))
        await run(s, "a skin")
        XCTAssertEqual(skins.selected.name, "My skin")
        XCTAssertEqual(skins.selected.id, "user-my-skin-1")
    }

    func testTurnWithoutPalettesFallsBackVisibly() async throws {
        var r = reply(.new)
        r.palettes = []
        let s = session(FakeBackend(r))
        await run(s, "a beach skin")
        XCTAssertEqual(skins.selected.id, "user-beach-1")
        // Whimbrel's colours, as a copy repaired for this skin's Glass archetype (N1).
        XCTAssertEqual(skins.selected.paletteIds, ["user-beach-1-p1"])
        XCTAssertEqual(skins.palettes(for: skins.selected).first?.name, ThemeDefaults.theme.name)
        XCTAssertTrue(s.turns.contains { $0.text.contains("Whimbrel's colours") })
    }

    func testResultAppliesToOriginatingSkinAfterSelectionChange() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        let gate = Gate()
        backend.gate = gate
        backend.reply = .success(reply(.refine, name: "Beach"))
        s.send("darker sand")
        skins.select(skinId: "game-day", paletteId: nil)
        gate.release()
        await s.current?.value
        XCTAssertEqual(skins.selected.id, "game-day")
        let history = try FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent("user-beach-1/history").path)
        XCTAssertEqual(history.count, 2, "the refine saved a second version of the originating skin")
        XCTAssertEqual(folderNames(), ["user-beach-1"])
    }

    func testAutoSearchOnlyOnNewSkinWithKey() async throws {
        access.hasSearchKey = true
        let backend = FakeBackend(reply(.new, queries: ["beach", "waves"]))
        backend.searchResults = { _ in (1...4).map { self.candidate($0) } }
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertEqual(backend.searches, ["beach", "waves"])
        XCTAssertEqual(s.candidates.count, 4, "duplicates across queries are shown once")
        XCTAssertTrue(s.status.contains("Searching the web for 'beach'…"))

        backend.reply = .success(reply(.refine, queries: ["dunes"]))
        await run(s, "darker sand")
        XCTAssertEqual(backend.searches, ["beach", "waves"], "a refine that doesn't mention images doesn't search")

        access.hasSearchKey = false
        backend.reply = .success(reply(.new, name: "Forest", queries: ["trees"]))
        await run(s, "a forest skin")
        XCTAssertEqual(backend.searches, ["beach", "waves"], "no key, no search")
    }

    func testSearchCappedAtSix() async throws {
        access.hasSearchKey = true
        let backend = FakeBackend(reply(.new, queries: ["a", "b", "c"]))
        backend.searchResults = { q in (1...5).map { self.candidate($0, q) } }
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertEqual(s.candidates.count, SkinChatSession.searchLimit)
        XCTAssertEqual(backend.searches, ["a", "b"], "no search once six are found")
    }

    func testGenerationOnlyWhenTheRequestAsksForAnImage() async throws {
        access.hasImageKey = true
        let backend = FakeBackend(reply(.new, prompts: ["a stadium at night"]))
        let s = session(backend)
        await run(s, "a game day skin")
        XCTAssertEqual(backend.generations, [], "R4: no image word, no generation")
        backend.reply = .success(reply(.refine, prompts: ["a stadium at night"]))
        await run(s, "make me a Banner of a stadium")
        XCTAssertEqual(backend.generations, ["a stadium at night"])
        XCTAssertEqual(s.candidates.map(\.source), ["generated"])
        let thumb = await s.thumbnail(for: s.candidates[0])
        XCTAssertNotNil(thumb)
        XCTAssertEqual(backend.fetches, [], "a generated image is never fetched")
    }

    func testLocalOnlyBlocksHostedProvider() async throws {
        access = SkinChatAccess(provider: .anthropic, model: "claude", localOnly: true, hasKey: true, hasSearchKey: true, hasImageKey: true)
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        XCTAssertFalse(s.available)
        s.send("a beach skin")
        await s.current?.value
        XCTAssertEqual(backend.turnCalls, 0)
        XCTAssertEqual(folderNames(), [])
        XCTAssertEqual(s.turns.last?.role, "whimbrel")

        access.provider = .ollama
        XCTAssertTrue(s.available, "a local model still works in local-only mode")
        XCTAssertFalse(access.canSearch || access.canGenerate, "hosted image calls are off in local-only mode")
    }

    // MARK: images

    func testNoImageWithoutApproval() async throws {
        access.hasSearchKey = true
        let backend = FakeBackend(reply(.new, queries: ["beach"]))
        backend.searchResults = { _ in [self.candidate(1), self.candidate(2)] }
        let s = session(backend)
        await run(s, "a beach skin")
        let images = root.appendingPathComponent("user-beach-1/images")
        XCTAssertEqual(s.candidates.count, 2)
        XCTAssertFalse(FileManager.default.fileExists(atPath: images.path))
        XCTAssertEqual(skins.selected.decoration.imagery, [])

        _ = await s.thumbnail(for: s.candidates[0])
        XCTAssertEqual(backend.fetches, ["https://img.test/t1.png"], "thumbnails go through the guarded fetch")
        XCTAssertFalse(FileManager.default.fileExists(atPath: images.path), "a thumbnail is not an approval")

        s.reject(s.candidates[1])
        XCTAssertFalse(FileManager.default.fileExists(atPath: images.path))

        s.approve(s.candidates[0])
        await s.current?.value
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: images.path), ["1.png"])
        XCTAssertEqual(skins.selected.decoration.imagery, [SkinImage(asset: "1.png", placement: .header, textFree: false)])
        XCTAssertEqual(s.candidates, [])
    }

    /// M2: your own file gets a neutral name by its type, so its file name never reaches the provider.
    func testOwnImageIsPlaced() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        let file = root.appendingPathComponent("Mike and the kids.png")
        try FakeBackend.png.write(to: file)
        s.addOwnImage(file)
        await s.current?.value
        XCTAssertEqual(skins.selected.decoration.imagery.map(\.asset), ["own.png"])
        let jpeg = root.appendingPathComponent("Holiday.JPG")
        try XCTUnwrap(NSBitmapImageRep(data: FakeBackend.png)?.representation(using: .jpeg, properties: [:])).write(to: jpeg)
        s.addOwnImage(jpeg)
        await s.current?.value
        XCTAssertEqual(skins.selected.decoration.imagery.map(\.asset), ["own.jpg"], "the newest header replaces the last")
        backend.reply = .success(reply(.refine))
        await run(s, "darker sand")
        let sent = String(decoding: try XCTUnwrap(backend.lastInput), as: UTF8.self).lowercased()
        XCTAssertTrue(sent.contains("own.jpg"))
        XCTAssertFalse(sent.contains("kids") || sent.contains("holiday"), "own file names never reach the provider")
    }

    /// I4: a second approved image becomes the header (the first stays on disk for undo), and the chat says so.
    func testSecondApprovedImageReplacesTheHeader() async throws {
        access.hasSearchKey = true
        let backend = FakeBackend(reply(.new, queries: ["beach"]))
        backend.searchResults = { _ in [self.candidate(1), self.candidate(2)] }
        let s = session(backend)
        await run(s, "a beach skin")
        s.approve(s.candidates[0])
        await s.current?.value
        XCTAssertEqual(s.turns.last?.text, "Added the image to the header.")
        s.approve(s.candidates[0])
        await s.current?.value
        XCTAssertEqual(skins.selected.decoration.imagery, [SkinImage(asset: "2.png", placement: .header, textFree: false)])
        XCTAssertEqual(s.turns.last?.text, "Replaced the header image with this one.")
        let images = root.appendingPathComponent("user-beach-1/images").path
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: images).sorted(), ["1.png", "2.png"])
    }

    /// Skins Task 16: an approved image is kept at most 1600 px on its long side, so syncing it stays cheap.
    func test_approval_downscales_to_1600() async throws {
        access.hasSearchKey = true
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3200, pixelsHigh: 800, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let backend = FakeBackend(reply(.new, queries: ["beach"]))
        backend.image = rep.representation(using: .png, properties: [:])!
        backend.searchResults = { _ in [self.candidate(1)] }
        let s = session(backend)
        await run(s, "a beach skin")
        s.approve(s.candidates[0])
        await s.current?.value
        let stored = try Data(contentsOf: users.imageURL(asset: "1.png", skinId: "user-beach-1"))
        let image = try XCTUnwrap(NSBitmapImageRep(data: stored))
        XCTAssertEqual(image.pixelsWide, 1600)
        XCTAssertEqual(image.pixelsHigh, 400)
    }

    /// M8: switching to local-only mode after images were found stops their downloads.
    func testLocalOnlySwitchStopsImageFetches() async throws {
        access.hasSearchKey = true
        let backend = FakeBackend(reply(.new, queries: ["beach"]))
        backend.searchResults = { _ in [self.candidate(1)] }
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertEqual(s.candidates.count, 1)
        access.localOnly = true
        let thumb = await s.thumbnail(for: s.candidates[0])
        XCTAssertNil(thumb)
        s.approve(s.candidates[0])
        await s.current?.value
        XCTAssertEqual(backend.fetches, [], "nothing is downloaded in local-only mode")
        XCTAssertEqual(skins.selected.decoration.imagery, [])
        XCTAssertTrue(s.turns.contains { $0.text.contains("Local-only mode") })
    }

    func testImageApprovedDuringRefineSurvives() async throws {
        access.hasSearchKey = true
        let backend = FakeBackend(reply(.new, queries: ["beach"]))
        backend.searchResults = { _ in [self.candidate(1)] }
        let s = session(backend)
        await run(s, "a beach skin")
        let gate = Gate()
        backend.gate = gate
        backend.reply = .success(reply(.refine))
        s.send("darker sand")
        let refine = s.current
        s.approve(s.candidates[0])
        await s.current?.value
        gate.release()
        await refine?.value
        XCTAssertEqual(skins.selected.decoration.imagery.map(\.asset), ["1.png"], "the refine kept the image approved while it ran")
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent("user-beach-1/images/1.png").path))

        // An image the model saw and left out was removed on purpose.
        await run(s, "remove the picture")
        XCTAssertEqual(skins.selected.decoration.imagery, [])
    }

    func testImageryNeedsARegularApprovedFile() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        let images = root.appendingPathComponent("user-beach-1/images")
        try FileManager.default.createDirectory(at: images.appendingPathComponent("dir"), withIntermediateDirectories: true)
        try FakeBackend.png.write(to: images.appendingPathComponent("real.png"))
        var r = reply(.refine)
        let entries = [".", "..", "dir", "missing.png", "real.png"].map { ["asset": $0, "placement": "header"] }
        r.skin = manifest(extra: ["decoration": ["imagery": entries]])
        backend.reply = .success(r)
        await run(s, "use my images")
        XCTAssertEqual(skins.selected.decoration.imagery.map(\.asset), ["real.png"])
    }

    func testPaletteRepairThatGivesUpFallsBackVisibly() async throws {
        let original = SkinChatSession.repairPalette
        SkinChatSession.repairPalette = { ($0, []) }
        defer { SkinChatSession.repairPalette = original }
        var r = reply(.new)
        r.palettes = [palette(light: ["backgroundPage": "#ffffff", "textPrimary": "#eeeeee"])]
        let s = session(FakeBackend(r))
        await run(s, "a pale skin")
        XCTAssertEqual(skins.selected.paletteIds, [ThemeDefaults.theme.id])
        XCTAssertTrue(s.turns.contains { $0.text.contains("Whimbrel's colours") })
        XCTAssertFalse(s.turns.contains { $0.text.contains("I adjusted") }, "no claim of a repair that didn't happen")
    }

    func testRefineKeepsReferencedPalettesAndSelection() async throws {
        var first = reply(.new)
        first.palettes = [palette("sand"), palette("dusk")]
        first.skin = manifest(palettes: ["sand", "dusk"])
        let backend = FakeBackend(first)
        let s = session(backend)
        await run(s, "a beach skin")
        skins.select(skinId: "user-beach-1", paletteId: "user-beach-1-p2")
        let dusk = users.palettes(for: "user-beach-1").first { $0.id == "user-beach-1-p2" }?.data

        var refine = reply(.refine)
        refine.palettes = [palette("night")]
        refine.skin = manifest(palettes: ["user-beach-1-p2", "night"])
        backend.reply = .success(refine)
        await run(s, "add a night palette")
        XCTAssertEqual(skins.selected.paletteIds, ["user-beach-1-p2", "user-beach-1-p3"])
        XCTAssertEqual(skins.selectedPaletteId, "user-beach-1-p2", "the selected palette survives the refine")
        XCTAssertEqual(users.palettes(for: "user-beach-1").first { $0.id == "user-beach-1-p2" }?.data, dusk, "an unreturned palette is kept as is")

        // A palette the refine dropped falls back to the skin's first.
        skins.select(skinId: "user-beach-1", paletteId: "user-beach-1-p1")
        XCTAssertEqual(skins.selectedPaletteId, "user-beach-1-p2")
    }

    func testRefineWithRejectedPalettesKeepsColoursAndSaysSo() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        var refine = reply(.refine)
        refine.palettes = [palette("bad", light: ["textPrimary": "red"])]
        refine.skin = manifest(palettes: ["bad"])
        backend.reply = .success(refine)
        await run(s, "redder text")
        XCTAssertEqual(skins.selected.paletteIds, ["user-beach-1-p1"])
        XCTAssertTrue(s.turns.contains { $0.text.contains("keeps its current colours") })
    }

    func testUnknownPaletteReferencesUseTheReplysPalettes() async throws {
        var r = reply(.new)
        r.skin = manifest(palettes: ["nope"])
        let s = session(FakeBackend(r))
        await run(s, "a beach skin")
        XCTAssertEqual(skins.selected.paletteIds, ["user-beach-1-p1"])
    }

    func testOwnImageLimits() async throws {
        let s = session(FakeBackend(reply(.new)))
        await run(s, "a beach skin")
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("own-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        let big = dir.appendingPathComponent("big.png"), text = dir.appendingPathComponent("notes.png")
        try (FakeBackend.png + Data(count: SkinChatSession.imageByteLimit)).write(to: big)
        try Data("not an image".utf8).write(to: text)
        for file in [big, text] {
            s.addOwnImage(file)
            await s.current?.value
        }
        XCTAssertEqual(skins.selected.decoration.imagery, [])
        XCTAssertTrue(s.turns.contains { $0.text.contains("10 MB") })
        XCTAssertTrue(s.turns.contains { $0.text.contains("isn't an image") })
        XCTAssertTrue(SkinChatSession.isImage(Data([0xFF, 0xD8, 0xFF, 0xE0])))
        XCTAssertTrue(SkinChatSession.isImage(Data("RIFF\0\0\0\0WEBPVP8 ".utf8)))
        XCTAssertFalse(SkinChatSession.isImage(Data("RIFF\0\0\0\0WAVE".utf8)))
    }

    // MARK: stop, undo, spend cap

    func testStopDiscardsResult() async throws {
        let backend = FakeBackend(reply(.new))
        let gate = Gate()
        backend.gate = gate
        let s = session(backend)
        s.send("a beach skin")
        XCTAssertTrue(s.isRunning)
        s.stop()
        XCTAssertFalse(s.isRunning)
        gate.release()
        await s.current?.value
        XCTAssertEqual(skins.selected.id, "whimbrel")
        XCTAssertEqual(folderNames(), [])
    }

    func testUndoRestoresPrevious() async throws {
        let backend = FakeBackend(reply(.new))
        let s = session(backend)
        await run(s, "a beach skin")
        var darker = reply(.refine)
        darker.skin = manifest(extra: ["voice": ["tagline": "Low tide."]])
        backend.reply = .success(darker)
        await run(s, "tagline low tide")
        XCTAssertEqual(skins.selected.voice.values[.tagline], "Low tide.")
        s.undo()
        XCTAssertEqual(skins.selected.id, "user-beach-1")
        XCTAssertEqual(skins.selected.voice.values[.tagline], SkinDefaults.whimbrel.voice.values[.tagline])
    }

    func testSpendCapStops() async throws {
        let backend = FakeBackend(reply(.new))
        backend.reply = .failure(CoreLLMError.spendCap(scope: .user, spentCents: 200, capCents: 200))
        let s = session(backend)
        await run(s, "a beach skin")
        XCTAssertEqual(s.turns.last?.text, SkinChatSession.spendCapLine)
        XCTAssertFalse(s.isRunning)
        XCTAssertEqual(folderNames(), [])

        // A cap reached during the image search stops the searches after the skin is made.
        final class CapOnSearch: FakeBackend, @unchecked Sendable {
            override func search(_ query: String) async throws -> [ImageCandidateDTO] {
                _ = try await super.search(query)
                throw CoreLLMError.spendCap(scope: .user, spentCents: 200, capCents: 200)
            }
        }
        access.hasSearchKey = true
        let capped = CapOnSearch(reply(.new, queries: ["a", "b"]))
        let t = session(capped)
        await run(t, "a beach skin")
        XCTAssertEqual(capped.searches, ["a"])
        XCTAssertEqual(t.turns.last?.text, SkinChatSession.spendCapLine)
    }

    // MARK: catalogue

    func testCatalogueListsLoaderValues() throws {
        let catalogue = try XCTUnwrap(JSONSerialization.jsonObject(with: SkinCatalogue.json()) as? [String: Any])
        let palette = try XCTUnwrap(catalogue["palette"] as? [String: Any])
        XCTAssertEqual(palette["tokens"] as? [String], ThemeToken.allCases.map(\.rawValue))
        XCTAssertEqual((catalogue["fonts"] as? [String: Any])?["families"] as? [String], SkinFamily.allCases.map(\.rawValue))
        XCTAssertEqual(catalogue["archetypes"] as? [String], SkinArchetype.allCases.map(\.rawValue))
        let voice = try XCTUnwrap((catalogue["voice"] as? [String: Any])?["keys"] as? [String: [String: Int]])
        XCTAssertEqual(voice["tagline"]?["limit"], SkinVoiceKey.tagline.limit)
        XCTAssertEqual(voice.count, SkinVoiceKey.allCases.count)
    }
}
