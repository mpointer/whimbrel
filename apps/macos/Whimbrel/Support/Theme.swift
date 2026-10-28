import Foundation
import OSLog

/// The color tokens a theme can set (raw value = the JSON key). See
/// docs/superpowers/specs/2026-10-06-themes-design.md.
enum ThemeToken: String, CaseIterable {
    case backgroundPage, backgroundCard, border
    case textPrimary, textSecondary
    case accentPrimary, accentSecondary, accentHighlight
    case error, warning, success, link
    case selection, highlight, textOnAccent
}

typealias ThemePalette = [ThemeToken: UInt32]

/// Which variants a theme file ships. A one-variant theme serves both appearances with it, so the
/// app forces the matching system appearance while it is selected (macOS `AppearanceSettings`).
enum ThemeVariants: Equatable { case both, lightOnly, darkOnly }

struct Theme: Equatable, Identifiable {
    let id: String
    let name: String
    let author: String
    /// Both palettes always hold all 15 tokens (missing ones were filled from Default).
    let light: ThemePalette
    let dark: ThemePalette
    var variants: ThemeVariants = .both
}

enum ThemeContrast {
    private static func channel(_ value: UInt32) -> Double {
        let c = Double(value) / 255
        return c <= 0.03928 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
    }

    static func relativeLuminance(_ hex: UInt32) -> Double {
        0.2126 * channel((hex >> 16) & 0xFF) + 0.7152 * channel((hex >> 8) & 0xFF) + 0.0722 * channel(hex & 0xFF)
    }

    /// WCAG contrast ratio, 1...21.
    static func ratio(_ a: UInt32, _ b: UInt32) -> Double {
        let la = relativeLuminance(a), lb = relativeLuminance(b)
        return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)
    }

    static let minimum = 4.5

    /// (foreground, background) pairs a variant must keep readable.
    static let requiredPairs: [(ThemeToken, ThemeToken)] = [
        (.textPrimary, .backgroundPage), (.textPrimary, .backgroundCard),
        (.textSecondary, .backgroundPage), (.textSecondary, .backgroundCard),
        (.textPrimary, .selection), (.textPrimary, .highlight),
        (.textOnAccent, .accentPrimary),
    ]

    /// One line per failing pair, e.g. `textOnAccent on accentPrimary = 3.26`.
    static func failures(_ palette: ThemePalette) -> [String] {
        requiredPairs.compactMap { fg, bg in
            guard let f = palette[fg], let b = palette[bg] else { return "\(fg.rawValue) on \(bg.rawValue) = missing" }
            let r = ratio(f, b)
            return r < minimum ? "\(fg.rawValue) on \(bg.rawValue) = \(String(format: "%.2f", r))" : nil
        }
    }

    static func passes(_ palette: ThemePalette) -> Bool { failures(palette).isEmpty }
}

enum ThemeLoader {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "themes")

    static func isValidId(_ id: String) -> Bool {
        !id.isEmpty && id.allSatisfy { ($0 >= "a" && $0 <= "z") || ($0 >= "0" && $0 <= "9") || $0 == "-" }
    }

    /// `#rrggbb` only.
    private static func hex(_ value: Any?) -> UInt32? {
        guard let s = value as? String, s.count == 7, s.hasPrefix("#") else { return nil }
        return UInt32(s.dropFirst(), radix: 16)
    }

    /// nil when any known token's value is not `#rrggbb`; unknown keys are ignored. Not contrast-checked.
    static func variant(_ raw: [String: Any]) -> ThemePalette? {
        var palette: ThemePalette = [:]
        for token in ThemeToken.allCases {
            guard let value = raw[token.rawValue] else { continue }
            guard let color = hex(value) else { return nil }
            palette[token] = color
        }
        return palette
    }

    /// A theme file → a complete `Theme`, or nil (logged) when the file is unusable. A variant
    /// that fails the contrast guard is replaced by `fallback`'s variant of the same kind.
    static func parse(_ data: Data, fallback: Theme = ThemeDefaults.theme) -> Theme? {
        guard let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let id = root["id"] as? String, isValidId(id),
              let name = root["name"] as? String, !name.isEmpty else {
            log.error("theme skipped: not a JSON object with a valid id and name")
            return nil
        }
        let lightRaw = root["light"] as? [String: Any]
        let darkRaw = root["dark"] as? [String: Any]
        guard lightRaw != nil || darkRaw != nil else {
            log.error("theme \(id, privacy: .public) skipped: no light or dark variant")
            return nil
        }
        var light: ThemePalette?
        var dark: ThemePalette?
        if let lightRaw {
            guard let parsed = variant(lightRaw) else {
                log.error("theme \(id, privacy: .public) skipped: a light color is not #rrggbb")
                return nil
            }
            light = fallback.light.merging(parsed) { _, new in new }
        }
        if let darkRaw {
            guard let parsed = variant(darkRaw) else {
                log.error("theme \(id, privacy: .public) skipped: a dark color is not #rrggbb")
                return nil
            }
            dark = fallback.dark.merging(parsed) { _, new in new }
        }
        // A theme that ships one variant serves both appearances.
        var lightPalette = light ?? dark
        var darkPalette = dark ?? light
        if let p = lightPalette, !ThemeContrast.passes(p) {
            log.error("theme \(id, privacy: .public): light variant fails the contrast guard, using Default's: \(ThemeContrast.failures(p).joined(separator: "; "), privacy: .public)")
            lightPalette = fallback.light
        }
        if let p = darkPalette, !ThemeContrast.passes(p) {
            log.error("theme \(id, privacy: .public): dark variant fails the contrast guard, using Default's: \(ThemeContrast.failures(p).joined(separator: "; "), privacy: .public)")
            darkPalette = fallback.dark
        }
        guard let lightPalette, let darkPalette else { return nil }
        let variants: ThemeVariants = lightRaw == nil ? .darkOnly : darkRaw == nil ? .lightOnly : .both
        return Theme(id: id, name: name, author: root["author"] as? String ?? "", light: lightPalette, dark: darkPalette, variants: variants)
    }

    /// Every `*.json` directly inside `url` (not recursive), Default first, then by name;
    /// broken files and duplicate ids are skipped. Always contains Default.
    static func loadDirectory(_ url: URL, fallback: Theme = ThemeDefaults.theme) -> [Theme] {
        let files = (try? FileManager.default.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)) ?? []
        var seen: Set<String> = [fallback.id]
        var loaded: [Theme] = []
        for file in files.filter({ $0.pathExtension == "json" }).sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
            guard let data = try? Data(contentsOf: file), let theme = parse(data, fallback: fallback) else {
                log.error("theme file skipped: \(file.lastPathComponent, privacy: .public)")
                continue
            }
            guard seen.insert(theme.id).inserted else { continue }
            loaded.append(theme)
        }
        return [fallback] + loaded.sorted { $0.name < $1.name }
    }
}
