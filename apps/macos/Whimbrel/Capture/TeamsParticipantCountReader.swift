import ApplicationServices
import Foundation

/// Reads Microsoft Teams's live participant count from its in-meeting Participants
/// panel.
///
/// Based on real Accessibility-tree inspection of a live, solo Teams meeting
/// (`com.microsoft.teams2`, the meeting window titled `"<meeting title> | Microsoft
/// Teams"`) — see this task's report for the full dump. Once the Participants
/// panel has been opened at least once during the call (via the meeting toolbar's
/// "People" button — an
/// `AXButton` whose `kAXDescriptionAttribute` is exactly `"People"`, not an
/// `AXPopUpButton` like Google Meet's equivalent), Teams's AX tree contains:
///
/// - An `AXStaticText` whose value is exactly `"In this meeting (N)"` (e.g.
///   `"In this meeting (1)"` in a solo call — confirming Teams's count, like
///   Zoom's and Meet's, includes the local user).
/// - Its ancestor `AXRow`/`AXGroup` pair carrying the equivalent `"In this meeting,
///   N total"` as title/description (e.g. `"In this meeting, 1 total"`).
///
/// This reader walks the window's tree looking for either exact pattern, mirroring
/// `ZoomParticipantCountReader`'s value/description fallback approach.
///
/// Real-world caveats found during investigation:
/// - Same as `ZoomParticipantCountReader`'s Participants-panel caveat: Teams does
///   **not** expose the "In this meeting" node at all until the People panel has
///   been opened once in the meeting UI. This reader has no way to force the panel
///   open, so it correctly returns nil in that case — a later poll may succeed once
///   the user opens the panel.
/// - Teams's People panel can show additional sections (e.g. "Suggested"/invited
///   participants not yet joined) alongside "In this meeting" in a larger meeting;
///   only "In this meeting" was observed in the real solo call available for this
///   investigation, and this reader deliberately keys on that section only rather
///   than summing every numbered section — same shape as
///   `GoogleMeetParticipantCountReader`'s Contributors/Viewers caveat.
/// - **Significant, separate finding, beyond the panel caveat above:** Teams's
///   desktop app renders its meeting UI in an embedded Chromium-based WebView
///   (Microsoft Edge WebView2), and — like the Chrome case already documented in
///   `BrowserMeetingCheck`'s doc comment and `docs/05-roadmap.md`'s FR-MTG-2
///   paragraph — that renderer's accessibility tree is lazily activated. Confirmed
///   directly: a from-scratch dump of a real, already-running Teams meeting window
///   returned only ~15-20 nodes, all bare `AXGroup`s with empty
///   title/value/description — no toolbar, no buttons, no text, nothing — even
///   after several seconds of repeated polling with no activation call. The only
///   thing that populates real content (toolbar buttons, "People"/"Chat"
///   descriptions, eventually the Participants panel) is explicitly calling
///   `AXUIElementSetAttributeValue(_, "AXEnhancedUserInterface", true)` — the exact
///   same call already flagged in `docs/05-roadmap.md` as "a real design decision,
///   deliberately deferred rather than bundled into this pass" for Chrome. **Per
///   an explicit decision from the human partner, this reader intentionally makes
///   that call itself** (see `activateEnhancedAccessibility`), accepting the
///   side effect on Teams's own accessibility-mode behavior rather than shipping a
///   reader that would otherwise never find anything under ordinary polling.
///   Two scoping/timing details confirmed live and worth preserving:
///   - The attribute must be set on the **application** element (derived from the
///     window's own pid via `AXUIElementGetPid`, then `AXUIElementCreateApplication`)
///     — setting it directly on the *window* element was confirmed live to fail
///     silently (`kAXErrorAttributeUnsupported`, no effect on the tree at all).
///   - Activation is not synchronous: toggling it off and back on live showed the
///     tree take roughly 0.1-1s to repopulate. Confirmed live that this means the
///     very first call against a freshly-cold window can still return nil (the
///     walk runs before the tree has repopulated), but the very next poll ~15s
///     later reliably succeeds — this reader does not block/sleep waiting for
///     activation to land, since `ParticipantCountEstimator.poll()` runs on
///     `@MainActor` and a synchronous sleep there would hitch the UI.
///   - Confirmed idempotent: setting the attribute repeatedly (every poll, in
///     production) causes no errors and no adverse effect — the formal return
///     code is always `kAXErrorNotImplemented` (`-25208`) regardless of whether
///     the underlying WebView2 bridge actually accepted it, which it does.
/// - "In this meeting" is an English-only label — same constraint
///   `ZoomParticipantCountReader` has with "Participants" and
///   `GoogleMeetParticipantCountReader` has with "People"/"Contributors".
///
/// **A real side effect this reader has, and why its worst case is bounded:**
/// `activateEnhancedAccessibility` mutates Teams's process-wide accessibility mode
/// — Chromium/WebView2 activates that mode lazily specifically because it has a
/// real CPU/memory cost, so calling it is not free. `MeetingHeuristicDetector` can
/// false-positive-detect a "meeting" purely from Teams running idle in the
/// background (same known limitation already documented there for Slack); without
/// mitigation, that would mean repeatedly toggling this mode on every ~15s poll
/// (more often still, since a poll can try more than one candidate window) for as
/// long as the false positive lasts. See
/// `maxConsecutiveEmptyReadsBeforePausingActivation` for the mitigation.
final class TeamsParticipantCountReader: MeetingParticipantCountReader {
    // Task 7's real investigation found the "In this meeting (N)" static text at
    // depth 21 from the window root, in a tree that had fully repopulated to ~126
    // nodes after activation (up from ~15-20 sparse nodes beforehand). 1000/25
    // keeps real margin above both the measured depth and node count. Unlike Google
    // Meet's budget, this isn't just margin-over-what-was-measured: the dump showed
    // the "In this meeting, N total" AXRow as the *first* child under the
    // "Attendees" AXOutline, ahead of the per-participant rows — so DFS reaches the
    // target before it descends into a larger meeting's additional participant
    // rows, meaning a bigger call doesn't need a bigger budget to find it. 1000
    // nowhere near the old 4000 figure, which read arbitrary next to Zoom's 2000/15
    // and Google Meet's budget.
    private static let maxDepth = 25
    private static let maxNodes = 1000

