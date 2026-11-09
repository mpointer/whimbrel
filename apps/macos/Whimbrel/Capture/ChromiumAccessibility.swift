import AppKit
import ApplicationServices

/// Chromium builds its accessibility tree only on request, and doing that costs the browser CPU,
/// so it stays on only around a possible call. See
/// docs/superpowers/specs/2026-11-01-chromium-meeting-detection-design.md.
@MainActor
final class ChromiumAccessibility {
    static let bundleIDs: Set<String> = [
        "com.google.Chrome",
        "com.microsoft.edgemac",
        "company.thebrowser.Browser",
        "com.brave.Browser",
    ]

    private var enabled: [pid_t: String] = [:]

    /// Which processes to change, given this poll's state. `candidate` is a frontmost Chromium
    /// browser, `tracked` the browser holding a tracked meeting.
    static func changes(enabled: Set<pid_t>, candidate: pid_t?, tracked: pid_t?, micInUse: Bool) -> (enable: Set<pid_t>, disable: Set<pid_t>) {
        var wanted: Set<pid_t> = micInUse ? enabled : []
        if micInUse, let candidate { wanted.insert(candidate) }
        if let tracked { wanted.insert(tracked) }
        return (wanted.subtracting(enabled), enabled.subtracting(wanted))
    }

    func update(frontmost: NSRunningApplication?, trackedWindow: AXUIElement?, micInUse: Bool) {
        let candidate = frontmost.flatMap { app in
            app.bundleIdentifier.map(Self.bundleIDs.contains) == true ? app.processIdentifier : nil
        }
        var tracked: pid_t = 0
        let hasTracked = trackedWindow.map { AXUIElementGetPid($0, &tracked) == .success } ?? false
        let change = Self.changes(enabled: Set(enabled.keys), candidate: candidate,
                                  tracked: hasTracked && enabled[tracked] != nil ? tracked : nil, micInUse: micInUse)
        for pid in change.enable { if let attribute = Self.turnOn(pid) { enabled[pid] = attribute } }
        for pid in change.disable { turnOff(pid) }
    }

    func turnOffAll() {
        enabled.keys.forEach(turnOff)
    }

    private func turnOff(_ pid: pid_t) {
        guard let attribute = enabled.removeValue(forKey: pid) else { return }
        _ = AXUIElementSetAttributeValue(AXUIElementCreateApplication(pid), attribute as CFString, kCFBooleanFalse)
    }

    /// The attribute that took, or nil when the browser refused both or there's no permission.
    private static func turnOn(_ pid: pid_t) -> String? {
        let app = AXUIElementCreateApplication(pid)
        return ["AXManualAccessibility", "AXEnhancedUserInterface"].first {
            AXUIElementSetAttributeValue(app, $0 as CFString, kCFBooleanTrue) == .success
        }
    }
}
