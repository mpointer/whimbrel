import AVFoundation
import EventKit
import SwiftUI
import WhimbrelUI

/// What the permission screens show for each kind, as pure functions of the kind and its state.
enum PermissionRedesign {
    static let required: [PermissionKind] = PermissionKind.firstRun
    static let optional: [PermissionKind] = PermissionKind.allCases.filter { !PermissionKind.firstRun.contains($0) }

    static func glyph(_ kind: PermissionKind) -> String {
        switch kind {
        case .screenRecording: return "display"
        case .microphone: return "mic.fill"
        case .accessibility: return "accessibility"
        case .calendar: return "calendar"
        case .notifications: return "bell.fill"
        }
    }

    /// One line on why Whimbrel asks. The old wording stays where it had no dash and no developer term.
    static func reason(_ kind: PermissionKind) -> String {
        switch kind {
        case .screenRecording:
            return String(localized: "Lets Whimbrel record the sound your Mac plays, so it can hear the other people in a meeting. No bot joins the call.")
        case .microphone:
            return kind.explanation
        case .accessibility:
            return String(localized: "Lets Whimbrel read the text of your active window to build a searchable timeline. Text only, no screenshots.")
        case .calendar:
            return String(localized: "Lets Whimbrel see events that have a meeting link or other people, so it can offer to record shortly before they start.")
        case .notifications:
            return kind.explanation
        }
    }

    /// Granted is allowed. Otherwise it is denied when macOS says so or when this screen already asked
    /// (the kinds macOS cannot report on), and not yet when neither is true.
    static func state(granted: Bool, systemDenied: Bool, askedHere: Bool) -> WPermissionState {
        if granted { return .allowed }
        return systemDenied || askedHere ? .denied : .notYet
    }

    /// macOS's own answer for the kinds that have one (microphone, calendar).
    static func systemDenied(_ kind: PermissionKind) -> Bool {
        switch kind {
        case .microphone:
            let status = AVCaptureDevice.authorizationStatus(for: .audio)
            return status == .denied || status == .restricted
        case .calendar:
            let status = EKEventStore.authorizationStatus(for: .event)
            return status == .denied || status == .restricted
        default:
            return false
        }
    }

    /// Whether pressing Allow on this screen counts as having asked: only for the kinds macOS cannot report on.
    static func asksTrackedHere(_ kind: PermissionKind) -> Bool {
        switch kind {
        case .microphone, .calendar: return false
        default: return true
        }
    }

    @MainActor
    static func rows(_ kinds: [PermissionKind], permissions: PermissionsManager, requested: Set<PermissionKind>, didAsk: @escaping (PermissionKind) -> Void) -> [WPermissionRowModel] {
        kinds.map { kind in
            WPermissionRowModel(
                id: kind.rawValue, glyph: glyph(kind), name: kind.title, reason: reason(kind),
                state: state(granted: permissions.granted(for: kind), systemDenied: systemDenied(kind), askedHere: requested.contains(kind)),
                onAllow: {
                    permissions.request(kind)
                    if asksTrackedHere(kind) { didAsk(kind) }
                },
                onOpenSettings: { permissions.openSystemSettings(for: kind) })
        }
    }

    static var requiredHeading: String { String(localized: "Required", comment: "Heading of the permissions Whimbrel needs to record a meeting.") }
    static var optionalHeading: String { String(localized: "Optional", comment: "Heading of the permissions that add extra features.") }
}
