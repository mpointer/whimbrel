import AppKit
import OSLog

/// Parses `skins/<id>.json` manifests. A file is skipped (logged) when it is not JSON or lacks a
/// valid, unique id and a name; any field-level problem is repaired from `base` and reported.
enum SkinLoader {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")

    /// The manifest keys `parse` reads; any other top-level key is ignored.
    static let topLevelKeys: Set<String> = ["id", "name", "author", "palettes", "typography", "shape", "icons", "decoration", "voice", "today", "motion"]

    static func parse(_ data: Data, base: Skin) -> (Skin, repairs: [String])? {
        guard let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let id = root["id"] as? String, ThemeLoader.isValidId(id),
              let name = root["name"] as? String, !name.isEmpty else {
            log.error("skin skipped: not a JSON object with a valid id and name")
            return nil
        }
        var repairs: [String] = []
        func repair(_ message: String) {
            repairs.append(message)
            log.error("skin \(id, privacy: .public): \(message, privacy: .public)")
        }
        /// An enum value by raw string; `fallback` (reported) when present but unknown.
        func pick<E: RawRepresentable>(_ raw: [String: Any]?, _ key: String, _ fallback: E) -> E where E.RawValue == String {
            guard let value = raw?[key] else { return fallback }
            if let s = value as? String, let e = E(rawValue: s) { return e }
            repair("\(key) is not a known value, using \(fallback.rawValue)")
            return fallback
        }
        func number(_ raw: [String: Any]?, _ key: String, _ fallback: CGFloat, _ range: ClosedRange<CGFloat>) -> CGFloat {
            guard let value = raw?[key] else { return fallback }
            guard CFGetTypeID(value as CFTypeRef) != CFBooleanGetTypeID(), let d = value as? Double else { repair("\(key) is not a number, using \(fallback)"); return fallback }
            let clamped = min(max(CGFloat(d), range.lowerBound), range.upperBound)
            if clamped != CGFloat(d) { repair("\(key) \(d) is outside \(range), clamped to \(clamped)") }
            return clamped
        }
        func face(_ raw: [String: Any]?, _ key: String, _ fallback: SkinFace) -> SkinFace {
            guard let value = raw?[key] else { return fallback }
            if let s = value as? String, let f = SkinFace(s) { return f }
            repair("\(key) font is not allowed, using the Whimbrel font")
            return fallback
        }

        var skin = base
        skin.id = id
        skin.name = name
        skin.author = root["author"] as? String ?? ""

        let palettes = (root["palettes"] as? [Any])?.compactMap { $0 as? String }.filter(ThemeLoader.isValidId) ?? []
        if palettes.isEmpty { repair("no usable palettes, using the Whimbrel palette") } else { skin.paletteIds = palettes }

        let typography = root["typography"] as? [String: Any]
        skin.typography = SkinTypography(
            display: face(typography, "display", base.typography.display),
            body: face(typography, "body", base.typography.body),
            numerals: face(typography, "numerals", base.typography.numerals),
            label: face(typography, "label", base.typography.label),
            caps: pick(typography, "caps", base.typography.caps))

        let shape = root["shape"] as? [String: Any]
        skin.shape = SkinShape(
            radius: number(shape, "radius", base.shape.radius, SkinShape.radiusRange),
            border: pick(shape, "border", base.shape.border),
            borderWidth: number(shape, "borderWidth", base.shape.borderWidth, SkinShape.widthRange),
            density: pick(shape, "density", base.shape.density))

        let icons = root["icons"] as? [String: Any]
        for role in SkinIconRole.allCases {
            guard let value = icons?[role.rawValue] else { continue }
            if let s = value as? String, NSImage(systemSymbolName: s, accessibilityDescription: nil) != nil {
                skin.icons.symbols[role] = s
            } else {
                repair("icon \(role.rawValue) is not an SF Symbol, using the Whimbrel icon")
            }
        }

        let decoration = root["decoration"] as? [String: Any]
        skin.decoration = SkinDecoration(
            headerBand: pick(decoration, "headerBand", base.decoration.headerBand),
            dividers: pick(decoration, "dividers", base.decoration.dividers),
            texture: pick(decoration, "texture", base.decoration.texture),
            imagery: (decoration?["imagery"] as? [Any] ?? []).compactMap { item in
                // The asset is a bundled file name, never a path.
                guard let d = item as? [String: Any], let asset = d["asset"] as? String, !asset.isEmpty,
                      !asset.contains("/"), !asset.hasPrefix("."),
                      let placement = (d["placement"] as? String).flatMap(SkinImagePlacement.init) else {
                    repair("an imagery entry needs a plain file name asset and a placement, dropped")
                    return nil
                }
                return SkinImage(asset: asset, placement: placement, textFree: d["textFree"] as? Bool ?? false)
            })

        let voice = root["voice"] as? [String: Any]
        for key in SkinVoiceKey.allCases {
            guard let value = voice?[key.rawValue] else { continue }
            if let s = value as? String, key.fits(s) {
                skin.voice.values[key] = s
            } else {
                repair("voice \(key.rawValue) is empty, not text or over \(key.limit) characters, using the Whimbrel wording")
            }
        }

        let today = root["today"] as? [String: Any]
        skin.archetype = pick(today, "archetype", base.archetype)
        skin.archetypeOptions = (today?["options"] as? [String: Any])?.compactMapValues { $0 as? String } ?? [:]
        skin.motion = pick(root, "motion", base.motion)
        return (skin, repairs)
    }

    /// Every `*.json` directly inside `url` (not recursive, so `skins/invalid/` is never loaded),
    /// `base` first, then by name; broken files and duplicate ids are skipped.
    static func loadDirectory(_ url: URL, base: Skin = SkinDefaults.whimbrel) -> [Skin] {
        let files = (try? FileManager.default.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)) ?? []
        var seen: Set<String> = [base.id]
        var loaded: [Skin] = []
        for file in files.filter({ $0.pathExtension == "json" }).sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
            guard let data = try? Data(contentsOf: file), let (skin, _) = parse(data, base: base) else {
                log.error("skin file skipped: \(file.lastPathComponent, privacy: .public)")
                continue
            }
            if skin.id == base.id { continue }  // e.g. whimbrel.json mirrors the base
            guard seen.insert(skin.id).inserted else {
                log.error("skin file skipped, duplicate id: \(file.lastPathComponent, privacy: .public)")
                continue
            }
            loaded.append(skin)
        }
        return [base] + loaded.sorted { $0.name < $1.name }
    }
}
