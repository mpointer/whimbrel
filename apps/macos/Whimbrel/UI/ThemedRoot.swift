import SwiftUI

/// Wraps a window's root so that choosing a theme repaints it: the colors are dynamic and read
/// the store when drawn, but SwiftUI does not know the store changed, so the root's identity
/// follows `ThemeStore.revision` and `SkinStore.revision` and the window rebuilds. A theme change is rare and happens in
/// Settings; it resets every view's `@State`, so anything that must survive it lives in a model above
/// this root (the live meeting's notes draft is reloaded from the store, its Ask lives on `LiveTranscriptionService`).
struct ThemedRoot<Content: View>: View {
    @ObservedObject private var store = ThemeStore.shared
    @ObservedObject private var skinStore = SkinStore.shared
    private let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        content.id("\(store.revision)-\(skinStore.revision)")
    }
}
