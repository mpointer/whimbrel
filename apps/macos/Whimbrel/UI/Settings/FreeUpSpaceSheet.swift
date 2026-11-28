import SwiftUI

/// One cleanup Free up space offers, with what it would free now.
struct FreeUpSpaceOption: Identifiable, Equatable {
    enum Kind: Hashable { case compactDatabase, compressAudio, deleteOldAudio(days: Int), deleteOldCaptures(days: Int) }
    var id: Kind { kind }
    let kind: Kind
    /// Bytes it frees, when known before running.
    let frees: Int64?
    let detail: String
}

enum FreeUpSpaceModel {
    static let dayChoices = [30, 90, 180]
    static let defaultDays = 90

    /// The cleanups worth offering now. One that would free nothing is left out, except
    /// compressing, which shows while any recording is still full quality.
    static func options(space: CoreDbSpace?, usage: CoreMediaUsage?, deletableAudio: (Int) -> Int64,
                        oldCaptures: (Int) -> (count: Int64, textBytes: Int64), days: Int) -> [FreeUpSpaceOption] {
        func size(_ b: Int64) -> String { ByteCountFormatter.string(fromByteCount: b, countStyle: .file) }
        var out: [FreeUpSpaceOption] = []
        if let space, space.freeBytes > 0 {
            out.append(.init(kind: .compactDatabase, frees: space.freeBytes,
                             detail: String(localized: "Frees \(size(space.freeBytes)) inside the database file.")))
        }
        if let usage, usage.audioFullBytes > 0 {
            out.append(.init(kind: .compressAudio, frees: nil,
                             detail: String(localized: "\(size(usage.audioFullBytes)) of recordings are full quality. Compressing keeps them playable at a smaller size.")))
        }
        let audio = deletableAudio(days)
        if audio > 0 {
            out.append(.init(kind: .deleteOldAudio(days: days), frees: audio,
                             detail: String(localized: "Recordings older than \(days) days: \(size(audio)). Transcripts, notes and summaries stay.")))
        }
        let captures = oldCaptures(days)
        if captures.count > 0 {
            out.append(.init(kind: .deleteOldCaptures(days: days), frees: captures.textBytes,
                             detail: String(localized: "\(captures.count) screen captures older than \(days) days (about \(size(captures.textBytes)) of text).")))
        }
        return out
    }

    /// What the sheet says when a cleanup ends; nil says nothing. A failed step says so instead
    /// of "Freed …": deleting old captures (`retentionFailed`), or a compact the user asked for
    /// (`forced`) that failed or didn't run.
    static func finishMessage(retentionFailed: Bool, forced: Bool, compacted: CoreCompactResult?, volumeFree: Int64?,
                              before: Int64?, after: Int64) -> String? {
        if retentionFailed { return String(localized: "Couldn't delete old screen captures.") }
        if forced, compacted?.ran != true { return CompactCopy.message(compacted, volumeFree: volumeFree) }
        guard let before else { return nil }
        return String(localized: "Freed \(ByteCountFormatter.string(fromByteCount: max(0, before - after), countStyle: .file)).")
    }
}

/// Tells the latest of several overlapping refreshes apart, so an older one landing last
/// doesn't overwrite the newer one's result.
struct RefreshGeneration {
    private(set) var current = 0

    /// Starts a refresh and returns its token.
    mutating func begin() -> Int {
        current += 1
        return current
    }

    func isCurrent(_ token: Int) -> Bool { token == current }
}
