import SwiftUI

// Tell a friend (docs/superpowers/plans/2026-11-24-referral-and-share.md, rollout row 1): a quiet row in the menu bar menu and a
// card in Settings, Sync & Devices. Both copy a short message with a plain link to whimbrelapp.com. No account is needed, no code
// and no reward is part of the message, nothing opens and nothing is sent, and the app learns nothing about who installs it.

/// The message a person copies. The words are the app's (a localized template); this is the one place that puts the link in.
public enum WTellAFriend {
    /// How long the row and the button say "Copied" before they go back to their name.
    public static let copiedSeconds: Double = 4

    /// The message for a localized `template` with one `%@` where the link goes. A template a translation damaged (no `%@`,
    /// or more than one) still gives a message with the link in it exactly once, at the end, so a copy never loses the link.
    public static func message(template: String, link: String = WTellAFriend.link) -> String {
        let marker = "%@"
        let parts = template.components(separatedBy: marker)
        if parts.count == 2 { return parts[0] + link + parts[1] }
        let words = template.replacingOccurrences(of: marker, with: "").trimmingCharacters(in: .whitespacesAndNewlines)
        return words.isEmpty ? link : words + " " + link
    }
}

/// The Settings card. `copy` writes the message to the pasteboard and says whether it worked: "Copied" shows only then.
public struct WTellAFriendCard: View {
    @Environment(\.wTheme) private var theme
    @State private var state = WCopyFlash()
    let labels: WTellAFriendLabels
    let copy: () -> Bool

    public init(labels: WTellAFriendLabels = WTellAFriendLabels(), copy: @escaping () -> Bool) {
        self.labels = labels
        self.copy = copy
    }

    public var body: some View {
        WSettingsCard(heading: labels.heading, description: labels.blurb) {
            wBoxRow(theme) {
                VStack(alignment: .leading, spacing: theme.space(6)) {
                    WButton(
                        state.copied ? labels.copied : labels.copyButton, glyph: state.copied ? "checkmark" : "doc.on.doc", kind: .secondary
                    ) {
                        let ok = copy()
                        state.finish(succeeded: ok)
                        if ok { AccessibilityNotification.Announcement(labels.copied).post() }
                    }
                    if state.failed { WErrorLine(text: labels.failed) }
                }
            }
        }
        .task(id: state.generation) {
            guard state.copied else { return }
            try? await Task.sleep(nanoseconds: UInt64(WTellAFriend.copiedSeconds * 1_000_000_000))
            if !Task.isCancelled { state.reset() }
        }
    }
}

/// "Copied" for a few seconds after a copy that worked, or a failure line after one that did not. A pure value so the rule can be
/// tested: each finished copy starts a new generation, which restarts the timer that clears it.
struct WCopyFlash: Equatable {
    private(set) var copied = false
    private(set) var failed = false
    private(set) var generation = 0

    mutating func finish(succeeded: Bool) {
        copied = succeeded
        failed = !succeeded
        generation += 1
    }

    mutating func reset() {
        copied = false
        failed = false
    }
}
