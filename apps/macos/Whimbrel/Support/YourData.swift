import Foundation
import AppKit

/// The Your data pane's facts and actions (trust surface spec,
/// docs/superpowers/specs/2026-11-04-trust-surface-design.md). The pure parts live here so
/// the test target can check them without an `AppState`.
enum YourData {
    /// One row of "What leaves this computer": a channel, when it sends, what the other
    /// side can see, and this Mac's current state for it.
    struct Outbound: Equatable, Identifiable {
        enum Channel: String, CaseIterable { case providers, syncRelay, updateCheck, google, typesafe }
        let channel: Channel
        var id: Channel { channel }
        let title: String
        let when: String
        let sees: String
        let state: String
        /// True when something is being sent on this channel today.
        let active: Bool
    }

    /// The inputs the rows are built from; the pane fills them from `AppState`.
    struct Inputs: Equatable {
        var hostedProviders: [String]
        var localOnly: Bool
        var syncOn: Bool
        var syncDevices: Int
        var googleAccounts: [String]
        var typesafeOn: Bool
    }

    static func outbound(_ i: Inputs) -> [Outbound] {
        let providers = Outbound(
            channel: .providers,
            title: String(localized: "AI providers"),
            when: String(localized: "When you ask a question, summarize a meeting, or run a routine with a hosted model."),
            sees: String(localized: "The provider whose key you added receives the text needed for that request, under its own terms. Known secrets are redacted first."),
            state: i.localOnly
                ? String(localized: "Local-only mode: nothing is sent.")
                : i.hostedProviders.isEmpty
                    ? String(localized: "No hosted keys. Only local models run.")
                    : String(localized: "Keys for \(i.hostedProviders.joined(separator: ", ")).") ,
            active: !i.localOnly && !i.hostedProviders.isEmpty
        )
        let relay = Outbound(
            channel: .syncRelay,
            title: String(localized: "Sync relay"),
            when: String(localized: "Continuously, while sync is on."),
            sees: String(localized: "The relay stores encrypted data it cannot read. It can see your account email and identifier, each device's name, platform, public key and enrollment dates, the type and time of each synced item, and the names of synced settings fields. Never content, never keys."),
            state: i.syncOn ? String(localized: "On, \(i.syncDevices) device(s).") : String(localized: "Off."),
            active: i.syncOn
        )
        let update = Outbound(
            channel: .updateCheck,
            title: String(localized: "Update check"),
            when: String(localized: "Once a week."),
            sees: String(localized: "A random identifier that changes every week, the platform and the app version, so we can count installs. It is not linked to you, your account or your devices."),
            state: String(localized: "Always on. It carries nothing about you."),
            active: true
        )
        let google = Outbound(
            channel: .google,
            title: String(localized: "Google"),
            when: String(localized: "When a Google account is connected."),
            sees: String(localized: "Google sees the app reading your Gmail and Calendar. The data lands in this Mac's database; we never receive it."),
            state: i.googleAccounts.isEmpty ? String(localized: "No account connected.") : i.googleAccounts.joined(separator: ", "),
            active: !i.googleAccounts.isEmpty
        )
        let typesafe = Outbound(
            channel: .typesafe,
            title: String(localized: "TypeSafe reply ranking"),
            when: String(localized: "When it is on, as email arrives."),
            sees: String(localized: "TypeSafe receives email subjects and snippets to score which threads need a reply."),
            state: i.typesafeOn ? String(localized: "On.") : String(localized: "Off (the default)."),
            active: i.typesafeOn
        )
        return [providers, relay, update, google, typesafe]
    }

    /// The AI providers row's spend line (the AI pane's last-30-day ledger), largest first;
    /// nil until a hosted call has been metered.
    static func spendLine(_ totals: CoreUsageTotals?) -> String? {
        guard let totals, totals.last30DaysCents > 0 else { return nil }
        let dollars = { (cents: Int64) in String(format: "$%.2f", Double(cents) / 100) }
        let names = ["anthropic": "Anthropic", "openai": "OpenAI", "google": "Google", "openrouter": "OpenRouter"]
        let parts = totals.last30DaysByProvider.filter { $0.cents > 0 }.sorted { $0.cents > $1.cents }
            .map { "\(names[$0.provider] ?? $0.provider) \(dollars($0.cents))" }
        return String(localized: "Last 30 days: \(dollars(totals.last30DaysCents)) in hosted model calls (\(parts.joined(separator: ", "))).")
    }

