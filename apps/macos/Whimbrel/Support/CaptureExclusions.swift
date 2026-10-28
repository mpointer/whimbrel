import AppKit
import Foundation

/// Privacy Controls-style app exclusion — mirrors the real Littlebird's "Exclude Apps."
/// Excluded apps' screen text is never captured: checked in
/// `ScreenTextCaptureService.captureFrontmostWindow()` before any AX/OCR extraction
/// happens, not filtered out afterward, so excluded content never even gets walked.
///
/// A singleton (not owned/injected via `AppState`) because `ScreenTextCaptureService`
/// needs to read it on every capture and doesn't otherwise hold a reference to sibling
/// services — simpler than threading a dependency through for a small, always-available
/// settings object.
@MainActor
final class CaptureExclusions: ObservableObject {
    static let shared = CaptureExclusions()

    private static let defaultsKey = "com.mpointer.kestrel.excludedBundleIds"

    /// System/lock-screen chrome that's never a legitimate capture target and never
    /// shows up in `candidateApps()` (non-`.regular` activation policy), so the user
    /// has no way to exclude it via the picker. Whimbrel's own windows are excluded too:
    /// capturing them only records Whimbrel showing back what it already stored, and sync
    /// then carries those rows to every other device. Always excluded, not user-configurable.
    private static let alwaysExcludedBundleIds: Set<String> = [
        "com.apple.loginwindow",
        "com.apple.ScreenSaver.Engine",
        "com.mpointer.kestrel",
    ]

    @Published private(set) var excludedBundleIds: Set<String> {
        didSet {
            UserDefaults.standard.set(Array(excludedBundleIds), forKey: Self.defaultsKey)
            SettingsFieldTimestamps.touch("captureExclusions.excludedBundleIds")
        }
    }

    private init() {
        let stored = UserDefaults.standard.stringArray(forKey: Self.defaultsKey) ?? []
        excludedBundleIds = Set(stored)
    }

    func isExcluded(_ bundleId: String) -> Bool {
        Self.alwaysExcludedBundleIds.contains(bundleId) || excludedBundleIds.contains(bundleId)
    }

    func setExcluded(_ excluded: Bool, for bundleId: String) {
        if excluded {
            excludedBundleIds.insert(bundleId)
        } else {
            excludedBundleIds.remove(bundleId)
        }
    }

    /// Currently-running, regular (non-background/menu-bar-only) apps — matches the
    /// real app's per-app toggle list. Lists *running* apps rather than every app
    /// installed on disk: there's no fast/reliable API to enumerate every installed
    /// app without hand-scanning /Applications and ~/Applications, and running apps
    /// cover the common case (you exclude the app you're currently worried about).
    func candidateApps() -> [(bundleId: String, name: String)] {
        NSWorkspace.shared.runningApplications
            .filter { $0.activationPolicy == .regular }
            .compactMap { app in
                guard let bundleId = app.bundleIdentifier, let name = app.localizedName else { return nil }
                return (bundleId: bundleId, name: name)
            }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}
