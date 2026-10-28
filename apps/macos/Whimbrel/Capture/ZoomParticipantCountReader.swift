import ApplicationServices
import Foundation

/// Reads Zoom's live participant count from its in-meeting Participants panel.
///
/// Based on real Accessibility-tree inspection of a live Zoom meeting (`us.zoom.xos`,
/// "Zoom Meeting" window) — see this task's report for the full dump. Once the
/// Participants panel has been opened at least once during the call, Zoom's AX tree
/// contains a static text node whose value is exactly `"Participants (N)"` (e.g.
/// `"Participants (1)"` in a solo call), alongside an `AXUnknown`-roled panel-header
/// node carrying the same text as its description. This reader walks the window's
/// tree looking for that exact pattern on either a node's value or description.
///
/// Important real-world caveat found during investigation: Zoom does **not** expose
/// this node at all until the Participants panel has been opened once in the meeting
/// UI — before that, the window's AX tree is sparse (just the audio/video toolbar
/// buttons, no participant list or count anywhere). This reader has no way to force
/// the panel open, so it correctly returns nil in that case, same as any other
/// "expected element not found" case — a later poll may succeed once the user (or
/// Zoom itself) opens the panel.
struct ZoomParticipantCountReader: MeetingParticipantCountReader {
    private static let maxDepth = 15
    private static let maxNodes = 2000

    func readParticipantCount(from window: AXUIElement) -> Int? {
        var visited = 0
        return Self.findCount(in: window, depth: 0, visited: &visited)
    }

    private static func findCount(in element: AXUIElement, depth: Int, visited: inout Int) -> Int? {
        guard depth < maxDepth, visited < maxNodes else { return nil }
        visited += 1

        if let value = AXTextWalker.stringAttribute(element, kAXValueAttribute),
           let count = parseParticipantCount(from: value) {
            return count
        }
        if let description = AXTextWalker.stringAttribute(element, kAXDescriptionAttribute),
           let count = parseParticipantCount(from: description) {
            return count
        }

        var childrenRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
              let children = childrenRef as? [AXUIElement] else { return nil }

        for child in children {
            if visited >= maxNodes { return nil }
            if let count = findCount(in: child, depth: depth + 1, visited: &visited) {
                return count
            }
        }
        return nil
    }

    /// Parses Zoom's real `"Participants (N)"` format, exactly as found on the
    /// Participants panel's header static text and its `AXUnknown` parent's
    /// description (e.g. `"Participants (1)"`). Not `private` so it can be unit
    /// tested directly — `AXUIElement` itself is a live, system-owned opaque type
    /// that can't be reasonably constructed in a unit test (same constraint noted
    /// on `AXTextWalker`/`ScreenTextCaptureService`), but this string-parsing logic
    /// is a pure function and is exercised directly by
    /// `ZoomParticipantCountReaderTests`.
    static func parseParticipantCount(from text: String) -> Int? {
        let prefix = "Participants ("
        guard text.hasPrefix(prefix), text.hasSuffix(")") else { return nil }
        let inner = text.dropFirst(prefix.count).dropLast()
        guard !inner.isEmpty else { return nil }
        return Int(inner)
    }
}
