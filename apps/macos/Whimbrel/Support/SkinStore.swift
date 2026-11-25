import Foundation
import SwiftUI

/// The installed skins and the selected one, with its palette. A skin's palettes are themes, so
/// the palette itself lives in `ThemeStore` (which keeps the old `themeId` key current); this
/// store adds which skin is chosen. Main thread only (`selected` and `withOverride` trap off-main in Debug). `revision` is observed next to
/// `ThemeStore.revision` by `ThemedRoot`. See docs/superpowers/specs/2026-10-20-skins-design.md.
final class SkinStore: ObservableObject {
    static let shared = SkinStore(movesToThread: true)
    static let skinKey = "com.mpointer.kestrel.skinId"
    static let paletteKey = "com.mpointer.kestrel.skinPaletteId"

    private let defaults: UserDefaults
    private let themes: ThemeStore
    private let userSkins: UserSkinStore?
    private var installed: [Skin] = [SkinDefaults.whimbrel]
    private var storedSkinId: String
    private var storedPaletteId: String
    private var override: Skin?
    @Published private(set) var revision = 0

    /// `userSkins` nil means `UserSkinStore.shared`, resolved only when user skins are loaded. `movesToThread` is the once-per-device
    /// move from the cream default to Thread (`ThreadMove`); only the app's own store (`shared`) asks for it, so a store made for a
    /// test starts where its stored values say.
    init(defaults: UserDefaults = .standard, themes: ThemeStore = .shared, userSkins: UserSkinStore? = nil, movesToThread: Bool = false) {
        self.defaults = defaults
        self.themes = themes
        self.userSkins = userSkins
        if let skin = defaults.string(forKey: Self.skinKey) {
            // The built-in skin was renamed with the product (rename design step 4b); a
            // choice stored under its old id keeps pointing at it and is rewritten once.
            storedSkinId = skin == "kestrel" ? SkinDefaults.whimbrel.id : skin
            if storedSkinId != skin { defaults.set(storedSkinId, forKey: Self.skinKey) }
            storedPaletteId = defaults.string(forKey: Self.paletteKey) ?? ThemeDefaults.theme.id
        } else {
            // First launch with skins: carry the old theme choice over (the old key stays, so a downgrade works).
            let old = defaults.string(forKey: ThemeStore.selectedKey) ?? ThemeDefaults.theme.id
            (storedSkinId, storedPaletteId) = Self.migrated(oldThemeId: old)
            defaults.set(storedSkinId, forKey: Self.skinKey)
            defaults.set(storedPaletteId, forKey: Self.paletteKey)
        }
        // Before anything reads the selection. A person who never picked a skin, or picked Classic with its first palette, is
        // moved to Thread once on this device; the marker keeps a later pick of Classic from being moved again.
        if movesToThread, !defaults.bool(forKey: ThreadMove.doneKey) {
            if let move = ThreadMove.decide(skinId: storedSkinId, paletteId: storedPaletteId, done: false) {
                (storedSkinId, storedPaletteId) = (move.skin, move.palette)
                defaults.set(storedSkinId, forKey: Self.skinKey)
                defaults.set(storedPaletteId, forKey: Self.paletteKey)
            }
            defaults.set(true, forKey: ThreadMove.doneKey)
        }
    }

    static func migrated(oldThemeId: String) -> (skin: String, palette: String) {
        switch oldThemeId {
        case "default", "slate", "sepia": return (SkinDefaults.whimbrel.id, oldThemeId)
        case "high-contrast": return ("high-contrast", "high-contrast")
        default: return (SkinDefaults.whimbrel.id, ThemeDefaults.theme.id)
        }
    }

    var skins: [Skin] { installed }

    /// The stored skin, or Whimbrel when its file is gone (the stored id is kept in case it returns).
    var selected: Skin {
        #if DEBUG
        dispatchPrecondition(condition: .onQueue(.main))
        #endif
        return override ?? installed.first { $0.id == storedSkinId } ?? SkinDefaults.whimbrel
    }

    /// The stored palette when the skin still offers it, else the skin's first one.
    var selectedPaletteId: String {
        if override != nil { return themes.selectedId }
        let ids = palettes(for: selected).map(\.id)
        let stored = installed.contains { $0.id == storedSkinId } ? storedPaletteId : nil
        return stored.flatMap { ids.contains($0) ? $0 : nil } ?? ids.first ?? ThemeDefaults.theme.id
    }

    /// The installed palette with `id` (Default is always known).
    func palette(id: String) -> Theme? {
        themes.themes.first { $0.id == id } ?? (id == ThemeDefaults.theme.id ? ThemeDefaults.theme : nil)
    }

    /// The skin's palettes that the theme store knows, in manifest order; Default when none do.
    func palettes(for skin: Skin) -> [Theme] {
        let known = themes.themes
        let found = skin.paletteIds.compactMap { id in known.first { $0.id == id } }
        return found.isEmpty ? [ThemeDefaults.theme] : found
    }

    /// Replaces the installed skins. Whimbrel is always first and always present.
    func install(_ skins: [Skin], base: Skin = SkinDefaults.whimbrel) {
        installed = [base] + skins.filter { $0.id != base.id }
        SkinImageResolver.invalidate()
        applyPalette()
        revision += 1
    }

    /// Installs the bundled themes, bundled and user palettes, then the bundled and user skins.
    /// A stored skin that is missing keeps its id, in case it returns (launch).
    func installBundledAndUserSkins() {
        let user = (userSkins ?? .shared).loadSkinsAndPalettes(base: SkinDefaults.whimbrel)
        var palettes = ThemeStore.bundledThemesDirectory.map { ThemeLoader.loadDirectory($0) } ?? []
        if let dir = Self.bundledSkinsDirectory?.appendingPathComponent("palettes") { palettes += ThemeLoader.loadDirectory(dir) }
        themes.install(palettes + user.palettes)
        let bundled = Self.bundledSkinsDirectory.map { SkinLoader.loadDirectory($0) } ?? []
        install(bundled + user.skins, base: bundled.first ?? SkinDefaults.whimbrel)
    }

