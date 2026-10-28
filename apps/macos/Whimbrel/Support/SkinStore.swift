import Foundation
import SwiftUI

/// The installed skins and the selected one, with its palette. A skin's palettes are themes, so
/// the palette itself lives in `ThemeStore` (which keeps the old `themeId` key current); this
/// store adds which skin is chosen. Main thread only (`selected` and `withOverride` trap off-main in Debug). `revision` is observed next to
/// `ThemeStore.revision` by `ThemedRoot`. See docs/superpowers/specs/2026-10-20-skins-design.md.
final class SkinStore: ObservableObject {
    static let shared = SkinStore()
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

    /// `userSkins` nil means `UserSkinStore.shared`, resolved only when user skins are loaded.
    init(defaults: UserDefaults = .standard, themes: ThemeStore = .shared, userSkins: UserSkinStore? = nil) {
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
    func install(_ skins: [Skin]) {
        installed = [SkinDefaults.whimbrel] + skins.filter { $0.id != SkinDefaults.whimbrel.id }
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
        install((Self.bundledSkinsDirectory.map { SkinLoader.loadDirectory($0) } ?? []) + user.skins)
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
