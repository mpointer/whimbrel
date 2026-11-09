import ApplicationServices
import AVFoundation
import AppKit
import CoreGraphics
import EventKit
import Foundation
import UserNotifications

/// FR-ONB-1/2: requests Screen & System Audio Recording, Microphone, Accessibility,
/// Calendar, and Notifications one at a time, and exposes granted/denied state so the
/// app can run with partial grants.
@MainActor
final class PermissionsManager: ObservableObject {
    @Published private(set) var screenRecordingGranted: Bool = CGPreflightScreenCaptureAccess()
    @Published private(set) var microphoneGranted: Bool = AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
    @Published private(set) var accessibilityGranted: Bool = AXIsProcessTrusted()
    @Published private(set) var calendarGranted: Bool = PermissionsManager.calendarAccessible
    @Published private(set) var notificationsGranted: Bool = false

    private var activationObserver: NSObjectProtocol?
    private var pollTimer: Timer?

    init() {
        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.refresh()
        }
        startPollingIfNeeded()
        refreshNotificationStatus()
        // Granted on an earlier run but reported as not determined (see `calendarAccessible`): ask
        // again. TCC already holds the grant, so this returns at once with no prompt.
        if EKEventStore.authorizationStatus(for: .event) == .notDetermined,
           UserDefaults.standard.bool(forKey: Self.calendarGrantedKey) {
            request(.calendar)
        }
    }

    private static let calendarGrantedKey = "com.mpointer.kestrel.calendarAccessGranted"
    private static var calendarRequestGranted = false

    /// Calendar access the app can use. On macOS 27 `authorizationStatus` has been seen to stay
    /// `.notDetermined` for the whole run while TCC allows full access and `requestFullAccessToEvents()`
    /// returns granted with no prompt, so a granted request this run counts too.
    static var calendarAccessible: Bool {
        let status = EKEventStore.authorizationStatus(for: .event)
        if status == .fullAccess, !UserDefaults.standard.bool(forKey: calendarGrantedKey) {
            UserDefaults.standard.set(true, forKey: calendarGrantedKey)
        }
        return status == .fullAccess || (status == .notDetermined && calendarRequestGranted)
    }

    deinit {
        if let activationObserver {
            NotificationCenter.default.removeObserver(activationObserver)
        }
        pollTimer?.invalidate()
    }

    /// App-activation notifications aren't fully reliable for granting Accessibility in
    /// particular — the grant happens in System Settings, a different app, and switching
    /// back doesn't always fire `didBecomeActiveNotification` promptly (seen in practice
    /// over remote-desktop sessions). Poll while anything is outstanding; stop once
    /// everything's granted so this doesn't cost anything at steady state (NFR-PRF-1).
    private func startPollingIfNeeded() {
        pollTimer?.invalidate()
        guard !allGranted else { return }
        pollTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.refresh()
            }
        }
    }

    var allGranted: Bool {
        screenRecordingGranted && microphoneGranted && accessibilityGranted && calendarGranted && notificationsGranted
    }

    func refresh() {
        screenRecordingGranted = CGPreflightScreenCaptureAccess()
        microphoneGranted = AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
        accessibilityGranted = AXIsProcessTrusted()
        calendarGranted = Self.calendarAccessible
        refreshNotificationStatus()
        if allGranted {
            pollTimer?.invalidate()
            pollTimer = nil
        } else if pollTimer == nil {
            startPollingIfNeeded()
        }
    }

    /// `UNUserNotificationCenter`'s status check is itself async — kicked off
    /// separately from the synchronous checks above so `refresh()` doesn't need to be.
    private func refreshNotificationStatus() {
        Task { @MainActor in
            let settings = await UNUserNotificationCenter.current().notificationSettings()
            notificationsGranted = settings.authorizationStatus == .authorized
        }
    }

    func granted(for kind: PermissionKind) -> Bool {
        switch kind {
        case .screenRecording: return screenRecordingGranted
        case .microphone: return microphoneGranted
        case .accessibility: return accessibilityGranted
        case .calendar: return calendarGranted
        case .notifications: return notificationsGranted
        }
    }

    func request(_ kind: PermissionKind) {
        switch kind {
        case .screenRecording:
            _ = CGRequestScreenCaptureAccess()
            refresh()
        case .microphone:
            AVCaptureDevice.requestAccess(for: .audio) { [weak self] granted in
                DispatchQueue.main.async {
                    self?.microphoneGranted = granted
                }
            }
        case .accessibility:
            let promptKey = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
            let options: NSDictionary = [promptKey: true]
            accessibilityGranted = AXIsProcessTrustedWithOptions(options)
        case .calendar:
            Task { @MainActor in
                let granted = (try? await EKEventStore().requestFullAccessToEvents()) ?? false
                Self.calendarRequestGranted = granted
                if granted { UserDefaults.standard.set(true, forKey: Self.calendarGrantedKey) }
                self.calendarGranted = Self.calendarAccessible
            }
        case .notifications:
            Task { @MainActor in
                let granted = (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])) ?? false
                self.notificationsGranted = granted
            }
        }
    }

    /// Calendar from where it's needed: the system prompt once, then Settings, which is the
    /// only place left once the user has answered.
    func requestCalendarOrOpenSettings(fallback: () -> Void) {
        if EKEventStore.authorizationStatus(for: .event) == .notDetermined { request(.calendar) } else { fallback() }
    }

    func openSystemSettings(for kind: PermissionKind) {
        NSWorkspace.shared.open(kind.systemSettingsURL)
    }
}
