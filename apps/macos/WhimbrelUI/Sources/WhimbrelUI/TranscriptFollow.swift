import SwiftUI

/// Whether the live transcript follows its newest line. Pure, so the rules are tested without a window.
///
/// The transcript sits inside the column's scroll view, so "at the bottom" means the bottom of the transcript card is on screen
/// (within `tolerance` points of the scroll view's lower edge). Following starts on. A new line scrolls to it while following. The
/// person scrolling up stops it, and scrolling back to the bottom starts it again. A highlighted line (an alert jump or a cited
/// line) pauses it for as long as the highlight lasts.
struct WTranscriptFollow: Equatable {
    static let tolerance: CGFloat = 48
    /// A new line makes the content grow before the scroll catches up, so the first reports after a scroll request can read
    /// "not at the bottom". This many are ignored, then a report is believed whatever it says.
    static let growthReports = 2

    private(set) var following = true
    private(set) var highlightActive = false
    private var ignoring = 0

    /// A new newest line arrived. True when the view should scroll to it.
    mutating func newLine() -> Bool {
        guard following, !highlightActive else { return false }
        ignoring = Self.growthReports
        return true
    }

    /// Where the bottom of the transcript is: points it lies below the lower edge of the scroll view (zero or less when visible).
    mutating func position(distanceBelow: CGFloat) {
        if distanceBelow <= Self.tolerance { ignoring = 0; following = true; return }
        if ignoring > 0 { ignoring -= 1; return }
        following = false
    }

    /// A line is highlighted (true) or the highlight ended (false). Ending does not resume following by itself: the jump moved
    /// the view, and following resumes when the person is back at the bottom.
    mutating func highlight(active: Bool) {
        highlightActive = active
        if active { ignoring = 0 }
    }
}

enum WTranscriptScrollSpace { static let name = "wLiveColumnScroll" }

private struct WTranscriptViewportKey: EnvironmentKey { static let defaultValue: CGFloat = 0 }

extension EnvironmentValues {
    /// The height of the scroll view the transcript sits in. Zero when it is not in one, which turns following off.
    var wTranscriptViewport: CGFloat {
        get { self[WTranscriptViewportKey.self] }
        set { self[WTranscriptViewportKey.self] = newValue }
    }
}

struct WViewportHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

struct WTranscriptBottomKey: PreferenceKey {
    static let defaultValue: CGFloat? = nil
    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) { value = nextValue() ?? value }
}
