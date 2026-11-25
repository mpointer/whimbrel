import Foundation

/// The read-only "Local-only mode" line in Capture & Privacy. The setting itself lives in the AI pane.
enum LocalOnlyLine {
    static func text(localOnly: Bool) -> String {
        localOnly ? String(localized: "Local-only mode: On") : String(localized: "Local-only mode: Off")
    }
}
