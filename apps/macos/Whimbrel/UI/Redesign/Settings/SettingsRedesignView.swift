import AppKit
import SwiftUI
import WhimbrelUI

/// The Settings window with the hidden redesign switch on. `SettingsView` hands over to this at the top of its body.
/// Same sections in the same order, the same search (the `SettingsSearch` registry and matching), the same section
/// jumps from other parts of the app (`pendingSettingsSection`) and the same scroll-and-flash after a search hit.
struct SettingsRedesignView: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        SettingsRedesignHost(suggestions: appState.profileSuggestions)
    }
}

private struct SettingsRedesignHost: View {
    @EnvironmentObject var appState: AppState
    /// The Profile badge counts these.
    @ObservedObject var suggestions: ProfileSuggestionsModel
    @State private var selection: SettingsSection? = .general
    @State private var query = ""
    /// The row to scroll to once the chosen section's content is on screen.
    @State private var scrollTarget: String?
    /// The same row, for a section that keeps it in a collapsed group (Connectors, Advanced): the group opens before the scroll.
    @State private var reveal: String?
    @StateObject private var searchState = SettingsSearchState()
    /// New each time the window closes. The secret rows (and the Connectors token sheet) listen for it and drop what was typed
    /// and not saved: closing a window orders it out and may not remove its views (`onDisappear` is not a promise). Nothing else
    /// is rebuilt, so a long export, compact or delete that is running keeps showing that it is running when the window reopens.
    @State private var secretSession = UUID()

    var body: some View {
        WSettingsWindow(
            items: SettingsRedesign.navItems(pendingSuggestions: suggestions.pending.count),
            selection: selection?.rawValue,
            onSelect: { selection = SettingsSection(rawValue: $0) },
            query: $query,
            searchPlaceholder: String(localized: "Search settings"),
            results: SettingsRedesign.results(for: query),
            onOpenResult: choose,
            pickerLabel: String(localized: "Section", comment: "Label of the menu that picks a Settings section when the window is narrow or the text is large."),
            scrollTarget: $scrollTarget,
            // A gallery click rebuilds this window (`ThemedRoot`); land back on the card, not the top.
            onScrolled: { id in
                searchState.flash(id)
                // The group has opened by now. Cleared, so the same hit chosen again is a new hand-off, and opening Connectors
                // later from the sidebar does not open Advanced on its own.
                reveal = WSettingsHandoff.afterScroll(target: reveal, done: id)
            },
            restoreScroll: { SkinStore.shared.takeGalleryScrollTarget() },
            detail: { content }
        )
        .settingsRedesignThemed()
        .environment(\.settingsSearchState, searchState)
        .background(WindowCloseObserver { secretSession = UUID() })
        .frame(minWidth: 640, minHeight: 480)
        .onAppear(perform: showPendingSection)
        .onChange(of: appState.pendingSettingsSection) { _, _ in showPendingSection() }
    }

    /// Opens the entry's section, clears the search and scrolls to the row once it is on screen.
    private func choose(_ id: String) {
        guard let section = SettingsRedesign.section(forResult: id) else { return }
        selection = section
        query = ""
        reveal = id
        scrollTarget = id
    }

    /// Lands on the section a button elsewhere asked for (`OpenAIProvidersSettingsButton`).
    private func showPendingSection() {
        guard let section = appState.pendingSettingsSection else { return }
        appState.pendingSettingsSection = nil
        query = ""
        selection = section
    }

    private var content: some View { sections.environment(\.wSecretClearToken, secretSession) }

    @ViewBuilder
    private var sections: some View {
        let highlighted = searchState.highlighted
        switch selection {
        case .general:
            GeneralSettingsRedesignView(openAISettings: { selection = .ai }, highlighted: highlighted)
        case .profile:
            ProfileSettingsRedesignView(highlighted: highlighted)
        case .plan:
            PlanSettingsRedesignView(openAISettings: { selection = .ai }, openDevices: { selection = .devices }, highlighted: highlighted)
        case .capture: MemoryPrivacySettingsRedesignView(openAISettings: { selection = .ai }, highlighted: highlighted)
        case .meetings: MeetingsSettingsRedesignView(highlighted: highlighted)
        case .ai: AISettingsRedesignView(highlighted: highlighted)
        case .topics: TopicsSettingsRedesignView(highlighted: highlighted)
        case .devices: SyncDevicesSettingsRedesignView(highlighted: highlighted)
        case .connectors: ConnectorsSettingsRedesignView(highlighted: highlighted, reveal: reveal)
        case .automations: AutomationsSettingsRedesignView(highlighted: highlighted)
        case .diagnostics: DiagnosticsSettingsRedesignView()
        case .none:
            Text("Select a section").foregroundStyle(Color.whimbrelTextSecondary)
        }
    }
}

/// A classic pane inside the redesigned window, in the classic card, until its section is redrawn.
struct ClassicPaneCard<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .whimbrelCard()
    }
}

/// Tells the app when the window this view is in closes (`NSWindow.willCloseNotification`), whether or not SwiftUI removes
/// the view.
private struct WindowCloseObserver: NSViewRepresentable {
    let onClose: () -> Void

    func makeNSView(context: Context) -> NSView { Anchor(onClose: onClose) }
    func updateNSView(_ view: NSView, context: Context) { (view as? Anchor)?.onClose = onClose }

    private final class Anchor: NSView {
        var onClose: () -> Void
        private var token: NSObjectProtocol?
        init(onClose: @escaping () -> Void) {
            self.onClose = onClose
            super.init(frame: .zero)
        }
        required init?(coder: NSCoder) { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let token { NotificationCenter.default.removeObserver(token); self.token = nil }
            guard let window else { return }
            token = NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak self] _ in
                self?.onClose()
            }
        }

        deinit { if let token { NotificationCenter.default.removeObserver(token) } }
    }
}
