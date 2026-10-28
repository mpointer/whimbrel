import SwiftUI

/// Re-renders a screen's content when the theme changes (colors are read from `Color.theme`
/// at render time) without touching the state that lives above it: the selected tab, the
/// navigation stack and the models stay as they are.
private struct Themed: ViewModifier {
    @ObservedObject private var themes = ThemeStore.shared

    func body(content: Content) -> some View {
        content.id(themes.revision)
    }
}

extension View {
    func themed() -> some View { modifier(Themed()) }
}
