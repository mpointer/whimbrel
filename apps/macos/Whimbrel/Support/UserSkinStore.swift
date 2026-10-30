import Foundation
import OSLog

/// One message in a skin's chat, stored with the skin so refining it later picks up where it left off.
struct SkinChatTurn: Codable, Equatable {
    let role: String  // "user" | "whimbrel"
    let text: String
    let date: Date
}

/// The skins you made, one folder each under `root` (`Application Support/Whimbrel/skins/<id>/`):
/// `skin.json`, `palettes/<id>.json`, `images/<flat name>`, `chat.json` and `history/<n>.json`
/// (the last 20 versions of manifest + palettes, for undo). Ids are `user-` prefixed so they never
/// collide with built-ins. Loading goes through `SkinLoader`/`ThemeLoader`, the same validation as
/// bundled skins. See docs/superpowers/specs/2026-10-21-skins-r2-conversation-design.md.
final class UserSkinStore {
    static var defaultRoot: URL { SpikePaths.root.appendingPathComponent("skins") }
    static let shared = UserSkinStore(root: defaultRoot)
    static let historyLimit = 20
    /// Longest skin name, slug and image name.
    static let nameLimit = 40

    /// `name` trimmed and cut so that, with `suffix` (a clash's " 2"), it is at most `nameLimit` characters.
    static func skinName(_ name: String, suffix: String = "") -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return String(trimmed.prefix(nameLimit - suffix.count)).trimmingCharacters(in: .whitespaces) + suffix
    }

    enum StoreError: Error { case invalidId(String), unreadableManifest(String), notSaved(String) }

    /// Held while a skins folder is written: a save, rename, undo, delete or image here, and a sync
    /// sending skins or writing other devices' into the folder (`SkinSync`), so neither sees the other half done.
    /// Recursive, since `rename` saves. Windows' `skin_maker::saving`.
    static let writing = NSRecursiveLock()
    /// Where deleted skins wait (core `skins::store::DELETED_DIR`), with the time of deletion in `.deleted-at`.
    static let deletedFolder = ".deleted"
    private static let deletedMarker = ".deleted-at"

    /// Called after each save, rename, undo and delete with the skin's id; the app sends it to sync
    /// (`SkinSync.connect`). Must not block: it runs while `writing` is held.
    var changed: ((String) -> Void)?

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")
    private let root: URL
    private let fm: FileManager

    var rootURL: URL { root }

    init(root: URL, fileManager: FileManager = .default) {
        self.root = root
        fm = fileManager
    }

    /// One saved version: the manifest and palette bytes exactly as written.
    private struct Version: Codable {
        struct Palette: Codable { var id: String; var data: Data }
        var manifest: Data
        var palettes: [Palette]
    }

    /// The skin's folder; ids become path components, so only `user-` ids of `a-z0-9-` pass.
    func folder(_ skinId: String) throws -> URL {
        guard skinId.hasPrefix("user-"), ThemeLoader.isValidId(skinId) else { throw StoreError.invalidId(skinId) }
        return root.appendingPathComponent(skinId, isDirectory: true)
    }

    func skinIds() -> [String] {
        let names = (try? fm.contentsOfDirectory(atPath: root.path)) ?? []
        return names.filter { (try? folder($0)) != nil }.sorted()
    }

    /// Writes the skin and appends it as a new version; returns the version number.
    func save(manifest: Data, palettes: [(id: String, data: Data)], for skinId: String) throws -> Int {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let dir = try folder(skinId)
        let version = Version(manifest: manifest, palettes: palettes.map { Version.Palette(id: $0.id, data: $0.data) })
        try writeCurrent(version, in: dir)
        let history = dir.appendingPathComponent("history", isDirectory: true)
        try fm.createDirectory(at: history, withIntermediateDirectories: true)
        var numbers = versionNumbers(history)
        let next = (numbers.last ?? 0) + 1
        try JSONEncoder().encode(version).write(to: history.appendingPathComponent("\(next).json"), options: .atomic)
        numbers.append(next)
        for old in numbers.dropLast(Self.historyLimit) {
            try? fm.removeItem(at: history.appendingPathComponent("\(old).json"))
        }
        changed?(skinId)
        return next
    }

    /// Drops the newest version and restores the one before it (manifest and palettes); false when there is none.
    func undo(skinId: String) throws -> Bool {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let history = try folder(skinId).appendingPathComponent("history", isDirectory: true)
        let numbers = versionNumbers(history)
        guard numbers.count >= 2 else { return false }
        let previous = try JSONDecoder().decode(Version.self, from: Data(contentsOf: history.appendingPathComponent("\(numbers[numbers.count - 2]).json")))
        try writeCurrent(previous, in: history.deletingLastPathComponent())
        try fm.removeItem(at: history.appendingPathComponent("\(numbers[numbers.count - 1]).json"))
        changed?(skinId)
        return true
    }

    /// `skin.json` and exactly these palettes (any others are removed).
    private func writeCurrent(_ version: Version, in dir: URL) throws {
        for palette in version.palettes where !ThemeLoader.isValidId(palette.id) { throw StoreError.invalidId(palette.id) }
        let palettes = dir.appendingPathComponent("palettes", isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        try version.manifest.write(to: dir.appendingPathComponent("skin.json"), options: .atomic)
        if fm.fileExists(atPath: palettes.path) { try fm.removeItem(at: palettes) }
        try fm.createDirectory(at: palettes, withIntermediateDirectories: true)
        for palette in version.palettes {
            try palette.data.write(to: palettes.appendingPathComponent("\(palette.id).json"), options: .atomic)
        }
    }

    private func versionNumbers(_ history: URL) -> [Int] {
        ((try? fm.contentsOfDirectory(atPath: history.path)) ?? [])
            .compactMap { $0.hasSuffix(".json") ? Int($0.dropLast(5)) : nil }.sorted()
    }

    private func currentPalettes(in dir: URL) -> [(id: String, data: Data)] {
        let palettes = dir.appendingPathComponent("palettes", isDirectory: true)
        return ((try? fm.contentsOfDirectory(atPath: palettes.path)) ?? []).sorted().compactMap { name in
            guard name.hasSuffix(".json"), let data = fm.contents(atPath: palettes.appendingPathComponent(name).path) else { return nil }
            return (id: String(name.dropLast(5)), data: data)
        }
    }

    /// The current `skin.json` bytes, or nil when the skin was never saved.
    func manifest(for skinId: String) -> Data? {
        (try? folder(skinId)).flatMap { fm.contents(atPath: $0.appendingPathComponent("skin.json").path) }
    }

    /// The current palette files, by id.
    func palettes(for skinId: String) -> [(id: String, data: Data)] {
        (try? folder(skinId)).map { currentPalettes(in: $0) } ?? []
    }

    // MARK: chat

    func chat(for skinId: String) -> [SkinChatTurn] {
        guard let url = try? folder(skinId).appendingPathComponent("chat.json"), let data = fm.contents(atPath: url.path) else { return [] }
        return (try? JSONDecoder().decode([SkinChatTurn].self, from: data)) ?? []
    }

    /// Refuses a skin that was never saved, so no folder ever holds only a chat.
    func appendChat(_ turn: SkinChatTurn, to skinId: String) throws {
        let dir = try folder(skinId)
        guard fm.fileExists(atPath: dir.appendingPathComponent("skin.json").path) else { throw StoreError.notSaved(skinId) }
        try JSONEncoder().encode(chat(for: skinId) + [turn]).write(to: dir.appendingPathComponent("chat.json"), options: .atomic)
    }

    // MARK: images

    /// Writes `images/<flat name>` (lower-case `a-z0-9.-`, at most `nameLimit` before a `-2`, `-3`...
    /// that makes it unique; a URL's query and fragment are dropped); returns the asset name.
    func addImage(_ data: Data, preferredName: String, to skinId: String) throws -> String {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let images = try folder(skinId).appendingPathComponent("images", isDirectory: true)
        try fm.createDirectory(at: images, withIntermediateDirectories: true)
        let path = preferredName.prefix { $0 != "?" && $0 != "#" }
        let flat = String((String(path) as NSString).lastPathComponent.lowercased()
            .map { ($0 >= "a" && $0 <= "z") || ($0 >= "0" && $0 <= "9") || $0 == "." ? $0 : "-" })
            .drop { $0 == "." }
        // A long "extension" is just part of the name (and would leave no room for the stem).
        let fullExt = (String(flat) as NSString).pathExtension
        let ext = fullExt.count <= 8 ? fullExt : ""
        let whole = ext.isEmpty ? String(flat) : (String(flat) as NSString).deletingPathExtension
        let stem = String(whole.prefix(Self.nameLimit - (ext.isEmpty ? 0 : ext.count + 1)))
        let base = stem.isEmpty ? "image" : stem
        var name = ext.isEmpty ? base : "\(base).\(ext)"
        var n = 2
        while fm.fileExists(atPath: images.appendingPathComponent(name).path) {
            name = ext.isEmpty ? "\(base)-\(n)" : "\(base)-\(n).\(ext)"
            n += 1
        }
        try data.write(to: images.appendingPathComponent(name), options: .atomic)
        return name
    }

    func imageURL(asset: String, skinId: String) -> URL {
        root.appendingPathComponent(skinId).appendingPathComponent("images").appendingPathComponent(asset)
    }

    // MARK: rename, delete, ids

    /// Rewrites `name` (capped, `skinName`) in `skin.json` as a new version (so it can be undone).
    func rename(skinId: String, to name: String) throws {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let dir = try folder(skinId)
        guard let data = fm.contents(atPath: dir.appendingPathComponent("skin.json").path),
              var root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw StoreError.unreadableManifest(skinId)
        }
        root["name"] = Self.skinName(name)
        let manifest = try JSONSerialization.data(withJSONObject: root, options: [.prettyPrinted, .sortedKeys])
        _ = try save(manifest: manifest, palettes: currentPalettes(in: dir), for: skinId)
    }

    /// Moves the skin to `.deleted/<id>/` (an older copy there is replaced) and stamps when, as core does,
    /// so an edit that lost to the deletion can still be found; `purgeDeleted` drops it later.
    func delete(skinId: String) throws {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let from = try folder(skinId)
        let deleted = root.appendingPathComponent(Self.deletedFolder, isDirectory: true)
        try fm.createDirectory(at: deleted, withIntermediateDirectories: true)
        let to = deleted.appendingPathComponent(skinId, isDirectory: true)
        if fm.fileExists(atPath: to.path) { try fm.removeItem(at: to) }
        try fm.moveItem(at: from, to: to)
        // The skin is gone from the folder either way, so it is always sent; without a stamp, the purge goes by the folder's date.
        try? Data("\(Int64(Date().timeIntervalSince1970 * 1000))".utf8).write(to: to.appendingPathComponent(Self.deletedMarker), options: .atomic)
        changed?(skinId)
    }

    /// Removes a skin's folder for good, unsent (a failed import's leftovers); not for a user's delete.
    func removeFolder(skinId: String) throws {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let dir = try folder(skinId)
        if fm.fileExists(atPath: dir.path) { try fm.removeItem(at: dir) }
    }

    /// Removes what has sat in `.deleted/` for longer than `age` (by its stamp, else the folder's date);
    /// returns how many were removed. Core's `purge_deleted`.
    @discardableResult
    func purgeDeleted(olderThan age: TimeInterval) -> Int {
        Self.writing.lock()
        defer { Self.writing.unlock() }
        let deleted = root.appendingPathComponent(Self.deletedFolder, isDirectory: true)
        var purged = 0
        for id in ((try? fm.contentsOfDirectory(atPath: deleted.path)) ?? []) where (try? folder(id)) != nil {
            let dir = deleted.appendingPathComponent(id, isDirectory: true)
            let stamp = fm.contents(atPath: dir.appendingPathComponent(Self.deletedMarker).path)
                .flatMap { Double(String(decoding: $0, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)) }
                .map { Date(timeIntervalSince1970: $0 / 1000) }
            let when = stamp ?? (try? fm.attributesOfItem(atPath: dir.path)[.modificationDate] as? Date) ?? Date()
            if Date().timeIntervalSince(when) > age, (try? fm.removeItem(at: dir)) != nil { purged += 1 }
        }
        return purged
    }

    /// `user-<slug>-<n>` with the lowest `n` whose folder does not exist; the slug is reduced to
    /// `a-z0-9` words joined by `-`, at most `nameLimit` characters.
    func newSkinId(slug: String) -> String {
        let words = slug.lowercased().split { !(($0 >= "a" && $0 <= "z") || ($0 >= "0" && $0 <= "9")) }
        let joined = String(words.joined(separator: "-").prefix(Self.nameLimit))
        let clean = joined.hasSuffix("-") ? String(joined.dropLast()) : joined.isEmpty ? "skin" : joined
        var n = 1
        while fm.fileExists(atPath: root.appendingPathComponent("user-\(clean)-\(n)").path) { n += 1 }
        return "user-\(clean)-\(n)"
    }

    // MARK: loading

    /// Every user skin (validated against `base`, id taken from its folder) by name, and their palettes.
    func loadSkinsAndPalettes(base: Skin) -> (skins: [Skin], palettes: [Theme]) {
        var skins: [Skin] = [], palettes: [Theme] = []
        for id in skinIds() {
            let dir = root.appendingPathComponent(id, isDirectory: true)
            guard let data = fm.contents(atPath: dir.appendingPathComponent("skin.json").path),
                  var (skin, _) = SkinLoader.parse(data, base: base) else {
                Self.log.error("user skin skipped: \(id, privacy: .public)")
                continue
            }
            skin.id = id  // images and history are found by folder, so the folder name wins
            skins.append(skin)
            // A user palette id is `<skinId>-p<n>`, so it can never shadow a bundled or another skin's palette.
            for theme in currentPalettes(in: dir).compactMap({ ThemeLoader.parse($0.data) }) {
                if theme.id.hasPrefix("\(id)-") { palettes.append(theme) } else {
                    Self.log.error("user palette skipped, id not \(id, privacy: .public)-…: \(theme.id, privacy: .public)")
                }
            }
        }
        return (skins.sorted { $0.name < $1.name }, palettes)
    }
}
