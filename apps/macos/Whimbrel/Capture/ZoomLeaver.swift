import AppKit
import ApplicationServices
import Foundation

/// Decides which Zoom controls Whimbrel may press to leave a meeting. Exact match
/// only, so "End meeting for all" can never be pressed: Whimbrel must never end a
/// meeting for everyone.
enum ZoomLeaveMatcher {
    static func isLeaveButton(title: String?, description: String?) -> Bool {
        matches("leave", title: title, description: description)
    }

    static func isLeaveMeetingConfirm(title: String?, description: String?) -> Bool {
        matches("leave meeting", title: title, description: description)
    }

    private static func matches(_ target: String, title: String?, description: String?) -> Bool {
        [title, description].contains { text in
            let t = (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            return t == target && !t.contains("end")
        }
    }
}

/// Live Leave: presses Zoom's toolbar Leave, then its "Leave meeting" confirm.
/// Not unit-tested against a real Zoom; the matchers carry the safety guarantee
/// and this only ever presses elements they accept.
final class ZoomLeaver: ZoomLeaving {
    private static let bundleID = "us.zoom.xos"
    private static let maxDepth = 15
    private static let maxNodes = 2000

    func leave() async -> Bool {
        guard let button = Self.find(where: ZoomLeaveMatcher.isLeaveButton) else { return false }
        guard AXUIElementPerformAction(button, kAXPressAction as CFString) == .success else { return false }

        // Some Zoom versions leave on the first press, so no confirm is fine.
        // Stop early once the meeting window is gone (no confirm needed).
        var confirm: AXUIElement?
        _ = await Self.poll(seconds: 3, { () -> Bool? in
            if !Self.meetingWindowOpen() { return true }
            confirm = Self.find(where: ZoomLeaveMatcher.isLeaveMeetingConfirm)
            return confirm == nil ? nil : true
        })
        if let confirm { _ = AXUIElementPerformAction(confirm, kAXPressAction as CFString) }
        return await Self.poll(seconds: 5, { Self.meetingWindowOpen() ? nil : true }) ?? false
    }

    private static func poll<T>(seconds: Double, _ check: () -> T?) async -> T? {
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            if let value = check() { return value }
            try? await Task.sleep(nanoseconds: 250_000_000)
        } while Date() < deadline
        return nil
    }

    private static func windows() -> [AXUIElement] {
        guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID).first else { return [] }
        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        var ref: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement, kAXWindowsAttribute as CFString, &ref) == .success else { return [] }
        return ref as? [AXUIElement] ?? []
    }

    /// Uses the same title prefixes as WindowFrameSource ("Zoom Meeting ...", "Zoom Webinar").
    static func isMeetingWindowTitle(_ title: String?) -> Bool {
        let prefixes = WindowFrameSource.meetingWindowTitlePrefixes[bundleID] ?? []
        return prefixes.contains { title?.hasPrefix($0) == true }
    }

    private static func meetingWindowOpen() -> Bool {
        windows().contains { isMeetingWindowTitle(AXTextWalker.stringAttribute($0, kAXTitleAttribute)) }
    }

    private static func find(where accept: (String?, String?) -> Bool) -> AXUIElement? {
        var visited = 0
        for window in windows() {
            if let hit = search(window, depth: 0, visited: &visited, accept: accept) { return hit }
        }
        return nil
    }

    private static func search(_ element: AXUIElement, depth: Int, visited: inout Int,
                               accept: (String?, String?) -> Bool) -> AXUIElement? {
        guard depth < maxDepth, visited < maxNodes else { return nil }
        visited += 1
        if accept(AXTextWalker.stringAttribute(element, kAXTitleAttribute),
                  AXTextWalker.stringAttribute(element, kAXDescriptionAttribute)) {
            return element
        }
        var ref: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &ref) == .success,
              let children = ref as? [AXUIElement] else { return nil }
        for child in children {
            if visited >= maxNodes { return nil }
            if let hit = search(child, depth: depth + 1, visited: &visited, accept: accept) { return hit }
        }
        return nil
    }
}
