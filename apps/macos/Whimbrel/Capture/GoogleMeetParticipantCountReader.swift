import ApplicationServices
import Foundation

/// Reads Google Meet's live participant count from the browser tab's `AXWebArea`
/// subtree.
///
/// Based on real Accessibility-tree inspection of a live, solo Google Meet call —
/// investigated in **both** Chrome and Safari, since they expose Meet's UI
/// differently (see this task's report for the full dumps) and
/// `BrowserMeetingCheck` documents Chrome's renderer accessibility tree as not
/// reliably reachable under ordinary polling, making **Safari the browser this
/// reader actually needs to work in for production**.
///
/// Two independent real findings, checked in priority order:
///
/// 1. **The "Contributors N" panel header (confirmed working in both browsers).**
///    Google Meet's in-call toolbar has an `AXPopUpButton` titled exactly `"People"`
///    that opens a side panel. Once that panel has been opened at least once during
///    the call, the panel contains an `AXButton` whose title is exactly
///    `"Contributors N"` (e.g. `"Contributors 1"` in a solo call — confirming
///    Meet's count, like Zoom's, includes the local user). Confirmed in Safari by
///    pressing the "People" button via `AXUIElementPerformAction` and re-reading the
///    tree. This needs the panel opened first, same caveat as
///    `ZoomParticipantCountReader`'s "Participants (N)" header.
/// 2. **A passive toolbar badge (confirmed in Chrome only, not reachable in
///    Safari).** In Chrome, nested a few levels beneath the "People" button (through
///    plain `AXGroup` wrapper nodes, not a fixed depth) is a bare `AXStaticText`
///    whose `AXValue` is the participant count as plain digits — visible from the
///    moment the call starts, no panel needed. In Safari, by contrast, that same
///    "People" button is a leaf node with no children at all until the panel is
///    opened — this path was directly confirmed to find nothing there. Since
///    Chrome isn't the browser this reader runs against in practice today (per
///    `BrowserMeetingCheck`'s documented limitation), this is kept only as a
///    zero-cost opportunistic fallback, not the primary signal. Whether Chrome's
///    accessibility gap can be de-risked at all (rather than accepted as-is) has a
///    real, live-verified datapoint worth cross-referencing:
///    `TeamsParticipantCountReader.activateEnhancedAccessibility` confirms, against
///    a real running Chromium-based renderer (Teams's embedded WebView2, not Chrome
///    itself, but the same underlying lazy-activation mechanism), that explicitly
///    calling `AXUIElementSetAttributeValue(..., "AXEnhancedUserInterface", true)`
///    on the process reliably populates an otherwise-empty tree — see
///    `docs/05-roadmap.md`'s FR-MTG-2 paragraph for the open Chrome question this
///    could inform.
///
/// Real-world caveats found during investigation:
/// - Both signals require the browser tab tracked by `browserWindowProvider()` to
///   still be the browser window's actual active tab. If the user switches to
///   another tab in the same window (or another app is frontmost), the window's
///   `AXWebArea` content reflects whatever tab is actually showing, so neither
///   signal is found and this correctly returns nil for that poll — same
///   graceful-degradation contract as `ZoomParticipantCountReader`'s "Workplace
///   window focused" caveat. Confirmed directly: switching the tracked window's
///   active tab away from the Meet tab and back reproduced nil, then a correct
///   re-read.
/// - Joining the same Meet call a second time from the same Google account (used
///   here to inspect Safari's mapping side-by-side with the already-open Chrome
///   tab) ended the first browser's session rather than showing 2 participants —
///   Meet treats it as one still-nil-worthy account, not two participants. This
///   means the N>1 path could not be directly observed; only a synthetic
///   multi-digit case is covered by the unit tests.
/// - Only verified with a solo call. Meet is known to show a truncated/overflow
///   badge (e.g. "9+") at high participant counts on the toolbar badge — not
///   observed directly — so `parseBareDigitCount` requires the text to be *all*
///   digits and returns nil otherwise. Large meetings/webinars may also split the
///   People panel into separate "Contributors N" / "Viewers N" sections; only the
///   "Contributors" section was observed (a small, standard call), so this reader
///   does not attempt to sum multiple sections.
/// - "People" / "Contributors" are English-only labels — same constraint
///   `ZoomParticipantCountReader` has with "Participants".
struct GoogleMeetParticipantCountReader: MeetingParticipantCountReader {
    // Task 6's real investigation found the "Contributors N" panel header (the
    // signal that actually matters — see the type-level doc comment on why Safari,
    // not Chrome, is the browser this reader needs to work in) at depth 8 from the
    // window root once the People panel is open. The Chrome-only bare-digit
    // fallback badge was found deeper, at depth 25 (nested under the "People"
    // AXPopUpButton, itself at depth 22). maxDepth=30 is a real measurement (25) plus
    // margin. maxNodes is NOT a measurement — Task 6 recorded depth positions but
    // never a total node count for either tree, and DFS child order in Meet's UI
    // (whether the "Side panel" group with "Contributors N" comes before or after
    // the video-tile grid) is unknown, so a larger call's participant tiles could
    // plausibly precede the panel and consume a chunk of this budget first. Kept at
    // the old ceiling (4000) for that reason, even though the walk itself is now
    // fused (one walk instead of what used to be up to three separate ones over the
    // same tree). The AX-call reduction from fusing isn't a flat 2-3x in every case
    // — the fused walk reads one extra attribute (kAXRoleAttribute) per node versus
    // the old primary-signal walk alone — but the case that used to cost the most
    // (nothing found anywhere, all three walks running to completion) now costs one
    // walk instead of three, which is the real win.
    private static let maxDepth = 30
    private static let maxNodes = 4000

