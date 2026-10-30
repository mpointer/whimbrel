import Foundation
import os

enum MeetingLifecycle {
    /// Posted on main when meetings' parts, segments or snapshots changed under open views.
    /// userInfo["meetingIds"] is a [Int64].
    static let meetingsDidChange = Notification.Name("com.mpointer.kestrel.meetings.didChange")
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-split")

    static func postMeetingsChanged(_ ids: [Int64]) {
        NotificationCenter.default.post(name: meetingsDidChange, object: nil, userInfo: ["meetingIds": ids])
    }

    struct SplitOutcome: Equatable {
        let newMeetingId: Int64
        /// True when the snapshot files could not be moved; they stay in the original folder.
        let snapshotMoveFailed: Bool
    }

    enum ForgetResult: Equatable {
        case deleted
        case deletedWithFileCleanupWarning
        case failed
    }

    /// Synchronous core used by both the sync test above and the async
    /// production entry point below: delete the durable row FIRST, and only
    /// clean up local files once that succeeds. A DB-delete failure must never
    /// touch files -- the record still exists and its files must stay valid.
    static func forgetMeetingSync(
        deleteRow: () -> Bool,
        deleteFiles: () -> Void
    ) -> ForgetResult {
        guard deleteRow() else { return .failed }
        deleteFiles()
        return .deleted
    }

    /// Production entry point. Runs on `CoreMemoryStore.accessQueue` like the
    /// call sites it replaces, then calls back on the main queue. Queued behind
    /// `MediaMover.queue` first, so it waits for the media retention work in flight.
    static func forgetMeeting(
        store: CoreMemoryStore,
        meetingId: Int64,
        recordingsDir: URL = SpikePaths.root,
        completion: @escaping (ForgetResult) -> Void
    ) {
        MediaMover.queue.async { CoreMemoryStore.accessQueue.sync {
            // The row delete returns every part's files and the snapshot folder (nil when the
            // meeting kept its snapshots or another meeting still has files in it).
            var files: CoreDeletedMeetingFiles?
            let result = forgetMeetingSync(
                deleteRow: {
                    files = store.deleteMeetingReturningFiles(id: meetingId)
                    return files != nil
                },
                deleteFiles: {
                    if let files {
                        CoreMemoryStore.deleteAllMeetingFiles(files, recordingsDir: recordingsDir)
                    }
                }
            )
            DispatchQueue.main.async {
                completion(result)
            }
        } }
    }

    /// Splits the last part of a completed meeting into its own meeting and moves the
    /// snapshot files it now owns. Returns the new meeting's id (nil: not splittable).
    /// File order is crash-safe: copy into the new folder, repoint the rows, only then
    /// delete the originals. A failed copy or repoint leaves the originals in place
    /// (the rows still point at them) and removes the partial copies.
    static func splitLastPartSync(
        store: CoreMemoryStore,
        meetingId: Int64,
        makeFolder: (_ start: Date, _ title: String) throws -> URL
    ) -> SplitOutcome? {
        guard let parts = store.meetingParts(meetingId: meetingId), parts.count >= 2,
              let result = store.splitMeeting(meetingId: meetingId, fromPartIndex: parts[parts.count - 1].partIndex)
        else { return nil }
        guard !result.movedSnapshotPaths.isEmpty, let moved = store.getMeeting(id: result.newMeetingId) else {
            return SplitOutcome(newMeetingId: result.newMeetingId, snapshotMoveFailed: false)
        }
        var failed = false
        let fm = FileManager.default
        var copies: [(old: String, new: String)] = []
        do {
            let folder = try makeFolder(Date(timeIntervalSince1970: Double(moved.startUnixMs) / 1000), moved.title)
            for old in result.movedSnapshotPaths where fm.fileExists(atPath: old) {
                let dest = folder.appendingPathComponent((old as NSString).lastPathComponent)
                try fm.copyItem(atPath: old, toPath: dest.path)
                copies.append((old, dest.path))
            }
            guard store.setMeetingSnapshotPaths(meetingId: result.newMeetingId, moves: copies, snapshotsDir: folder.path) else {
                throw CocoaError(.fileWriteUnknown)
            }
            for c in copies { try? fm.removeItem(atPath: c.old) }
        } catch {
            failed = true
            log.error("Split of meeting \(meetingId): snapshot move failed: \(String(describing: error))")
            for c in copies { try? fm.removeItem(atPath: c.new) }
        }
        return SplitOutcome(newMeetingId: result.newMeetingId, snapshotMoveFailed: failed)
    }

    /// Off-main wrapper; on main it posts `meetingsDidChange` for both meetings, then calls back.
    /// Waits for the media retention work in flight (`MediaMover.queue`): that meeting's or a
    /// meeting sharing its snapshot folder may be mid-move.
    static func splitLastPart(
        store: CoreMemoryStore,
        meetingId: Int64,
        makeFolder: @escaping (Date, String) throws -> URL,
        completion: @escaping (SplitOutcome?) -> Void
    ) {
        MediaMover.queue.async {
            let outcome = CoreMemoryStore.accessQueue.sync { splitLastPartSync(store: store, meetingId: meetingId, makeFolder: makeFolder) }
            DispatchQueue.main.async {
                if let outcome { postMeetingsChanged([meetingId, outcome.newMeetingId]) }
                completion(outcome)
            }
        }
    }
}
