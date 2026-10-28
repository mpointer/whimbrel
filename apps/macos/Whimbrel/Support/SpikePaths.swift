import Foundation
import os

/// Local, unencrypted output for the M1.1 capture spike only.
/// Production capture writes through core/memory (docs/04-architecture.md), not raw files.
enum SpikePaths {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "data-folder")

    /// The data folder's name before Whimbrel; still searched for, so it moves over once.
    static let previousFolderName = "Kestrel"

    /// Under the test host this is a per-process temp folder, never the real Application
    /// Support: resolving there would move the developer's real data folder (and no launch
    /// re-root follows, because the test host never builds AppState).
    static let appSupportDirectory: URL = KeychainAccess.isTestHost
        ? FileManager.default.temporaryDirectory.appendingPathComponent("whimbrel-test-host-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
        : FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]

    private static let resolved: (root: URL, movedFrom: URL?) = {
        try? FileManager.default.createDirectory(at: appSupportDirectory, withIntermediateDirectories: true)
        return resolveRoot(appSupport: appSupportDirectory)
    }()

    static var root: URL { resolved.root }

    /// The legacy folder this launch moved the data out of, so stored absolute paths can
    /// be rewritten; nil when nothing moved.
    static var movedFrom: URL? { resolved.movedFrom }

    /// The data folder is `Application Support/Whimbrel/`. It used to be
    /// `Application Support/Kestrel/`, and before that `Application Support/littlebird/spike/`,
    /// which on case-insensitive APFS is inside the real Littlebird app's own folder, so
    /// uninstalling Littlebird deleted it (#180).
    /// Moves the first legacy folder that exists (Kestrel, then littlebird) over with one
    /// same-volume rename. If a `Whimbrel/` without a database is in the way, or the move
    /// fails, that legacy folder stays in use.
    static func resolveRoot(appSupport: URL, fileManager fm: FileManager = .default) -> (root: URL, movedFrom: URL?) {
        let newRoot = appSupport.appendingPathComponent("Whimbrel", isDirectory: true)
        let legacyRoots = [
            appSupport.appendingPathComponent(previousFolderName, isDirectory: true),
            appSupport.appendingPathComponent("littlebird/spike", isDirectory: true),
        ]
        let hasDatabase = { (dir: URL) in fm.fileExists(atPath: dir.appendingPathComponent("core-memory.sqlite").path) }

        if hasDatabase(newRoot) {
            for legacy in legacyRoots where hasDatabase(legacy) {
                log.error("Both \(newRoot.path, privacy: .public) and \(legacy.path, privacy: .public) hold a database; using the first and leaving the second alone.")
            }
            return (newRoot, nil)
        }
        if let legacyRoot = legacyRoots.first(where: { fm.fileExists(atPath: $0.path) }) {
            if fm.fileExists(atPath: newRoot.path) {
                log.error("Not moving \(legacyRoot.path, privacy: .public): \(newRoot.path, privacy: .public) already exists without a database.")
                return (legacyRoot, nil)
            }
            do {
                try fm.moveItem(at: legacyRoot, to: newRoot)
            } catch {
                // Another launch moved it first: use its result, don't fall back to a folder that's gone.
                if hasDatabase(newRoot) { return (newRoot, nil) }
                log.error("Could not move \(legacyRoot.path, privacy: .public) to \(newRoot.path, privacy: .public): \(error.localizedDescription, privacy: .public)")
                return (legacyRoot, nil)
            }
            let legacyParent = legacyRoot.deletingLastPathComponent()
            if legacyParent != appSupport, (try? fm.contentsOfDirectory(atPath: legacyParent.path))?.isEmpty == true {
                try? fm.removeItem(at: legacyParent)
            }
            log.notice("Moved the data folder to \(newRoot.path, privacy: .public).")
            return (newRoot, legacyRoot)
        }
        try? fm.createDirectory(at: newRoot, withIntermediateDirectories: true)
        return (newRoot, nil)
    }

    /// Folders (with trailing slash) whose stored paths should be rewritten onto `root`.
    /// Every launch on Whimbrel gets both legacy folders, not just the launch that moved,
    /// so a re-root that failed or was interrupted is retried; it is idempotent.
    static func legacyPrefixes(appSupport: URL, root: URL) -> [String] {
        guard root.lastPathComponent == "Whimbrel" else { return [] }
        return [previousFolderName, "littlebird/spike"].map { appSupport.appendingPathComponent($0, isDirectory: true).path + "/" }
    }

    static var rerootPrefixes: [String] { legacyPrefixes(appSupport: appSupportDirectory, root: root) }

    /// `path` re-rooted from the first matching prefix onto `root`, or nil if none matches.
    static func reroot(_ path: String, prefixes: [String], root: URL) -> String? {
        guard let old = prefixes.first(where: { path.hasPrefix($0) }) else { return nil }
        return root.path + "/" + path.dropFirst(old.count)
    }

    static func rerooted(_ path: String) -> String? {
        reroot(path, prefixes: rerootPrefixes, root: root)
    }

    static func audioFile(name: String) -> URL {
        root.appendingPathComponent(name)
    }

    static let screenTextLog = root.appendingPathComponent("screen-text.jsonl")

    static let database = root.appendingPathComponent("core-memory.sqlite")

    static func sessionTimestamp() -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: Date()).replacingOccurrences(of: ":", with: "-")
    }

    /// Reverses `sessionTimestamp()`'s `":"` → `"-"` filename-safe substitution and
    /// parses the result back into the `Date` it was generated from. Only the time
    /// portion (after `"T"`) ever had colons, so only that portion's dashes are
    /// converted back — the date portion's own dashes (`yyyy-MM-dd`) are left alone.
    static func sessionStartDate(from stamp: String) -> Date? {
        guard let tIndex = stamp.firstIndex(of: "T") else { return nil }
        let datePart = stamp[stamp.startIndex..<tIndex]
        let timePart = stamp[stamp.index(after: tIndex)...].replacingOccurrences(of: "-", with: ":")
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: "\(datePart)T\(timePart)")
    }
}
