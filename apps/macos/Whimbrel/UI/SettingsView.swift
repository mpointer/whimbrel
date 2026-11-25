import SwiftUI

/// Native macOS Settings scene (⌘,): configuration only. Live status and one-shot
/// actions live in Library's header and the menu-bar popover. Each section is a stack
/// of cards, one per existing pane, so related settings share a page (docs/13 item 9).
/// The routines inbox and pending approvals live in the Automations tab, not here.
enum SettingsSection: String, CaseIterable, Identifiable {
    case general
    case profile
    case plan
    case capture
    case meetings
    case ai
    case topics
    case devices
    case connectors
    case automations
    case diagnostics

    var id: String { rawValue }

    /// Diagnostics is developer tooling, so only Debug builds list it.
    static var visibleCases: [SettingsSection] {
        #if DEBUG
        allCases
        #else
        allCases.filter { $0 != .diagnostics }
        #endif
    }

    var title: String {
        switch self {
        case .general: return String(localized: "General")
        case .profile: return String(localized: "Profile")
        case .plan: return String(localized: "Plan & billing")
        case .capture: return String(localized: "Capture & Privacy")
        case .meetings: return String(localized: "Meetings")
        case .ai: return String(localized: "AI")
        case .topics: return String(localized: "Topics")
        case .devices: return String(localized: "Sync & Devices")
        case .connectors: return String(localized: "Connectors")
        case .automations: return String(localized: "Automations")
        case .diagnostics: return String(localized: "Diagnostics")
        }
    }

    var systemImage: String {
        switch self {
        case .general: return "gearshape"
        case .profile: return "person.crop.circle"
        case .plan: return "creditcard"
        case .capture: return "eye.slash"
        case .meetings: return "calendar"
        case .ai: return "cpu"
        case .topics: return "tag"
        case .devices: return "macbook.and.iphone"
        case .connectors: return "puzzlepiece.extension"
        case .automations: return "clock.badge.checkmark"
        case .diagnostics: return "stethoscope"
        }
    }
}

struct SettingsView: View {
    @EnvironmentObject var appState: AppState
    @State private var selection: SettingsSection? = .general
    @State private var query = ""
    /// The row to scroll to once the chosen section's content is on screen.
    @State private var scrollTarget: String?
    @State private var scrollTask: Task<Void, Never>?
    @StateObject private var searchState = SettingsSearchState()

    private var isSearching: Bool { !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var body: some View {
        // The hidden redesign switch: the redrawn window, or the classic one below, exactly as it was.
        if appState.redesignEnabled {
            SettingsRedesignView()
        } else {
            classicBody
        }
    }

    private var classicBody: some View {
        NavigationSplitView {
            sidebar
                .searchable(text: $query, placement: .sidebar, prompt: "Search settings")
                .navigationSplitViewColumnWidth(180)
        } detail: {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: Spacing.lg) {
                        content
                    }
                    .padding(Spacing.xl)
                    .frame(maxWidth: 760, alignment: .leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .background(Color.whimbrelBackgroundPage)
                .task {
                    // A gallery click rebuilds this window (`ThemedRoot`); land back on the card, not the top.
                    if let card = SkinStore.shared.takeGalleryScrollTarget() { proxy.scrollTo(card, anchor: .center) }
                }
                .onChange(of: scrollTarget) { _, id in
                    guard let id else { return }
                    scrollTarget = nil
                    scrollTask?.cancel()
                    scrollTask = Task {
                        // Let the chosen section's content replace the old one first.
                        try? await Task.sleep(nanoseconds: 150_000_000)
                        guard !Task.isCancelled else { return }
                        withAnimation { proxy.scrollTo(id, anchor: .top) }
                        searchState.flash(id)
                        // Heavy sections can still be laying out; one more nudge, unless a newer pick replaced this one.
                        try? await Task.sleep(nanoseconds: 350_000_000)
                        guard !Task.isCancelled else { return }
                        proxy.scrollTo(id, anchor: .top)
                    }
                }
            }
        }
        .environment(\.settingsSearchState, searchState)
        .frame(minWidth: 640, minHeight: 480)
        .onAppear(perform: showPendingSection)
        .onChange(of: appState.pendingSettingsSection) { _, _ in showPendingSection() }
    }

