import SwiftUI

// A skin's motion, applied to interface transitions only (docs/superpowers/plans/2026-11-25-richer-skins.md, section 6).
// Nothing here is used by the recording indicator, the capture control, the consent banner or approvals.

/// Which duration of the theme's motion a view reads.
enum WMotionKind {
    case control, marker, panel

    func seconds(_ motion: WMotion) -> Double {
        switch self {
        case .control: return motion.control
        case .marker: return motion.marker
        case .panel: return motion.panel
        }
    }
}

private struct WMotionAnimation<V: Equatable>: ViewModifier {
    @Environment(\.wTheme) private var theme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let kind: WMotionKind
    let value: V

    func body(content: Content) -> some View {
        let seconds = kind.seconds(theme.motion.respecting(reduceMotion: reduceMotion))
        if seconds > 0 {
            content.animation(WMotion.animation(seconds), value: value)
        } else {
            content
        }
    }
}

/// A Home section fades in and rises once, on first show. With no motion the section is there from the first frame.
private struct WSectionAppear: ViewModifier {
    @Environment(\.wTheme) private var theme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = false

    func body(content: Content) -> some View {
        let motion = theme.motion.respecting(reduceMotion: reduceMotion)
        if motion.section > 0 {
            content
                .opacity(shown ? 1 : 0)
                .offset(y: shown ? 0 : motion.rise)
                .onAppear {
                    guard !shown else { return }
                    withAnimation(WMotion.animation(motion.section)) { shown = true }
                }
        } else {
            content
        }
    }
}

extension View {
    /// Eases a change of `value` over the skin's duration for `kind`. No change at all when that duration is zero.
    func wMotion<V: Equatable>(_ kind: WMotionKind, value: V) -> some View {
        modifier(WMotionAnimation(kind: kind, value: value))
    }

    /// Fade and rise on first show, for a Home section.
    func wSectionAppear() -> some View { modifier(WSectionAppear()) }
}
