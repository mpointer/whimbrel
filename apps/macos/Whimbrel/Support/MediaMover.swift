import CryptoKit
import Foundation
import os

/// Storage retention's file move (Task 6): copy, verify size and SHA-256, flush to the
/// drive, rewrite the stored paths, and only then delete the originals. Any failure before
/// the path rewrite removes what this run created and leaves the originals and their stored
/// paths alone.
///
/// File work runs on `queue`; the store calls hop to `CoreMemoryStore.accessQueue.sync`,
/// so never call `moveAudio`/`moveSnapshots` from `accessQueue` itself.
enum MediaMover {
    static let queue = DispatchQueue(label: "com.mpointer.kestrel.media-mover", qos: .utility)
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "MediaMover")

    /// Test seam: the raw copy that `copyVerified` checks afterwards.
    nonisolated(unsafe) static var copyItem: (URL, URL) throws -> Void = { try FileManager.default.copyItem(at: $0, to: $1) }
    /// Test seam: runs just before the stored paths are rewritten.
    nonisolated(unsafe) static var beforePathUpdate: () -> Void = {}

    struct VerifyFailed: Error { let file: URL }
    struct DestinationTaken: Error { let file: URL }

    /// Copies `src` to `dst` through a hidden `.<name>.partial` that is verified and flushed
    /// before it is renamed into place. Returns true when this call created `dst`, false when
    /// an identical file was already there. A different file at `dst` is never replaced:
    /// that throws `DestinationTaken`. Nothing partial is left behind on failure.
    @discardableResult
    static func copyVerified(_ src: URL, to dst: URL) throws -> Bool {
        let fm = FileManager.default
        let srcSize = try size(src)
        if fm.fileExists(atPath: dst.path) {
            if (try? size(dst)) == srcSize, try sha256(dst) == sha256(src) { return false }
            throw DestinationTaken(file: dst)
        }
        let partial = dst.deletingLastPathComponent().appendingPathComponent("." + dst.lastPathComponent + ".partial")
        try? fm.removeItem(at: partial)  // ours by name: left by a run that stopped
        do {
            try copyItem(src, partial)
            guard try size(partial) == srcSize, try sha256(partial) == sha256(src) else { throw VerifyFailed(file: partial) }
            try flush(partial)
            try fm.moveItem(at: partial, to: dst)  // rename(2); fails if `dst` appeared meanwhile
        } catch {
            try? fm.removeItem(at: partial)
            throw error
        }
        return true
    }

    /// Moves every existing track of each stored path's pair into `audioDir` (this device's
    /// `<media>/Audio/<deviceId>`) as one unit, rewrites the stored paths, then deletes the
    /// originals. False, with the originals and stored paths untouched and nothing this run
    /// created left behind, on any failure before the rewrite.
    static func moveAudio(meetingId: Int64, audioPaths: [String], to audioDir: URL, store: CoreMemoryStore) -> Bool {
        dispatchPrecondition(condition: .notOnQueue(CoreMemoryStore.accessQueue))
        let fm = FileManager.default
        // Only make folders inside a media folder that exists: an unplugged drive's mount
        // point must not get folders made on the internal disk.
        let audioRoot = audioDir.deletingLastPathComponent()
        guard fm.fileExists(atPath: audioRoot.deletingLastPathComponent().path) else { return false }

        // Deduped (the meeting row repeats its last part's path) and never a file already
        // in `audioDir`: its "original" would be the copy itself.
        var stored: [String] = []
        for p in audioPaths where !stored.contains(p) && !isInside(URL(fileURLWithPath: p), audioDir) {
            stored.append(p)
        }
        // A gone folder on an unplugged drive: skip the whole meeting. A folder deleted on a
        // mounted volume: its files are already gone, so leave that path as it is.
        for p in stored {
            let folder = URL(fileURLWithPath: p).deletingLastPathComponent()
            if !fm.fileExists(atPath: folder.path), volumeGone(folder) { return false }
        }
        stored.removeAll { !fm.fileExists(atPath: URL(fileURLWithPath: $0).deletingLastPathComponent().path) }
        guard !stored.isEmpty else { return true }

        var files: [(src: URL, dst: URL)] = []
        for p in stored {
            let tracks = TranscriptionService.trackURLs(forAudioPath: p).map { [$0.system, $0.mic] } ?? [URL(fileURLWithPath: p)]
            for t in tracks where !files.contains(where: { $0.src == t }) {
                // A gone folder is an unreachable drive: skip the whole meeting.
                guard fm.fileExists(atPath: t.deletingLastPathComponent().path) else { return false }
                if fm.fileExists(atPath: t.path) {
                    files.append((t, audioDir.appendingPathComponent(t.lastPathComponent)))
                }
            }
        }
        // Two sources with one name can't share the folder (nor two stored paths one new path).
        guard Set(files.map(\.dst)).count == files.count,
              Set(stored.map { URL(fileURLWithPath: $0).lastPathComponent }).count == stored.count else { return false }

        var copied: [URL] = []
        let undo = { for c in copied { try? fm.removeItem(at: c) } }
        do {
            try fm.createDirectory(at: audioDir, withIntermediateDirectories: true)
            for f in files {
                if try copyVerified(f.src, to: f.dst) { copied.append(f.dst) }
            }
            try flush(audioDir)
            try flush(audioRoot)
        } catch {
            log.error("meeting \(meetingId): audio copy failed: \(String(describing: error))")
            undo()
            return false
        }

        beforePathUpdate()
        let moves = stored.map { (old: $0, new: audioDir.appendingPathComponent(URL(fileURLWithPath: $0).lastPathComponent).path) }
        let (rewritten, durable) = CoreMemoryStore.accessQueue.sync {
            let ok = store.setMeetingAudioPaths(meetingId: meetingId, moves: moves)
            return (ok, ok && store.checkpointDurable())
        }
        guard rewritten else {
            log.error("meeting \(meetingId): audio path update failed; originals kept")
            undo()
            return false
        }
        // The rows may point at the copies already: keep both until the commit is durable.
        guard durable else {
            log.error("meeting \(meetingId): audio path update not confirmed durable; keeping originals and copies")
            return false
        }
        // The rows point at the copies now; an original that won't delete is just an orphan.
        for f in files {
            do { try fm.removeItem(at: f.src) } catch {
                log.error("meeting \(meetingId): could not delete \(f.src.path): \(String(describing: error))")
            }
        }
        return true
    }

    /// Moves the meeting's snapshot files into a new marked folder, named like the old one,
    /// under `snapshotsRoot`; repoints the rows; then deletes the moved originals one by one.
    /// The old folder goes only once it holds nothing but the marker and `.DS_Store`: after
    /// a split another meeting may still have files in it.
    static func moveSnapshots(meetingId: Int64, to snapshotsRoot: URL, store: CoreMemoryStore) -> Bool {
        dispatchPrecondition(condition: .notOnQueue(CoreMemoryStore.accessQueue))
        let fm = FileManager.default
        guard fm.fileExists(atPath: snapshotsRoot.deletingLastPathComponent().path) else { return false }
        let (snaps, dir) = CoreMemoryStore.accessQueue.sync {
            (store.listMeetingSnapshots(meetingId: meetingId), store.meetingSnapshotsInfo(meetingId: meetingId)?.dir)
        }
        // The stored string is what the rewrite must match, so keep it as is.
        var srcs = snaps.map { (stored: $0.filePath, url: URL(fileURLWithPath: $0.filePath)) }
            .filter { !isInside($0.url, snapshotsRoot) }
        // The listing already dropped rows whose file is gone from an existing folder, so a
        // missing file here means a gone folder. On an unplugged drive: skip the meeting.
        // Deleted on a mounted volume: leave that row as it is (already gone).
        if srcs.contains(where: { !fm.fileExists(atPath: $0.url.path) && volumeGone($0.url) }) { return false }
        srcs.removeAll { !fm.fileExists(atPath: $0.url.path) }
        guard let first = srcs.first else { return true }
        // Two files with one name can't share the new folder.
        guard Set(srcs.map(\.url.lastPathComponent)).count == srcs.count else { return false }

        let name = (dir.map { URL(fileURLWithPath: $0) } ?? first.url.deletingLastPathComponent()).lastPathComponent
        var folder: URL?
        var moves: [(old: String, new: String)] = []
        var durable = false
        do {
            let made = try SnapshotStorage.create(name, under: snapshotsRoot)
            folder = made
            for src in srcs {
                let dst = made.appendingPathComponent(src.url.lastPathComponent)
                try copyVerified(src.url, to: dst)
                moves.append((src.stored, dst.path))
            }
            try flush(made)
            try flush(snapshotsRoot)
            beforePathUpdate()
            let (rewritten, ok) = CoreMemoryStore.accessQueue.sync {
                let ok = store.setMeetingSnapshotPaths(meetingId: meetingId, moves: moves, snapshotsDir: made.path)
                return (ok, ok && store.checkpointDurable())
            }
            guard rewritten else { throw CocoaError(.fileWriteUnknown) }
            durable = ok
        } catch {
            log.error("meeting \(meetingId): snapshot move failed: \(String(describing: error))")
            // Just made by this run: only our marker and copies can be in it.
            if let folder {
                removeFolder(folder, ifOnly: Set([SnapshotStorage.markerName] + moves.map { URL(fileURLWithPath: $0.new).lastPathComponent }))
            }
            return false
        }
        // The rows may point at the copies already: keep both until the commit is durable.
        guard durable else {
            log.error("meeting \(meetingId): snapshot path update not confirmed durable; keeping originals and copies")
            return false
        }

        for src in srcs { try? fm.removeItem(at: src.url) }
        for old in Set(srcs.map { $0.url.deletingLastPathComponent() }) {
            removeFolder(old, ifOnly: [SnapshotStorage.markerName, ".DS_Store"])
        }
        return true
    }

    // MARK: Helpers

    /// `url` is under `/Volumes/<name>` and that volume isn't mounted: an unplugged drive,
    /// not a folder the user deleted. Mirrors core's `media_retention::volume_gone`.
    static func volumeGone(_ url: URL) -> Bool {
        let c = url.standardizedFileURL.pathComponents
        return c.count > 2 && c[0] == "/" && c[1] == "Volumes"
            && !FileManager.default.fileExists(atPath: "/Volumes/" + c[2])
    }

    /// Removes `dir` only when it holds nothing but `allowed` names: unlinks each (never a
    /// folder), then rmdir(2), which refuses a folder that isn't empty. Never recursive.
    private static func removeFolder(_ dir: URL, ifOnly allowed: Set<String>) {
        guard let left = try? FileManager.default.contentsOfDirectory(atPath: dir.path),
              Set(left).isSubset(of: allowed) else { return }
        for n in left { unlink(dir.appendingPathComponent(n).path) }
        rmdir(dir.path)
    }

    /// Pushes a file's or folder's contents to the drive itself (F_FULLFSYNC), so a pulled
    /// drive or power cut can't leave a truncated copy once the original is gone. Falls back
    /// to fsync(2) on file systems without F_FULLFSYNC.
    static func flush(_ url: URL) throws {
        let fd = open(url.path, O_RDONLY)
        guard fd >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        defer { close(fd) }
        if fcntl(fd, F_FULLFSYNC) == -1, fsync(fd) == -1 {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
    }

    private static func size(_ url: URL) throws -> Int64 {
        (try FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.int64Value ?? -1
    }

    /// Streamed in 1 MB reads.
    private static func sha256(_ url: URL) throws -> SHA256.Digest {
        let h = try FileHandle(forReadingFrom: url)
        defer { try? h.close() }
        var hasher = SHA256()
        while try autoreleasepool(invoking: { () throws -> Bool in
            guard let chunk = try h.read(upToCount: 1 << 20), !chunk.isEmpty else { return false }
            hasher.update(data: chunk)
            return true
        }) {}
        return hasher.finalize()
    }

    /// Component-wise on resolved paths, so `Audio2/` is not inside `Audio/` and
    /// `/var` vs `/private/var` never matters.
    private static func isInside(_ file: URL, _ folder: URL) -> Bool {
        let f = file.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        let d = folder.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        return f.count > d.count && Array(f.prefix(d.count)) == d
    }
}