    @ViewBuilder
    private var sidebar: some View {
        if isSearching {
            searchResultsList
        } else {
            sectionList
        }
    }

    private var sectionList: some View {
        List(SettingsSection.visibleCases, selection: $selection) { section in
            Label {
                Text(section.title)
                    .foregroundStyle(Color.whimbrelTextPrimary)
            } icon: {
                Image(systemName: section.systemImage)
                    .symbolRenderingMode(.monochrome)
                    .foregroundStyle(selection == section ? Color.whimbrelTextPrimary : Color.whimbrelTextSecondary)
            }
            .badge(section == .profile ? appState.profileSuggestions.pending.count : 0)
            .tag(section)
            .listRowBackground(
                RoundedRectangle(cornerRadius: Radius.control)
                    .fill(selection == section ? Color.whimbrelSelection : Color.clear)
                    .padding(.horizontal, Spacing.xs)
            )
        }
        .listStyle(.sidebar)
        .tint(Color.whimbrelAccentPrimary)
    }

    /// Results replace the section list while the query is non-empty.
    @ViewBuilder
    private var searchResultsList: some View {
        let results = SettingsSearch.results(for: query)
        if results.isEmpty {
            Text("No matching settings")
                .foregroundStyle(Color.whimbrelTextSecondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .padding(.top, Spacing.lg)
        } else {
            List(results, id: \.id) { entry in
                Button { choose(entry) } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(entry.title).foregroundStyle(Color.whimbrelTextPrimary)
                        Text(entry.section.title)
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .listStyle(.sidebar)
        }
    }

    /// Opens the entry's section, clears the search and scrolls to the row once it is on screen.
    private func choose(_ entry: SettingsSearchEntry) {
        selection = entry.section
        query = ""
        scrollTarget = entry.id
    }

    /// Lands on the section a button elsewhere asked for (`OpenAIProvidersSettingsButton`).
    private func showPendingSection() {
        guard let section = appState.pendingSettingsSection else { return }
        appState.pendingSettingsSection = nil
        query = ""
        selection = section
    }

    @ViewBuilder
    private var content: some View {
        switch selection {
        case .general:
            card { GeneralSettingsPane(openAISettings: { selection = .ai }) }
            card {
                RedesignSwitch(enabled: appState.redesignEnabled) {
                    PermissionChecklistRedesignView()
                } old: {
                    OnboardingView()
                }
                .environmentObject(appState.permissions)
                .settingsAnchor("general.permissions")
            }
        case .profile:
            card { ProfileSettingsPane() }
        case .plan:
            card { PlanSettingsPane(openAISettings: { selection = .ai }, openDevices: { selection = .devices }) }
        case .capture:
            card { YourDataSettingsPane() }
            // X4: shown once the launch opened the store (Task 11b-2 sets `live`).
            if let live = EncryptionSettingsPane.live {
                card { EncryptionSettingsCard(live: live) }
            }
            card { CaptureSettingsPane() }
            card { RetentionSettingsPane() }
        case .meetings:
            card { MeetingsSettingsPane() }
            card { TranscriptionSettingsPane() }
        case .ai:
            card { AIProvidersSettingsPane() }
            card { PromptsSettingsPane() }
        case .topics:
            card { TopicsSettingsPane() }
        case .devices:
            card { DevicesSettingsPane() }
        case .connectors:
            card { ConnectorsSettingsPane() }
            card { MCPClientsSettingsPane() }
        case .automations:
            card { RoutinesSettingsPane() }
        case .diagnostics:
            card { DiagnosticsSettingsPane() }
            #if DEBUG
            card { DesignSystemGalleryLauncher() }
            #endif
        case .none:
            Text("Select a section").foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        content()
            .frame(maxWidth: .infinity, alignment: .leading)
            .whimbrelCard()
    }
}

/// "Open AI Providers settings": shown next to a message whose fix is there (a spent monthly budget).
struct OpenAIProvidersSettingsButton: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        Button("Open AI Providers settings") {
            appState.pendingSettingsSection = .ai
            openSettings()
        }
        .buttonStyle(.whimbrelSecondary)
        .controlSize(.small)
    }
}
