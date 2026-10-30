import Foundation
import ImageIO
import os
import UniformTypeIdentifiers

/// Storage retention's image shrink (Task 8): an old snapshot becomes `<name>.thumb.jpg`
/// beside it, 320 px wide. The thumbnail goes through a hidden temp that must decode and is
/// flushed, then is renamed into place without overwriting and the folder flushed; only then
/// is the row rewritten (OCR text unchanged) and checkpointed, and only once both succeed is
/// the full image deleted. Any failure keeps the full image.
///
/// Only Whimbrel's own snapshots are touched: names `fileName` makes, in a folder holding
/// `SnapshotStorage.markerName`.
///
/// The store calls hop to `CoreMemoryStore.accessQueue.sync`, so never call `shrinkSnapshot`
/// from `accessQueue` itself.
enum SnapshotThumbnailer {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "SnapshotThumbnailer")

    static let width = 320
    static let quality = 0.7

    /// Test seam: the encoder `shrink` writes the temp with.
    nonisolated(unsafe) static var writeJPEG: (CGImage, URL) throws -> Void = encodeJPEG
    /// Test seam: runs just before the row is rewritten.
    nonisolated(unsafe) static var beforePathUpdate: () -> Void = {}

    struct NotASnapshot: Error { let file: URL }
    struct DestinationTaken: Error { let file: URL }
    struct VerifyFailed: Error { let file: URL }

    /// A full-size snapshot as `SnapshotStorage.fileName` names it (core `is_snapshot_file`,
    /// minus the thumbnail).
    nonisolated(unsafe) private static let fullName = /^[0-9]{2}\.[0-9]{2}\.[0-9]{2}(-[0-9]+)?\.(png|heic)$/

    static func encodeJPEG(_ image: CGImage, to url: URL) throws {
        guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else {
            throw CocoaError(.fileWriteUnknown)
        }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { throw CocoaError(.fileWriteUnknown) }
    }

    /// Writes `<name minus extension>.thumb.jpg` beside `full`: `width` px wide, or the image's
    /// own width if narrower. Never replaces an existing file and never deletes `full`. On a
    /// throw, nothing new is left behind.
    static func shrink(_ full: URL) throws -> (url: URL, width: Int, height: Int) {
        let fm = FileManager.default
        let folder = full.deletingLastPathComponent()
        let name = full.lastPathComponent
        guard name.wholeMatch(of: fullName) != nil,
              fm.fileExists(atPath: folder.appendingPathComponent(SnapshotStorage.markerName).path) else {
            throw NotASnapshot(file: full)
        }
        let dst = folder.appendingPathComponent(full.deletingPathExtension().lastPathComponent + ".thumb.jpg")
        guard !fm.fileExists(atPath: dst.path) else { throw DestinationTaken(file: dst) }

        guard let src = CGImageSourceCreateWithURL(full as CFURL, nil),
              let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any],
              let w = props[kCGImagePropertyPixelWidth] as? Int, let h = props[kCGImagePropertyPixelHeight] as? Int,
              w > 0, h > 0 else { throw VerifyFailed(file: full) }
        // The size limit is on the long side: scale it so the width comes out at `width`.
        let targetW = min(w, width)
        let maxSide = max(targetW, Int((Double(h) * Double(targetW) / Double(w)).rounded()))
        guard let thumb = CGImageSourceCreateThumbnailAtIndex(src, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxSide,
        ] as CFDictionary) else { throw VerifyFailed(file: full) }

        let temp = folder.appendingPathComponent("." + dst.lastPathComponent)
        try? fm.removeItem(at: temp)  // ours by name: left by a run that stopped
        var renamed = false
        do {
            try writeJPEG(thumb, temp)
            guard let back = CGImageSourceCreateWithURL(temp as CFURL, nil).flatMap({ CGImageSourceCreateImageAtIndex($0, 0, nil) }),
                  back.width == thumb.width, back.height == thumb.height else { throw VerifyFailed(file: temp) }
            try MediaMover.flush(temp)
            try fm.moveItem(at: temp, to: dst)  // fails if `dst` appeared meanwhile
            renamed = true
            try MediaMover.flush(folder)
        } catch {
            try? fm.removeItem(at: renamed ? dst : temp)
            throw error
        }
        return (dst, thumb.width, thumb.height)
    }

    /// Shrinks one snapshot: `shrink`, then the row rewritten to the thumbnail (OCR text
    /// unchanged) and checkpointed, then the full image deleted. False on any failure, with
    /// the full image kept; the thumbnail is removed too unless the row may point at it.
    static func shrinkSnapshot(_ snap: CoreMeetingSnapshot, store: CoreMemoryStore) -> Bool {
        dispatchPrecondition(condition: .notOnQueue(CoreMemoryStore.accessQueue))
        let full = URL(fileURLWithPath: snap.filePath)
        let thumb: (url: URL, width: Int, height: Int)
        do { thumb = try shrink(full) } catch {
            log.error("snapshot \(snap.id): thumbnail failed: \(String(describing: error))")
            return false
        }
        beforePathUpdate()
        let (rewritten, durable) = CoreMemoryStore.accessQueue.sync {
            let ok = store.replaceMeetingSnapshot(id: snap.id, filePath: thumb.url.path, width: thumb.width,
                                                  height: thumb.height, ocrText: snap.ocrText)
            return (ok, ok && store.checkpointDurable())
        }
        guard rewritten else {
            log.error("snapshot \(snap.id): row update failed; full image kept")
            try? FileManager.default.removeItem(at: thumb.url)
            return false
        }
        // The row may point at the thumbnail already: keep both until the commit is durable.
        guard durable else {
            log.error("snapshot \(snap.id): row update not confirmed durable; keeping the full image and thumbnail")
            return false
        }
        do { try FileManager.default.removeItem(at: full) } catch {
            log.error("snapshot \(snap.id): could not delete \(full.path): \(String(describing: error))")
        }
        return true
    }
}
