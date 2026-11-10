import SwiftUI
import AppKit

/// UserDefaults keys for the Obsidian vault the topic scheduler keeps up to date.
enum VaultSettings {
    static let enabledKey = "vault.enabled"
    static let pathKey = "vault.path"
    static let lastUpdatedKey = "vault.lastUpdated"
    static let failedKey = "vault.failed"
}

/// Settings → Capture & Privacy → Your data: what is on this Mac, what leaves it, and the
/// two actions that take it all or remove it all (trust surface spec,
/// docs/superpowers/specs/2026-11-04-trust-surface-design.md).
struct YourDataSettingsPane: View {
    @EnvironmentObject var appState: AppState
    @State private var counts: CoreDataCounts?
    @State private var dataFolderBytes: Int64 = 0
    @State private var mediaBytes: Int64 = 0
    @State private var usageTotals: CoreUsageTotals?
    /// Core's provider table, for Whimbrel AI's data sentence (the AI Providers card's).
    @State private var providerRows: [CoreProviderInfo] = CoreMemoryStore.providers()
    @State private var alsoDeleteSyncAccount = false
    @State private var deleting = false
    @State private var exportStatus: String?
    @State private var exportFolder: URL?
    @State private var exporting = false
    @State private var confirmingDelete = false
    @State private var deleteTyped = ""
    @State private var deleteFailure: String?
    @AppStorage(YourData.deleteLeftoversKey) private var deleteLeftovers = ""
    @AppStorage(VaultSettings.enabledKey) private var vaultEnabled = false
    @AppStorage(VaultSettings.pathKey) private var vaultPath = ""
    @AppStorage(VaultSettings.lastUpdatedKey) private var vaultLastUpdated = 0.0
    @AppStorage(VaultSettings.failedKey) private var vaultFailed = false

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Your data").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("What Whimbrel keeps on this Mac, what leaves it, and how to take it with you or remove it.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)

            onThisMac
            Divider().overlay(Color.whimbrelBorder)
            leaves
            Divider().overlay(Color.whimbrelBorder)
            actions
        }
        .onAppear(perform: refresh)
        .sheet(isPresented: $confirmingDelete) { deleteSheet }
    }

    // MARK: On this Mac

    private var onThisMac: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("On this Mac").font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            if let counts {
                LazyVGrid(columns: [GridItem(.flexible(), alignment: .leading), GridItem(.flexible(), alignment: .leading)], alignment: .leading, spacing: Spacing.xs) {
                    countRow(String(localized: "Meetings"), counts.meetings)
                    countRow(String(localized: "Transcript lines"), counts.transcriptLines)
                    countRow(String(localized: "Screen captures"), counts.captures)
                    countRow(String(localized: "Slides"), counts.snapshots)
                    countRow(String(localized: "Emails"), counts.emails)
                    countRow(String(localized: "Calendar events"), counts.calendarEvents)
                    countRow(String(localized: "Email threads"), counts.threads)
                    countRow(String(localized: "Activity episodes"), counts.episodes)
                    countRow(String(localized: "Conversations"), counts.conversations)
                    countRow(String(localized: "Routine runs"), counts.routineRuns)
                    countRow(String(localized: "Topics"), counts.topics)
                    countRow(String(localized: "People and organizations"), counts.entities)
                }
            } else {
                Text("Counting…").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            HStack {
                Text("Data folder: \(YourData.formatBytes(dataFolderBytes))")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Spacer()
                Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([SpikePaths.root]) }
                    .buttonStyle(.whimbrelSecondary)
            }
            Text("Recordings and slide images: \(appState.mediaSettings.folderURL.path), \(YourData.formatBytes(mediaBytes))")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    private func countRow(_ label: String, _ value: Int64) -> some View {
        HStack {
            Text(label).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            Spacer(minLength: Spacing.sm)
            Text(value, format: .number).font(.whimbrelBody(.caption)).monospacedDigit()
        }
    }

    // MARK: What leaves

    private var leaves: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("What leaves this Mac").font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
            ForEach(YourData.outbound(inputs)) { row in
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: Spacing.xs) {
                        Circle().fill(row.active ? Color.whimbrelAccentPrimary : Color.whimbrelBorder).frame(width: 6, height: 6)
                        Text(row.title).font(.whimbrelBody(.body))
                        Spacer()
                        Text(row.state).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    Text(row.when).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    Text(row.sees).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if row.channel == .providers, let spend = YourData.spendLine(usageTotals) {
                        Text(spend).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            Link("Read the full list on the privacy page", destination: URL(string: "https://whimbrelapp.com/privacy/#leaves")!)
                .font(.whimbrelBody(.caption))
        }
    }

    private var inputs: YourData.Inputs {
        var providers: [String] = []
        if appState.routing.hasKey(for: .anthropic) { providers.append("Anthropic") }
        if appState.routing.hasKey(for: .openai) { providers.append("OpenAI") }
        if appState.routing.hasKey(for: .google) { providers.append("Google") }
        if appState.routing.hasKey(for: .openrouter) { providers.append("OpenRouter") }
        // Whimbrel AI counts once it is the chosen provider and signed in (core adds it to
        // a chain only then); its row's data sentence then joins "sees".
        let whimbrelAI = appState.routing.preferredProvider == .whimbrel && appState.routing.hasKey(for: .whimbrel)
        if whimbrelAI { providers.append("Whimbrel AI") }
        var syncOn = false
        if case .ready = appState.syncState.status { syncOn = true }
        let accounts = Set(appState.gmailConnectorState.accounts.map(\.email) + appState.calendarConnectorState.accounts.map(\.email)).sorted()
        return YourData.Inputs(
            hostedProviders: providers,
            localOnly: appState.routing.localOnlyMode,
            syncOn: syncOn,
            syncDevices: appState.syncState.devices.count,
            googleAccounts: accounts,
            typesafeOn: KeychainService.load(for: .typesafe)?.isEmpty == false,
            whimbrelAITerms: whimbrelAI ? providerRows.first(where: { $0.id == "whimbrel" })?.dataTerms?.localizedText : nil
        )
    }

    // MARK: Actions

    private var actions: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            vaultSection
            Divider().overlay(Color.whimbrelBorder)
            HStack {
                Button(exporting ? "Exporting…" : "Export everything…") { exportEverything() }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(exporting)
                if let exportFolder {
                    Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([exportFolder]) }
                        .buttonStyle(.whimbrelSecondary)
                }
            }
            Text("Meetings with their transcripts as Markdown, screen text, email, calendar, conversations and routines as JSON, and topics as Obsidian notes. Readable without Whimbrel. Recordings and slide images are not exported.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let exportStatus {
                Text(exportStatus).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }

            if !deleteLeftovers.isEmpty {
                Text("Delete everything could not remove these. Quit Whimbrel and remove them by hand:\n\(deleteLeftovers)")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Dismiss") { deleteLeftovers = "" }
                    .buttonStyle(.whimbrelSecondary)
            }
            Button("Delete everything on this Mac…") {
                deleteTyped = ""
                deleteFailure = nil
                alsoDeleteSyncAccount = false
                confirmingDelete = true
                // A fresh balance for the forfeit line if the sync account goes too.
                Task { await appState.syncState.refreshAIBalance() }
            }
            .buttonStyle(.whimbrelSecondary)
            .tint(Color.whimbrelError)
        }
    }

    private var vaultSection: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Toggle("Keep an Obsidian vault up to date", isOn: $vaultEnabled)
            HStack {
                Button("Choose folder…") { chooseVaultFolder() }
                    .buttonStyle(.whimbrelSecondary)
                if !vaultPath.isEmpty {
                    Text(vaultPath)
                        .font(.whimbrelBody(.caption))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            if vaultEnabled, !vaultPath.isEmpty, vaultLastUpdated > 0 {
                let when = RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: vaultLastUpdated), relativeTo: Date())
                Text("Last updated \(when)")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            }
            if vaultEnabled, !vaultPath.isEmpty, vaultFailed {
                Text("Couldn't update the vault. Check that the folder exists and Whimbrel can write to it.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func chooseVaultFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        // A new folder has not been written yet: no "Last updated" from the old one.
        if url.path != vaultPath { vaultLastUpdated = 0 }
        vaultPath = url.path
        vaultFailed = false
    }

    private func refresh() {
        guard let store = appState.coreStore else { return }
        let media = mediaEntries()
        CoreMemoryStore.accessQueue.async {
            let c = store.dataCounts()
            let totals = store.usageTotals()
            let bytes = YourData.folderBytes(SpikePaths.root)
            let mBytes = media.reduce(Int64(0)) { $0 + YourData.folderBytes($1) }
            DispatchQueue.main.async {
                counts = c
                usageTotals = totals
                dataFolderBytes = bytes
                mediaBytes = mBytes
            }
        }
    }

    private func exportEverything() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Export here")
        guard panel.runModal() == .OK, let root = panel.url, let store = appState.coreStore else { return }
        exporting = true
        exportStatus = nil
        CoreMemoryStore.accessQueue.async {
            let report = store.exportAll(root: root)
            DispatchQueue.main.async {
                exporting = false
                if let report {
                    exportFolder = URL(fileURLWithPath: report.folder)
                    exportStatus = String(localized: "Exported \(report.meetings) meeting(s), \(report.captures) screen capture(s), \(report.emails) email(s) and \(report.conversations) conversation(s): \(report.files) files, \(YourData.formatBytes(report.bytes)).")
                } else {
                    exportFolder = nil
                    exportStatus = String(localized: "The export failed. Check that the folder can be written to and try again.")
                }
            }
        }
    }

    // MARK: Delete everything

    private var deleteSheet: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Delete everything on this Mac?").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("This removes the database and the data folder (\(YourData.formatBytes(dataFolderBytes))), and this Mac's recordings and slide images in \(appState.mediaSettings.folderURL.path) (\(YourData.formatBytes(mediaBytes))), then restarts Whimbrel as new. Settings, keys and downloaded models stay.")
                .font(.whimbrelBody(.body))
                .fixedSize(horizontal: false, vertical: true)
            if case .ready = appState.syncState.status {
                Toggle("Also delete my sync account", isOn: $alsoDeleteSyncAccount)
                Text(alsoDeleteSyncAccount
                     ? [String(localized: "The encrypted copy on the sync server is deleted and every device is signed out of sync. Your other devices keep their own data."),
                        appState.syncState.aiCreditForfeitNote].compactMap { $0 }.joined(separator: " ")
                     : String(localized: "Sync is on. This Mac will download everything again from your other devices unless you also delete your sync account."))
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(alsoDeleteSyncAccount ? Color.whimbrelTextSecondary : Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text("Type DELETE to confirm.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            TextField("DELETE", text: $deleteTyped).textFieldStyle(.roundedBorder)
            if let deleteFailure {
                Text(deleteFailure).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
            HStack {
                Spacer()
                Button("Cancel") { confirmingDelete = false }.buttonStyle(.whimbrelSecondary)
                Button("Delete and restart") { Task { await deleteEverything() } }
                    .buttonStyle(.whimbrelSecondary)
                    .tint(Color.whimbrelError)
                    .disabled(deleteTyped != "DELETE" || deleting)
            }
        }
        .padding(Spacing.lg)
        .frame(width: 480)
    }

    private func mediaEntries() -> [URL] {
        YourData.mediaEntriesToDelete(audio: appState.mediaSettings.audioURL(deviceId: appState.deviceId),
                                      snapshots: appState.mediaSettings.snapshotsURL)
    }

    private func deleteEverything() async {
        deleting = true
        defer { deleting = false }
        // The account goes first: if the server refuses, nothing on this Mac is touched yet.
        if alsoDeleteSyncAccount, case .ready = appState.syncState.status {
            await appState.syncState.deleteSyncAccount()
            if case .ready = appState.syncState.status {
                deleteFailure = appState.syncState.actionError
                    ?? String(localized: "The sync account could not be deleted. Nothing was removed; try again.")
                return
            }
        }
        if appState.activityCaptureEnabled { appState.toggleActivityCapture() }
        appState.endActiveMeetingIfNeeded()
        let entries = YourData.entriesToDelete(in: SpikePaths.root) + mediaEntries()
        do {
            try YourData.scheduleDelete(entries, in: SpikePaths.root)
            YourData.relaunch()
        } catch {
            deleteFailure = String(localized: "Could not start the deletion: \(error.localizedDescription). Quit Whimbrel and remove the data folder by hand.")
        }
    }
}
