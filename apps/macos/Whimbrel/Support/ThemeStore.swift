#if os(macOS)
import AppKit
#else
import UIKit
#endif
import SwiftUI

/// The installed themes and the selected one. Colors read it at render time (from any thread,
/// hence the lock); `select`/`install` are called on the main thread and bump `revision`, which
/// each window root observes to repaint. The selection is per device. See
/// docs/superpowers/specs/2026-10-06-themes-design.md.
final class ThemeStore: ObservableObject {
    static let shared = ThemeStore()
    static let selectedKey = "com.mpointer.kestrel.themeId"

    private let lock = NSLock()
    private let defaults: UserDefaults
    private var installed: [Theme] = [ThemeDefaults.theme]
    private var storedId: String
    private var overrideId: String?
    @Published private(set) var revision = 0

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        storedId = defaults.string(forKey: Self.selectedKey) ?? ThemeDefaults.theme.id
    }

    private func locked<T>(_ body: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body()
    }

    private var currentId: String {
        let id = overrideId ?? storedId
        return installed.contains { $0.id == id } ? id : ThemeDefaults.theme.id
    }

    var themes: [Theme] { locked { installed } }
    var selectedId: String { locked { currentId } }
    var selected: Theme { locked { installed.first { $0.id == currentId } ?? ThemeDefaults.theme } }

    /// Replaces the installed themes. Default is always first and always present.
    func install(_ themes: [Theme]) {
        locked {
            installed = [ThemeDefaults.theme] + themes.filter { $0.id != ThemeDefaults.theme.id }
        }
        revision += 1
    }

    func select(_ id: String) {
        let known = locked { installed.contains { $0.id == id } }
        guard known else { return }
        locked { storedId = id }
        defaults.set(id, forKey: Self.selectedKey)
        revision += 1
    }

    /// Runs `body` with `paletteId` in force (Default if unknown) and restores the previous one
    /// afterwards, even when `body` throws. Not persisted and `revision` does not change. The
    /// lock is not held while `body` runs, since it reads colors through the same lock; one
    /// override at a time, for rendering previews on the main thread.
    func withOverride<T>(paletteId: String, _ body: () throws -> T) rethrows -> T {
        let previous = locked { overrideId }
        locked { overrideId = paletteId }
        defer { locked { overrideId = previous } }
        return try body()
    }

    /// `themes/` inside the app bundle (Contents/Resources/themes), or nil when it is missing.
    static var bundledThemesDirectory: URL? {
        Bundle.main.url(forResource: "themes", withExtension: nil)
    }

    func hex(_ token: ThemeToken, isDark: Bool) -> UInt32 {
        let theme = selected
        let palette = isDark ? theme.dark : theme.light
        return palette[token] ?? (isDark ? ThemeDefaults.dark : ThemeDefaults.light)[token] ?? 0
    }
}

#if os(macOS)
extension NSColor {
    convenience init(srgbHex hex: UInt32) {
        self.init(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: 1
        )
    }
}

extension Color {
    /// A color that asks `ThemeStore.shared` for its value every time it is rendered, in the
    /// light or dark variant that matches the appearance it is drawn in.
    static func theme(_ token: ThemeToken) -> Color {
        Color(NSColor(name: nil, dynamicProvider: { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return NSColor(srgbHex: ThemeStore.shared.hex(token, isDark: isDark))
        }))
    }
}
#else
extension UIColor {
    convenience init(srgbHex hex: UInt32) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: 1
        )
    }
}

extension Color {
    /// iOS counterpart of the macOS `Color.theme` above, keyed on the trait collection's style.
    static func theme(_ token: ThemeToken) -> Color {
        Color(UIColor { traits in
            UIColor(srgbHex: ThemeStore.shared.hex(token, isDark: traits.userInterfaceStyle == .dark))
        })
    }
}
#endif
