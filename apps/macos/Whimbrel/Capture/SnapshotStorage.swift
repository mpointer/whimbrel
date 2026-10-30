import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Per-meeting snapshot folders. Rust's retention sweep deletes a folder only if it holds
/// `SNAPSHOT_MARKER` and otherwise only `.DS_Store` and images named like `fileName` makes them
/// (`retention::is_snapshot_file`), so write nothing else here and keep the two in step.
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
        // Use the root only when its parent (the media folder) already exists: an unmounted
        // drive's mount point outside /Volumes must not get folders made on the internal disk.
        let parentExists = FileManager.default.fileExists(atPath: root.deletingLastPathComponent().path)
        do {
            guard parentExists else { throw CocoaError(.fileNoSuchFile) }
            return try Self.create(name, under: root)
        } catch {
            return try Self.create(name, under: fallbackRoot)
        }
    }

    static func create(_ name: String, under parent: URL) throws -> URL {
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

    /// The encoder `writeHEIC` asks for; a test seam for a Mac without one.
    nonisolated(unsafe) static var heicType = UTType.heic.identifier

    /// `HH.mm.ss.<ext>`, suffixed `-2`, `-3`… while any file of that stem is taken: a `.png`,
    /// `.heic` or `.thumb.jpg` of one stem would share a thumbnail name.
    static func fileName(captured: Date, existing: Set<String>, ext: String = "heic") -> String {
        let base = formatted(captured, "HH.mm.ss", .current)
        var stem = base
        var n = 2
        while ["png", "heic", "thumb.jpg"].contains(where: { existing.contains("\(stem).\($0)") }) {
            stem = "\(base)-\(n)"
            n += 1
        }
        return "\(stem).\(ext)"
    }

    /// Writes the frame as HEIC at full resolution; where HEIC can't be encoded, as the PNG it
    /// always was, so a frame is never dropped for want of an encoder.
    func writeHEIC(_ image: CGImage, to folder: URL, captured: Date) throws -> URL {
        let existing = Set((try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? [])
        for (ext, type) in [("heic", Self.heicType), ("png", UTType.png.identifier)] {
            let url = folder.appendingPathComponent(Self.fileName(captured: captured, existing: existing, ext: ext))
            if let dest = CGImageDestinationCreateWithURL(url as CFURL, type as CFString, 1, nil) {
                CGImageDestinationAddImage(dest, image, nil)
                if CGImageDestinationFinalize(dest) { return url }
            }
            try? FileManager.default.removeItem(at: url)
        }
        throw CocoaError(.fileWriteUnknown)
    }
}
