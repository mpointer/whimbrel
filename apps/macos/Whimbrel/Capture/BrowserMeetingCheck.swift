import ApplicationServices
import AppKit
import Foundation

/// A lightweight, narrowly-scoped AX check for "does this browser's focused window
/// currently show a video-meeting URL" — deliberately NOT `AXTextWalker.extract()`,
/// which does a full 400-node/40-depth recursive text walk. This runs on a 5-second
/// poll continuously in the background (`MeetingHeuristicDetector.poll()`) whenever a
/// known browser happens to be frontmost, so it only reads the one attribute it needs
/// (an `AXWebArea`'s `AXURL`), bounded to a small fixed depth, and stops as soon as it
/// finds the first web area rather than needing to find every one.
enum BrowserMeetingCheck {
    /// Bundle IDs of browsers this check knows how to look at. Scoped to the two most
    /// common cases, matching this codebase's existing tolerance for a similarly
    /// incomplete native-app bundle-ID list (Teams/Webex are flagged there as
    /// unverified too) — Arc/Edge/Firefox aren't covered. **Chrome is present in this
    /// set but non-functional as shipped**: Chromium's renderer accessibility tree is
    /// lazily activated and nothing triggers that activation under ordinary polling, so
    /// `webAreaURL` never finds a web area in a real Chrome window — root-caused in
    /// `docs/05-roadmap.md`'s FR-MTG-2 paragraph. Left in the set anyway since it's
    /// harmless (Chrome simply never matches) and keeps a future fix a one-line
    /// re-enable rather than needing this ID re-added from scratch.
    static let knownBrowserBundleIDs: Set<String> = [
        "com.apple.Safari",
        "com.google.Chrome",
    ]

    /// A browser window's main content area is nested several levels under
    /// toolbar/tab-strip containers, not the window's direct child — bounded to a
    /// small fixed depth, not `AXTextWalker`'s 40, since this only needs to find the
    /// topmost web area, not walk the whole tree. Measured against a real running
    /// Safari (manual verification, Task 6): its `AXWebArea` sits at depth 6
    /// (`AXWindow`→`AXSplitGroup`→`AXTabGroup`→`AXGroup`→`AXGroup`→`AXScrollArea`→
    /// `AXWebArea`). A depth-8 figure for Chrome was also observed during that same
    /// pass, but only after forcing `AXUIElementSetAttributeValue(...,
    /// "AXEnhancedUserInterface", true)` on Chrome's process — under ordinary polling
    /// (no such call made), Chrome's web area is not reachable at all, so that number
    /// does not reflect real usage; see `docs/05-roadmap.md`'s FR-MTG-2 paragraph. 12
    /// leaves real margin above Safari's measured depth.
    private static let maxSearchDepth = 12

    /// Breadth cap mirroring `AXTextWalker.extract`'s `maxNodes` precedent — without
    /// it, a window with no reachable `AXWebArea` (Chrome under ordinary polling, a
    /// Safari Preferences/Start Page window, an AX-permission-denied state) would walk
    /// its entire accessible subtree to `maxSearchDepth` with no ceiling, one
    /// synchronous cross-process AX call per node, on the main actor.
    private static let maxNodes = 400

    /// If `app`'s bundle ID is a known browser, finds its focused window's URL (via
    /// the first `AXWebArea` found, depth-first, within `maxSearchDepth`/`maxNodes`).
    /// Returns `nil` if `app` isn't a known browser, has no focused window, or no web
    /// area is found — every failure mode collapses to "no signal here," matching this
    /// codebase's existing AX-code convention (never a crash, never a thrown error).
    /// Note: this returns *whatever* URL the focused window's web area carries, not
    /// specifically a Meet URL — callers (`MeetingHeuristicDetector.poll()`,
    /// `MeetingEndDetector`) are responsible for their own `meetingURLProvider` check.
    static func focusedWebAreaURL(for app: NSRunningApplication) -> (window: AXUIElement, urlString: String)? {
        guard let bundleID = app.bundleIdentifier, knownBrowserBundleIDs.contains(bundleID) else { return nil }

        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        var focusedWindowRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement, kAXFocusedWindowAttribute as CFString, &focusedWindowRef) == .success,
              let focusedWindowRef else { return nil }
        let windowElement = focusedWindowRef as! AXUIElement

        guard let urlString = currentURL(of: windowElement) else { return nil }
        return (windowElement, urlString)
    }

    /// Re-reads `window`'s URL directly — used by `MeetingEndDetector` to check
    /// whether a previously-found window still shows a matching URL, without redoing
    /// the frontmost-app lookup (the window doesn't need to still be frontmost; the
    /// user may have reasonably switched to another app mid-meeting). If `window` has
    /// since closed, `AXUIElementCopyAttributeValue` fails (e.g. `.invalidUIElement`)
    /// and this returns `nil`, same as any other "no signal" case.
    static func currentURL(of window: AXUIElement) -> String? {
        var visited = 0
        if let url = webAreaURL(window, depth: 0, visited: &visited) { return url }
        return nil
    }

    private static func webAreaURL(_ element: AXUIElement, depth: Int, visited: inout Int) -> String? {
        guard depth < maxSearchDepth, visited < maxNodes else { return nil }
        visited += 1

        if let role = AXTextWalker.stringAttribute(element, kAXRoleAttribute), role == "AXWebArea" {
            return AXTextWalker.urlAttribute(element)
        }

        var childrenRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
              let children = childrenRef as? [AXUIElement] else { return nil }

        for child in children {
            if visited >= maxNodes { return nil }
            if let found = webAreaURL(child, depth: depth + 1, visited: &visited) { return found }
        }
        return nil
    }
}