    /// Re-installs bundled and user skins after a user skin changed; keeps the selection when it
    /// still exists, else selects Whimbrel with its first palette (Default).
    func reloadUserSkins() {
        installBundledAndUserSkins()
        if !installed.contains(where: { $0.id == storedSkinId }) { select(skinId: SkinDefaults.whimbrel.id, paletteId: nil) }
    }

    /// `paletteId` nil (or not one of the skin's) picks the skin's first palette. Unknown skins are ignored.
    func select(skinId: String, paletteId: String?) {
        guard let skin = installed.first(where: { $0.id == skinId }) else { return }
        let ids = palettes(for: skin).map(\.id)
        let palette = paletteId.flatMap { ids.contains($0) ? $0 : nil } ?? ids[0]
        storedSkinId = skinId
        storedPaletteId = palette
        defaults.set(skinId, forKey: Self.skinKey)
        defaults.set(palette, forKey: Self.paletteKey)
        applyPalette()
        revision += 1
    }

    /// Set by a gallery click or a skin chat action, whose skin change rebuilds the Settings window;
    /// read once by the rebuilt window so it scrolls back there instead of the top.
    private var settingsScrollTarget: String?

    /// A gallery card's scroll and accessibility id.
    static func galleryCardId(_ skinId: String) -> String { "skin-card-\(skinId)" }

    /// `select` on behalf of the Settings gallery.
    func selectFromGallery(skinId: String, paletteId: String?) {
        select(skinId: skinId, paletteId: paletteId)
        settingsScrollTarget = Self.galleryCardId(selected.id)
    }

    /// The next Settings rebuild scrolls to `id` (the skin chat marks its actions, whose results
    /// rebuild the window later).
    func scrollSettingsBack(to id: String) {
        settingsScrollTarget = id
    }

    /// The scroll id set by the latest gallery click or chat action, then nil until the next one.
    func takeGalleryScrollTarget() -> String? {
        defer { settingsScrollTarget = nil }
        return settingsScrollTarget
    }

    /// Makes the theme store show the resolved palette (it can differ from the stored one after a fallback).
    private func applyPalette() {
        let id = selectedPaletteId
        if themes.selectedId != id { themes.select(id) }
    }

    /// Runs `body` with `skin` and its palette in force and restores the previous ones afterwards,
    /// even when `body` throws. Nothing is stored and `revision` does not change, so the live app
    /// is unaffected; for rendering previews.
    func withOverride<T>(skin: Skin, paletteId: String, _ body: () throws -> T) rethrows -> T {
        #if DEBUG
        dispatchPrecondition(condition: .onQueue(.main))
        #endif
        let previous = override
        override = skin
        defer { override = previous }
        return try themes.withOverride(paletteId: paletteId, body)
    }

    /// `skins/` inside the app bundle (Contents/Resources/skins), or nil when it is missing.
    static var bundledSkinsDirectory: URL? {
        Bundle.main.url(forResource: "skins", withExtension: nil)
    }
}

/// The redesign switch. It is on for everyone: the stored value is no longer read, and there is no way back to the old layout.
/// The type and its call sites stay until the clean-up (switch-removal.md, part B), which deletes the old branches and then this.
/// The Thread skin ships as `skins/thread.json` and is listed in the gallery (`SkinGallery.orderedCards`); `ThreadMove` is the
/// one-time move of the cream default to it. Windows has the same switch, also on.
enum RedesignFlag {
    /// The old defaults key. Not read any more; an old value left in the file does nothing.
    static let key = "com.mpointer.whimbrel.redesign.enabled"

    /// Bundled skins that were hidden while the switch could be off (core/skins `REDESIGN_SKIN_IDS`).
    static let skinIds: Set<String> = ["thread"]

    /// Always true. The parameter stays so call sites need no change before the clean-up.
    static func isOn(_ defaults: UserDefaults = .standard) -> Bool { true }

    /// `skins` as a list shows them: without the redesign skins unless `redesign` is on.
    static func visible(_ skins: [Skin], redesign: Bool) -> [Skin] {
        redesign ? skins : skins.filter { !skinIds.contains($0.id) }
    }
}

/// The once-per-device move from the cream default to Thread (switch-removal.md, A2). Pure; `SkinStore.init` applies it.
enum ThreadMove {
    /// Set once the check has run on this device, whether or not it moved anyone. Not synced.
    static let doneKey = "com.mpointer.whimbrel.skin.threadMoveDone"
    static let skinId = "thread"
    static let paletteId = "thread"

    /// Where a person with this stored choice goes, or nil to leave them. Classic (skin `whimbrel`) with its first palette
    /// (`default`) moves; Slate and Sepia, every other skin, a user's own skin and High Contrast stay. Nothing moves twice.
    static func decide(skinId stored: String, paletteId: String, done: Bool) -> (skin: String, palette: String)? {
        guard !done, stored == SkinDefaults.whimbrel.id, paletteId == ThemeDefaults.theme.id else { return nil }
        return (skinId, Self.paletteId)
    }
}

private struct RedesignEnabledKey: EnvironmentKey {
    static let defaultValue = true
}

extension EnvironmentValues {
    /// The redesign switch (`RedesignFlag`); on, also in a root that never sets it.
    var redesignEnabled: Bool {
        get { self[RedesignEnabledKey.self] }
        set { self[RedesignEnabledKey.self] = newValue }
    }
}
