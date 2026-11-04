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

    /// Sites whose pages are never captured (trust surface spec,
    /// docs/superpowers/specs/2026-11-04-trust-surface-design.md): lowercase hosts,
    /// matched against the window's URL host and its subdomains, checked after the
    /// Accessibility walk found the URL and before anything is written. Synced with the
    /// bundle ids.
    private static let domainsDefaultsKey = "com.mpointer.kestrel.excludedDomains"
    static let domainsFieldKey = "captureExclusions.excludedDomains"

    @Published private(set) var excludedDomains: Set<String> {
        didSet {
            UserDefaults.standard.set(Array(excludedDomains), forKey: Self.domainsDefaultsKey)
            SettingsFieldTimestamps.touch(Self.domainsFieldKey)
        }
    }

    private init() {
        let stored = UserDefaults.standard.stringArray(forKey: Self.defaultsKey) ?? []
        excludedBundleIds = Set(stored)
        let domains = UserDefaults.standard.stringArray(forKey: Self.domainsDefaultsKey) ?? []
        excludedDomains = Set(domains.compactMap(Self.normalizedDomain))
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

    /// True when `url`'s host is a listed domain or a subdomain of one. A URL with no
    /// host (a file path, a custom scheme) is never excluded by this list.
    func isExcluded(url: String) -> Bool {
        guard let host = Self.host(of: url) else { return false }
        return excludedDomains.contains { Self.host(host, matches: $0) }
    }

    /// Adds a domain typed by the user; returns false for input that is not a host.
    @discardableResult
    func addExcludedDomain(_ raw: String) -> Bool {
        guard let domain = Self.normalizedDomain(raw) else { return false }
        excludedDomains.insert(domain)
        return true
    }

    func removeExcludedDomain(_ domain: String) {
        excludedDomains.remove(domain)
    }

    /// Replaces the list wholesale (settings sync).
    func setExcludedDomains(_ domains: [String]) {
        let normalized = Set(domains.compactMap(Self.normalizedDomain))
        if normalized != excludedDomains {
            excludedDomains = normalized
        }
    }

    /// "Example.com/path", "https://example.com", " example.com " all become
    /// "example.com"; nothing without a dot or with spaces inside is a domain.
    static func normalizedDomain(_ raw: String) -> String? {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let scheme = s.range(of: "://") { s = String(s[scheme.upperBound...]) }
        if let slash = s.firstIndex(of: "/") { s = String(s[..<slash]) }
        if let at = s.lastIndex(of: "@") { s = String(s[s.index(after: at)...]) }
        if let colon = s.firstIndex(of: ":") { s = String(s[..<colon]) }
        s = s.trimmingCharacters(in: CharacterSet(charactersIn: "."))
        guard !s.isEmpty, s.contains("."), !s.contains(" ") else { return nil }
        return s
    }

    static func host(of url: String) -> String? {
        guard let parsed = URL(string: url), let host = parsed.host?.lowercased(), !host.isEmpty else { return nil }
        return host
    }

    static func host(_ host: String, matches domain: String) -> Bool {
        host == domain || host.hasSuffix("." + domain)
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
