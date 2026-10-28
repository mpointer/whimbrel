import SwiftUI

/// Native macOS Settings scene (⌘,): configuration only. Live status and one-shot
/// actions live in Library's header and the menu-bar popover. Each section is a stack
/// of cards, one per existing pane, so related settings share a page (docs/13 item 9).
/// The routines inbox and pending approvals live in the Automations tab, not here.
enum SettingsSection: String, CaseIterable, Identifiable {
    case general
    case profile
    case capture
    case meetings
    case ai
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
        case .general: return "General"
        case .profile: return "Profile"
        case .capture: return "Capture & Privacy"
        case .meetings: return "Meetings"
        case .ai: return "AI"
        case .devices: return "Sync & Devices"
        case .connectors: return "Connectors"
        case .automations: return "Automations"
        case .diagnostics: return "Diagnostics"
        }
    }

    var systemImage: String {
        switch self {
        case .general: return "gearshape"
        case .profile: return "person.crop.circle"
        case .capture: return "eye.slash"
        case .meetings: return "calendar"
        case .ai: return "cpu"
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

    var body: some View {
        NavigationSplitView {
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
            }
        }
        .frame(minWidth: 640, minHeight: 480)
    }

    @ViewBuilder
    private var content: some View {
        switch selection {
        case .general:
            card { GeneralSettingsPane(openAISettings: { selection = .ai }) }
            card { OnboardingView().environmentObject(appState.permissions) }
        case .profile:
            card { ProfileSettingsPane() }
        case .capture:
            card { CaptureSettingsPane() }
            card { RetentionSettingsPane() }
        case .meetings:
            card { MeetingsSettingsPane() }
            card { TranscriptionSettingsPane() }
        case .ai:
            card { AIProvidersSettingsPane() }
            card { PromptsSettingsPane() }
        case .devices:
            card { DevicesSettingsPane() }
        case .connectors:
            card { ConnectorsSettingsPane() }
            card { MCPClientsSettingsPane() }
        case .automations:
            card { RoutinesSettingsPane() }
        case .diagnostics:
            card { DiagnosticsSettingsPane() }
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
