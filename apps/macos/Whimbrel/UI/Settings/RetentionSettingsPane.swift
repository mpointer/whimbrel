import SwiftUI
import AppKit

/// The Media section's usage and preview lines.
enum MediaRetentionCopy {
    static func usage(_ u: CoreMediaUsage) -> String {
        func size(_ bytes: Int64) -> String { ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file) }
        return "Audio: \(size(u.audioFullBytes)) full quality, \(size(u.audioCompressedBytes)) compressed. Images: \(size(u.imageBytes))."
    }

    /// Counts meetings, not steps: a meeting moving both audio and images is one move.
    /// Next to the "Delete audio after" switch while it's off.
    static func deleteSwitchNote(_ u: CoreMediaUsage, deleteOn: Bool) -> String? {
        guard !deleteOn else { return nil }
        guard u.audioDeletableBytes > 0 else { return "Turning this on would free nothing yet." }
        return "Turning this on would free \(ByteCountFormatter.string(fromByteCount: u.audioDeletableBytes, countStyle: .file)) now."
    }

    static func progress(step: CoreMediaStep?, done: Int, total: Int) -> String {
        let verb: String
        switch step {
        case .compress: verb = "Compressing…"
        case .shrinkImages: verb = "Shrinking images…"
        case .deleteAudio: verb = "Deleting audio…"
        case .moveAudio, .moveSnapshots: verb = "Moving files…"
        case nil: verb = "Working…"
        }
        return "\(verb) \(done) of \(total)"
    }

    /// Everything due now (a run takes up to `MediaRetentionRunner.maxMeetings` of it), the
    /// pace, that existing recordings count, and deletion by name whenever it's on.
    static func preview(_ p: CoreMediaPlan, policy: CoreMediaPolicy) -> String {
        func count(_ steps: Set<CoreMediaStep>) -> Int { p.work.filter { !steps.isDisjoint(with: $0.steps) }.count }
        func meetings(_ n: Int) -> String { n == 1 ? "1 meeting" : "\(n) meetings" }
        let compress = count([.compress]), delete = count([.deleteAudio]), shrink = count([.shrinkImages])
        let move = count([.moveAudio, .moveSnapshots])
        let clauses = [
            compress > 0 ? "compress \(meetings(compress))" : nil,
            delete > 0 ? "delete audio of \(delete)" : nil,
            shrink > 0 ? "shrink images of \(shrink)" : nil,
            move > 0 ? "move \(move)" : nil,
        ].compactMap { $0 }
        var text = clauses.isEmpty ? "Nothing due now."
            : "Due now: \(clauses.joined(separator: ", ")) (\(meetings(p.work.count)) in all)."
        text += " Up to \(MediaRetentionRunner.maxMeetings) meetings per run; runs at launch, daily and after each meeting."
        text += " Includes existing recordings."
        if let days = policy.deleteAudioAfterDays {
            text += " Audio of transcribed meetings older than \(days) days is deleted permanently."
        }
        if p.disconnectedMeetings > 0 { text += " \(meetings(p.disconnectedMeetings)) on a disconnected drive." }
        return text
    }
}

/// Settings copy with counts: plural forms come from the String Catalog.
enum SettingsCopy {
    static func retentionDays(_ n: Int) -> String {
        String(localized: "\(n) days")
    }

    static func eventsFound(_ n: Int) -> String {
        String(localized: "\(n) events found in the next 7 days.")
    }
}
