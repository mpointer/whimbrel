import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

/// Replies with `reply`, finds three images per search and serves a solid-colour PNG for any fetch.
private final class StubBackend: SkinTurnBackend, @unchecked Sendable {
    var reply: SkinTurnReplyDTO
    init(_ reply: SkinTurnReplyDTO) { self.reply = reply }

    func turn(_ input: Data, provider: RoutingSettings.Provider, model: String) async throws -> SkinTurnReplyDTO { reply }

    func search(_ query: String) async throws -> [ImageCandidateDTO] {
        (1...3).map { ImageCandidateDTO(url: "https://img.test/\($0).png", thumbnailUrl: "https://img.test/t\($0).png", title: "\(query) \($0)", source: "web") }
    }

    func generate(_ prompt: String) async throws -> Data { Self.png(.systemTeal) }

    func fetch(_ url: String) async throws -> Data {
        Self.png(url.hasSuffix("1.png") ? .systemOrange : url.hasSuffix("2.png") ? .systemBlue : .systemGreen)
    }

    static func png(_ color: NSColor) -> Data {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 8, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        color.setFill()
        NSRect(x: 0, y: 0, width: 8, height: 8).fill()
        NSGraphicsContext.restoreGraphicsState()
        return rep.representation(using: .png, properties: [:])!
    }
}

@MainActor
final class SkinChatViewTests: XCTestCase {
    private var root: URL!
    private var users: UserSkinStore!
    private var skins: SkinStore!
    private var access = SkinChatAccess(provider: .ollama, model: "gemma", localOnly: false, hasKey: true, hasSearchKey: true, hasImageKey: false)

