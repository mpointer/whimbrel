import SwiftUI

/// The sections of the Settings window (⌘,). The window is `SettingsRedesignView`. Configuration only: live status and
/// one-shot actions live in the menu bar. The routines inbox and pending approvals live in the Automations tab, not here.
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