    /// Bytes under `url`, following no symlinks; 0 for a missing folder.
    static func folderBytes(_ url: URL, fileManager fm: FileManager = .default) -> Int64 {
        guard let enumerator = fm.enumerator(at: url, includingPropertiesForKeys: [.totalFileAllocatedSizeKey, .isRegularFileKey], options: [.skipsHiddenFiles]) else { return 0 }
        var total: Int64 = 0
        for case let file as URL in enumerator {
            guard let values = try? file.resourceValues(forKeys: [.totalFileAllocatedSizeKey, .isRegularFileKey]),
                  values.isRegularFile == true else { continue }
            total += Int64(values.totalFileAllocatedSize ?? 0)
        }
        return total
    }

    static func formatBytes(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    /// Everything "Delete everything on this computer" removes: every entry directly under
    /// the data folder except the ones in `keep` (the downloaded models, which are not the
    /// user's data and cost hundreds of megabytes to fetch again). Sorted, for a stable
    /// list in the confirmation sheet.
    static func entriesToDelete(in root: URL, keep: Set<String> = ["models"], fileManager fm: FileManager = .default) -> [URL] {
        let entries = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil, options: [])) ?? []
        return entries
            .filter { !keep.contains($0.lastPathComponent) }
            .sorted { $0.lastPathComponent.localizedCaseInsensitiveCompare($1.lastPathComponent) == .orderedAscending }
    }

    /// This Mac's recordings and slide images in the media folder (default `~/Pictures/Whimbrel`),
    /// which is outside the data folder: its own `Audio/<device>` folder and every
    /// `Snapshots/` meeting folder Whimbrel made (the ones holding `SnapshotStorage.markerName`).
    /// Another Mac's `Audio/<device>` in a shared media folder is never listed.
    // ponytail: snapshot folders carry no device id, so in a media folder shared by two Macs
    // the other Mac's slide folders are listed too; the sheet names the folder.
    static func mediaEntriesToDelete(audio: URL, snapshots: URL, fileManager fm: FileManager = .default) -> [URL] {
        let meetingFolders = ((try? fm.contentsOfDirectory(at: snapshots, includingPropertiesForKeys: nil, options: [])) ?? [])
            .filter { fm.fileExists(atPath: $0.appendingPathComponent(SnapshotStorage.markerName).path) }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        return (fm.fileExists(atPath: audio.path) ? [audio] : []) + meetingFolders
    }

    /// Removes the entries; returns the ones that could not be removed.
    @discardableResult
    static func delete(_ entries: [URL], fileManager fm: FileManager = .default) -> [URL] {
        entries.filter { (try? fm.removeItem(at: $0)) == nil && fm.fileExists(atPath: $0.path) }
    }

    /// The note "Delete everything" leaves in the data folder: the paths to remove and the
    /// process that must quit first. The removal itself runs at the next launch, before the
    /// database opens, because removing an open SQLite file under a running app crashes it
    /// or lets it write a half-empty database back.
    static let pendingDeleteName = ".delete-on-launch.json"

    private struct PendingDelete: Codable { let pid: Int32; let paths: [String] }

    static func scheduleDelete(_ entries: [URL], in root: URL, pid: Int32 = ProcessInfo.processInfo.processIdentifier) throws {
        let data = try JSONEncoder().encode(PendingDelete(pid: pid, paths: entries.map(\.path)))
        try data.write(to: root.appendingPathComponent(pendingDeleteName), options: .atomic)
    }

    /// Runs a scheduled "Delete everything", once the instance that scheduled it has quit.
    /// Returns nil when nothing is pending, else the entries that could not be removed. The
    /// note is removed whatever happens, so a later launch never deletes data made after
    /// this one: if the old instance is still running after `timeout` (or the note can't be
    /// read), nothing is removed and every entry comes back as not removed.
    @discardableResult
    static func runPendingDelete(in root: URL, timeout: TimeInterval = 10,
                                 isRunning: (Int32) -> Bool = { kill($0, 0) == 0 },
                                 fileManager fm: FileManager = .default) -> [URL]? {
        let note = root.appendingPathComponent(pendingDeleteName)
        guard let data = fm.contents(atPath: note.path) else { return nil }
        defer { try? fm.removeItem(at: note) }
        guard let pending = try? JSONDecoder().decode(PendingDelete.self, from: data) else { return [note] }
        let entries = pending.paths.map { URL(fileURLWithPath: $0) }
        let deadline = Date().addingTimeInterval(timeout)
        while isRunning(pending.pid) {
            if Date() >= deadline { return entries }
            Thread.sleep(forTimeInterval: 0.1)
        }
        return delete(entries, fileManager: fm)
    }

    /// UserDefaults key holding what the last "Delete everything" could not remove, one path
    /// per line; the Your data pane shows it until dismissed.
    static let deleteLeftoversKey = "yourData.deleteLeftovers"

    /// Starts a fresh instance of this app and quits this one.
    static func relaunch() {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: configuration) { _, _ in
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }
}
