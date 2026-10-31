import Foundation

/// FR-ONB-1: staged TCC permission onboarding.
enum PermissionKind: String, CaseIterable, Identifiable {
    case screenRecording
    case microphone
    case accessibility
    case calendar
    case notifications

    var id: String { rawValue }

    var title: String {
        switch self {
        case .screenRecording: return String(localized: "Screen & System Audio Recording")
        case .microphone: return String(localized: "Microphone")
        case .accessibility: return String(localized: "Accessibility")
        case .calendar: return String(localized: "Calendar")
        case .notifications: return String(localized: "Notifications")
        }
    }

    var explanation: String {
        switch self {
        case .screenRecording:
            return String(localized: "Lets Whimbrel capture your meetings' system audio (what you hear) — no bot joins the call, and nothing else needs to be installed.")
        case .microphone:
            return String(localized: "Lets Whimbrel capture your voice during meetings, on-device, alongside system audio.")
        case .accessibility:
            return String(localized: "Lets Whimbrel read the text of your active window to build a searchable timeline — text only, no screenshots.")
        case .calendar:
            return String(localized: "Lets Whimbrel see events with a conferencing link or other attendees, so it can prompt you to record a meeting shortly before it starts (FR-MTG-1).")
        case .notifications:
            return String(localized: "Lets Whimbrel alert you when a detected meeting is about to start, even if the app isn't in front.")
        }
    }

    var systemSettingsURL: URL {
        switch self {
        case .screenRecording:
            return URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!
        case .microphone:
            return URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!
        case .accessibility:
            return URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!
        case .calendar:
            return URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars")!
        case .notifications:
            return URL(string: "x-apple.systempreferences:com.apple.preference.notifications")!
        }
    }
}
