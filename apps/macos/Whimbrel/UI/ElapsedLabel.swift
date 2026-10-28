import Foundation

/// Menu bar recording time as "h:mm" (minutes truncated; 59 s shows "0:00").
enum ElapsedLabel {
    static func text(_ elapsed: TimeInterval) -> String {
        let minutes = Int(max(0, elapsed)) / 60
        return String(format: "%d:%02d", minutes / 60, minutes % 60)
    }
}
