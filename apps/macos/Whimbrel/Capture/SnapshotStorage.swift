import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Per-meeting snapshot folders. Rust's retention sweep deletes a folder only if it holds
/// `SNAPSHOT_MARKER` and otherwise only `.DS_Store` and PNGs named like `fileName` makes them
/// (`retention::is_snapshot_png`), so write nothing else here and keep the two in step.
struct SnapshotStorage {
    static let markerName = ".kestrel-snapshots"

    let root: URL
    let fallbackRoot: URL

    private static func formatted(_ date: Date, _ format: String, _ timeZone: TimeZone) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = timeZone
        f.dateFormat = format
        return f.string(from: date)
    }

    static func folderName(start: Date, title: String, timeZone: TimeZone = .current) -> String {
        var t = title
        for c in ["/", ":", "\\"] { t = t.replacingOccurrences(of: c, with: "-") }
        // Cc only: CharacterSet.controlCharacters also strips ZWJ (Cf), which breaks emoji sequences.
        t.unicodeScalars.removeAll { $0.properties.generalCategory == .control }
        let trim = { (s: String) in
            String(s.trimmingCharacters(in: .whitespacesAndNewlines).drop(while: { $0 == "." }))
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        t = trim(String(trim(t).prefix(80)))
        // APFS rejects names over 255 bytes; leave room for the date prefix and " N" suffix.
        while t.utf8.count > 200 { t.removeLast() }
        t = trim(t)
        if t.isEmpty { t = "Meeting" }
        return "\(formatted(start, "yyyy-MM-dd HH.mm", timeZone)) \(t)"
    }

    func makeMeetingFolder(start: Date, title: String) throws -> URL {
        let name = Self.folderName(start: start, title: title)
        do {
            return try Self.create(name, under: root)
        } catch {
            return try Self.create(name, under: fallbackRoot)
        }
    }

    private static func create(_ name: String, under parent: URL) throws -> URL {
        let fm = FileManager.default
        try fm.createDirectory(at: parent, withIntermediateDirectories: true)
        var n = 1
        while true {
            let folder = parent.appendingPathComponent(n == 1 ? name : "\(name) \(n)", isDirectory: true)
            if !fm.fileExists(atPath: folder.path) {
                try fm.createDirectory(at: folder, withIntermediateDirectories: false)
                try Data().write(to: folder.appendingPathComponent(markerName))
                return folder
            }
            n += 1
        }
    }

    static func fileName(captured: Date, existing: Set<String>) -> String {
        let base = formatted(captured, "HH.mm.ss", .current)
        var name = "\(base).png"
        var n = 2
        while existing.contains(name) {
            name = "\(base)-\(n).png"
            n += 1
        }
        return name
    }

    func writePNG(_ image: CGImage, to folder: URL, captured: Date) throws -> URL {
        let existing = Set((try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? [])
        let url = folder.appendingPathComponent(Self.fileName(captured: captured, existing: existing))
        guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
            throw CocoaError(.fileWriteUnknown)
        }
        CGImageDestinationAddImage(dest, image, nil)
        guard CGImageDestinationFinalize(dest) else { throw CocoaError(.fileWriteUnknown) }
        return url
    }
}