    func readParticipantCount(from window: AXUIElement) -> Int? {
        var visited = 0
        var fallback: Int?
        if let count = Self.search(window, depth: 0, visited: &visited, insidePeopleButtonSubtree: false, fallback: &fallback) {
            return count
        }
        return fallback
    }

    /// Single fused tree walk checking both signals per node, sharing one
    /// visited/depth budget instead of the three separate walks (each resetting its
    /// own `visited` count) this used to do over the same tree. The primary signal
    /// ("Contributors N", checked first at every node) short-circuits the walk the
    /// moment it's found. The Chrome-only bare-digit fallback is only a real
    /// candidate once the walk has descended into the "People" button's own
    /// subtree (`insidePeopleButtonSubtree`), matching the pre-fusion
    /// implementation's scoping so it still can't accidentally match an unrelated
    /// numeric label elsewhere on the page; it's recorded into `fallback` (first
    /// match wins) but
    /// the walk keeps going in case the primary signal is found later in traversal
    /// order, since the primary signal always wins when both are present.
    private static func search(
        _ element: AXUIElement,
        depth: Int,
        visited: inout Int,
        insidePeopleButtonSubtree: Bool,
        fallback: inout Int?
    ) -> Int? {
        guard depth < maxDepth, visited < maxNodes else { return nil }
        visited += 1

        for attribute in [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute] {
            if let text = AXTextWalker.stringAttribute(element, attribute),
               let count = parseContributorsCount(from: text) {
                return count
            }
        }

        var isInsidePeopleButtonSubtree = insidePeopleButtonSubtree
        if !isInsidePeopleButtonSubtree,
           AXTextWalker.stringAttribute(element, kAXRoleAttribute) == "AXPopUpButton",
           AXTextWalker.stringAttribute(element, kAXTitleAttribute) == "People" {
            isInsidePeopleButtonSubtree = true
        }
        if isInsidePeopleButtonSubtree, fallback == nil,
           let value = AXTextWalker.stringAttribute(element, kAXValueAttribute),
           let count = parseBareDigitCount(from: value) {
            fallback = count
        }

        var childrenRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
              let children = childrenRef as? [AXUIElement] else { return nil }

        for child in children {
            if visited >= maxNodes { return nil }
            if let count = search(child, depth: depth + 1, visited: &visited, insidePeopleButtonSubtree: isInsidePeopleButtonSubtree, fallback: &fallback) {
                return count
            }
        }
        return nil
    }

    /// Parses the real `"Contributors N"` panel-header format (e.g.
    /// `"Contributors 1"`). Not `private` so it can be unit tested directly —
    /// `AXUIElement` itself is a live, system-owned opaque type that can't be
    /// reasonably constructed in a unit test (same constraint noted on
    /// `AXTextWalker`/`ZoomParticipantCountReader`), but this string-parsing logic
    /// is a pure function and is exercised directly by
    /// `GoogleMeetParticipantCountReaderTests`.
    static func parseContributorsCount(from text: String) -> Int? {
        let prefix = "Contributors "
        guard text.hasPrefix(prefix) else { return nil }
        let rest = text.dropFirst(prefix.count)
        guard !rest.isEmpty, rest.allSatisfy(\.isNumber) else { return nil }
        return Int(rest)
    }

    /// Parses Chrome's passive bare-digits toolbar badge (e.g. `"1"`). Requires the
    /// whole string to be digits (not just a numeric prefix) so an overflow badge
    /// like `"9+"` correctly returns nil instead of silently parsing as `9`.
    static func parseBareDigitCount(from text: String) -> Int? {
        guard !text.isEmpty, text.allSatisfy(\.isNumber) else { return nil }
        return Int(text)
    }
}