    /// After this many consecutive *reads* (not polls — `ParticipantCountEstimator.
    /// poll()` can call `readParticipantCount(from:)` more than once per poll, once
    /// per distinct candidate window it's trying; see its `resolveWindowCandidates()`)
    /// where activation was attempted and the tree walk still found no participant
    /// count (not just "panel not opened yet" in a real meeting — genuinely nothing,
    /// which is indistinguishable from the idle-Teams false-positive case from this
    /// reader's vantage point), stop calling `activateEnhancedAccessibility`. Set
    /// high enough to cover several polls' worth of reads even when a poll tries
    /// more than one distinct window, not just a single poll. This is safe for a
    /// real meeting, not just a cost-saving shortcut: activation is confirmed
    /// idempotent and sticky once it lands (Task 7's investigation found the tree
    /// "stayed stably populated... throughout" once activated), so a real meeting's
    /// window keeps yielding a count once the user opens the Participants panel
    /// even after this reader stops re-triggering activation — only the redundant,
    /// costly re-activation calls stop, not the passive tree walk itself.
    private static let maxConsecutiveEmptyReadsBeforePausingActivation = 8

    /// Consecutive reads (since the last non-nil read, or since this reader was
    /// created) where activation was attempted but no count was found — a "read" is
    /// one call to `readParticipantCount(from:)`, which may happen more than once
    /// per poll (see `maxConsecutiveEmptyReadsBeforePausingActivation`). Mutable
    /// per-read state is why this reader is a `class` rather than the stateless
    /// struct the other `MeetingParticipantCountReader` conformers are.
    private var consecutiveEmptyReads = 0

    func readParticipantCount(from window: AXUIElement) -> Int? {
        if consecutiveEmptyReads < Self.maxConsecutiveEmptyReadsBeforePausingActivation {
            Self.activateEnhancedAccessibility(for: window)
        }
        var visited = 0
        let count = Self.findCount(in: window, depth: 0, visited: &visited)
        consecutiveEmptyReads = count == nil ? consecutiveEmptyReads + 1 : 0
        return count
    }

    /// Triggers Chromium/WebView2's lazy accessibility-tree activation for the
    /// process that owns `window` — see the type-level doc comment for the full
    /// investigation. Best-effort: the formal AX return code from this call is
    /// always `kAXErrorNotImplemented` (confirmed live), so its result is
    /// deliberately ignored rather than treated as a real success/failure signal;
    /// the only observable effect is whether the *next* tree-walk finds content.
    /// Safe to call on every poll — confirmed live to be idempotent with no
    /// adverse effect once already active.
    private static func activateEnhancedAccessibility(for window: AXUIElement) {
        var pid: pid_t = 0
        guard AXUIElementGetPid(window, &pid) == .success else { return }
        let appElement = AXUIElementCreateApplication(pid)
        _ = AXUIElementSetAttributeValue(appElement, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    }

    private static func findCount(in element: AXUIElement, depth: Int, visited: inout Int) -> Int? {
        guard depth < maxDepth, visited < maxNodes else { return nil }
        visited += 1

        if let value = AXTextWalker.stringAttribute(element, kAXValueAttribute),
           let count = parseInThisMeetingValue(from: value) {
            return count
        }
        for attribute in [kAXTitleAttribute, kAXDescriptionAttribute] {
            if let text = AXTextWalker.stringAttribute(element, attribute),
               let count = parseInThisMeetingTotal(from: text) {
                return count
            }
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

    /// Parses Teams's real Participants-panel `AXStaticText` value format, exactly
    /// as found live: `"In this meeting (N)"` (e.g. `"In this meeting (1)"`). Not
    /// `private` so it can be unit tested directly — same `AXUIElement`
    /// non-constructibility constraint noted on `ZoomParticipantCountReader`.
    static func parseInThisMeetingValue(from text: String) -> Int? {
        let prefix = "In this meeting ("
        guard text.hasPrefix(prefix), text.hasSuffix(")") else { return nil }
        let inner = text.dropFirst(prefix.count).dropLast()
        guard !inner.isEmpty else { return nil }
        return Int(inner)
    }

    /// Parses Teams's real Participants-panel row/group title-or-description
    /// format, exactly as found live: `"In this meeting, N total"` (e.g. `"In this
    /// meeting, 1 total"`). Not `private` for the same testing reason as
    /// `parseInThisMeetingValue`.
    static func parseInThisMeetingTotal(from text: String) -> Int? {
        let prefix = "In this meeting, "
        let suffix = " total"
        guard text.hasPrefix(prefix), text.hasSuffix(suffix) else { return nil }
        let inner = text.dropFirst(prefix.count).dropLast(suffix.count)
        guard !inner.isEmpty else { return nil }
        return Int(inner)
    }
}