    override func setUp() async throws {
        let suite = "skin-chat-view-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        root = FileManager.default.temporaryDirectory.appendingPathComponent("skin-chat-view-\(UUID().uuidString)")
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

    private func skin(_ id: String, _ name: String) -> Skin {
        var s = SkinDefaults.whimbrel
        s.id = id
        s.name = name
        return s
    }

    private func reply(_ intent: SkinTurnReplyDTO.Intent, queries: [String] = []) -> SkinTurnReplyDTO {
        let manifest = try! JSONSerialization.data(withJSONObject: ["id": "x", "name": "Beach", "palettes": ["sand"]])
        // Passes every archetype's pairs (`ArchetypeContrast`), so saving it repairs nothing and adds no chat line.
        let palette = try! JSONSerialization.data(withJSONObject: ["id": "sand", "name": "Sand", "light": ["backgroundPage": "#fdf6e3", "textPrimary": "#222222", "textSecondary": "#444444"]])
        return SkinTurnReplyDTO(intent: intent, reply: "Done. Sandy page, sea-blue accents.",
                                notes: [.init(step: "palette", text: "Sandy page, sea accents"), .init(step: "type", text: "Rounded type for a relaxed feel")],
                                skin: manifest, palettes: [palette], imageQueries: queries)
    }

    private func session(_ backend: StubBackend) -> SkinChatSession {
        SkinChatSession(skinStore: skins, userSkins: users, backend: backend, routing: RoutingSettings(), access: { [unowned self] in self.access })
    }

    // MARK: helpers

    func testOrderedCardsPutsBuiltInsFirstThenUserSkinsByName() {
        let ordered = SkinGallery.orderedCards(skins: [
            skin("whimbrel", "Whimbrel"), skin("user-zed-1", "Zed"), skin("retro-terminal", "Retro Terminal"),
            skin("user-alpha-1", "Alpha"), skin("almanac", "Almanac"),
        ])
        XCTAssertEqual(ordered.map(\.id), ["whimbrel", "retro-terminal", "almanac", "user-alpha-1", "user-zed-1"],
                       "built-ins keep their order; yours follow, by name")
    }

    func testIsUserSkin() {
        XCTAssertTrue(SkinGallery.isUserSkin(skin("user-beach-1", "Beach")))
        XCTAssertFalse(SkinGallery.isUserSkin(skin("whimbrel", "Whimbrel")))
        XCTAssertFalse(SkinGallery.isUserSkin(skin("retro-terminal", "Retro Terminal")))
    }

    func testPlaceholderFollowsTheSelectedSkin() {
        XCTAssertEqual(SkinChatView.placeholder(for: skin("whimbrel", "Whimbrel")), "Describe a skin…")
        XCTAssertEqual(SkinChatView.placeholder(for: skin("user-beach-1", "Beach")), "Ask for a change…")
    }

    // MARK: ownership

    /// A turn that switches the skin bumps the revision (ThemedRoot rebuilds Settings) and leaves its
    /// chat state on the session, which `AppState.skinChat` keeps alive across that rebuild. (AppState
    /// itself starts live services and can't be built in tests, so the lazy var is not covered here.)
    func testTurnThatSwitchesSkinBumpsRevisionAndKeepsChatState() async throws {
        let s = session(StubBackend(reply(.new, queries: ["beach"])))
        let before = skins.revision
        s.send("a beach skin")
        await s.current?.value
        XCTAssertGreaterThan(skins.revision, before, "the new skin switched on, so ThemedRoot rebuilds")
        XCTAssertEqual(skins.selected.id, "user-beach-1")

        let rebuilt = SkinChatView(session: s, setUp: {})
        XCTAssertEqual(rebuilt.session.turns.map(\.text), ["a beach skin", "Done. Sandy page, sea-blue accents."])
        XCTAssertEqual(rebuilt.session.candidates.count, 3)
        XCTAssertEqual(rebuilt.session.status.first, "Reading your request…")
    }

    func testChatActionsScrollTheRebuiltSettingsBackToTheChat() {
        XCTAssertNil(skins.takeGalleryScrollTarget())
        skins.scrollSettingsBack(to: SkinChatView.scrollId)
        XCTAssertEqual(skins.takeGalleryScrollTarget(), SkinChatView.scrollId)
        XCTAssertNil(skins.takeGalleryScrollTarget(), "taken once")
        skins.scrollSettingsBack(to: SkinChatView.scrollId)
        skins.selectFromGallery(skinId: "whimbrel", paletteId: nil)
        XCTAssertEqual(skins.takeGalleryScrollTarget(), SkinStore.galleryCardId("whimbrel"), "the latest action wins")
    }

    /// The scroll target is set only where the session reloads skins, so a turn that changes nothing
    /// can't leave a stale target for a later, unrelated rebuild.
    func testTurnsThatChangeNothingLeaveNoScrollTarget() async throws {
        var question = reply(.clarify)
        question.reply = "Which colours?"
        let backend = StubBackend(question)
        let s = session(backend)
        s.send("make me a skin")
        await s.current?.value
        XCTAssertNil(skins.takeGalleryScrollTarget(), "a question back changes no skin")

        var unusable = reply(.new)
        unusable.skin = nil
        backend.reply = unusable
        s.send("a beach skin")
        await s.current?.value
        XCTAssertNil(skins.takeGalleryScrollTarget(), "an unusable reply changes no skin")

        s.undo()
        XCTAssertNil(skins.takeGalleryScrollTarget(), "nothing to undo on a built-in")
    }

    func testTurnThatSavesASkinScrollsSettingsBackToTheChat() async throws {
        let s = session(StubBackend(reply(.new)))
        s.send("a beach skin")
        await s.current?.value
        XCTAssertEqual(skins.takeGalleryScrollTarget(), SkinChatView.scrollId)
    }

    // MARK: snapshot

    func testSnapshotThreeTurnsFourStatusLinesThreeCandidates() async throws {
        // One saved turn on your own skin, then a refine that asks for a photo: 3 turns; status is
        // "Reading…", two notes and one search; the search finds 3.
        let id = "user-beach-1"
        let manifest = try JSONSerialization.data(withJSONObject: ["id": id, "name": "Beach", "palettes": [ThemeDefaults.theme.id]])
        _ = try users.save(manifest: manifest, palettes: [], for: id)
        try users.appendChat(SkinChatTurn(role: "whimbrel", text: "Here's your skin.", date: Date()), to: id)
        skins.reloadUserSkins()
        skins.select(skinId: id, paletteId: nil)

        let s = session(StubBackend(reply(.refine, queries: ["beach"])))
        s.send("add a beach photo")
        await s.current?.value
        XCTAssertEqual(s.turns.count, 3)
        XCTAssertEqual(s.status.count, 4)
        XCTAssertEqual(s.candidates.count, 3)
        for c in s.candidates { _ = await s.thumbnail(for: c) }

        let view = SkinChatView(session: s, setUp: {})
            .padding(16)
            .frame(width: 640, height: 520, alignment: .topLeading)
            .background(Color.whimbrelBackgroundPage)
        // Your skin in force (so the field asks for a change), drawn in Whimbrel's colours.
        try SkinStore.shared.withOverride(skin: skins.selected, paletteId: ThemeDefaults.theme.id) {
            try assertSnapshot(view, named: "skin-chat-light", size: CGSize(width: 640, height: 520), colorScheme: .light)
        }
    }
}
