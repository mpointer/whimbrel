import SwiftUI

// Which Settings search anchors are drawn right now. A row that exists only in some states (Manage subscription on a Free
// plan, This device's name when signed out) has an anchor in the registry but no view, and `scrollTo` says nothing about an
// id it cannot find. Every anchored row reports itself here, so the window can tell "found" from "not shown".

/// Counts, not a set: when SwiftUI swaps a row for a new one it may run the new row's `onAppear` before the old row's
/// `onDisappear`, and a set would then lose the row.
@MainActor
public final class WAnchorPresence {
    private var counts: [String: Int] = [:]

    public init() {}

    func appeared(_ id: String) { counts[id, default: 0] += 1 }

    func disappeared(_ id: String) {
        guard let n = counts[id] else { return }
        if n <= 1 { counts[id] = nil } else { counts[id] = n - 1 }
    }

    public func contains(_ id: String) -> Bool { counts[id] != nil }

    /// Waits until the row is drawn, up to `seconds`: a section can still be laying out after the choice, and a collapsed group
    /// opens first. True as soon as the row is there, false when the time ran out or the task was cancelled.
    public func waitFor(_ id: String, seconds: Double = 1.5) async -> Bool {
        let step: UInt64 = 100_000_000
        var waited: UInt64 = 0
        let limit = UInt64(seconds * 1_000_000_000)
        while !contains(id) {
            if waited >= limit || Task.isCancelled { return false }
            try? await Task.sleep(nanoseconds: step)
            waited += step
        }
        return true
    }
}

private struct WAnchorPresenceKey: EnvironmentKey {
    static let defaultValue: WAnchorPresence? = nil
}

extension EnvironmentValues {
    var wAnchorPresence: WAnchorPresence? {
        get { self[WAnchorPresenceKey.self] }
        set { self[WAnchorPresenceKey.self] = newValue }
    }
}

private struct WAnchorMark: ViewModifier {
    @Environment(\.wAnchorPresence) private var presence
    let id: String

    func body(content: Content) -> some View {
        content
            .id(id)
            .onAppear { presence?.appeared(id) }
            .onDisappear { presence?.disappeared(id) }
    }
}

extension View {
    /// A Settings search anchor: the scroll target, and a report that the row is on screen.
    public func wAnchor(_ id: String) -> some View { modifier(WAnchorMark(id: id)) }
}

// MARK: What to say after a hit

/// What the window says after a search hit.
public enum WJumpNotice: Equatable, Sendable {
    /// The row is there (said to a screen reader; the mark on the row is the sighted cue).
    case shown(String)
    /// The section is open but the row is not drawn in this state (said to everyone).
    case hidden(String)

    /// The words, whichever case it is.
    public var text: String {
        switch self {
        case .shown(let s), .hidden(let s): return s
        }
    }

    /// - shownFormat: "Showing %@." in the app's language. hiddenText: "That setting isn't shown right now."
    public static func after(found: Bool, title: String, shownFormat: String, hiddenText: String) -> WJumpNotice {
        found ? .shown(String(format: shownFormat, title)) : .hidden(hiddenText)
    }
}
