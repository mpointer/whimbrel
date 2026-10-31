import SwiftUI

struct GeneralSettingsPane: View {
    @EnvironmentObject var appState: AppState
    var openAISettings: () -> Void = {}
    @State private var restartBlockedByRecording = false
    @State private var languageOverride = AppLanguage.override ?? ""
    @State private var languageChanged = false
    @AppStorage(AppLanguage.writeInKey) private var writeIn = ""

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("General").font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)

            LabeledContent("App version") {
                Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown")
            }
            LabeledContent("Core version") {
                Text(CoreMemoryStore.coreVersion)
            }
            updateRow

            Divider().overlay(Color.whimbrelBorder)
            languageRows

            Divider().overlay(Color.whimbrelBorder)
            AppearanceThemeSettings(appearance: appState.appearanceSettings, skinChat: appState.skinChat, openAISettings: openAISettings)
        }
    }

    private func languageName(_ tag: String) -> String {
        Locale.current.localizedString(forIdentifier: tag) ?? tag
    }

    private var languageRows: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Picker("Language", selection: $languageOverride) {
                Text("Automatic").tag("")
                ForEach(AppLanguage.shipped, id: \.self) { Text(verbatim: languageName($0)).tag($0) }
            }
            .onChange(of: languageOverride) { _, tag in
                AppLanguage.setOverride(tag.isEmpty ? nil : tag)
                languageChanged = true
            }
            if languageChanged {
                Text("Quit and reopen Whimbrel to use the new language.")
                    .font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            Picker("Write notes and summaries in", selection: $writeIn) {
                Text("Same as app language").tag("")
                ForEach(AppLanguage.writeInOptions, id: \.self) { Text(verbatim: languageName($0)).tag($0) }
            }
            Text("Routines run on your primary device and write in that device's language.")
                .font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
            if let weak = localModelNote {
                Text(weak).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    /// Shown when notes are written in a language other than English by a local model.
    private var localModelNote: String? {
        let tag = AppLanguage.outputTag(uiTag: AppLanguage.uiTag(), writeIn: writeIn,
                                        multilingual: AppLanguage.multilingual())
        guard !tag.isEmpty, !tag.hasPrefix("en"), appState.routing.preferredProvider == .ollama else { return nil }
        return String(localized: "Local models may write less well in \(languageName(tag)).")
    }

    private var updateStatus: UpdateChecker.UpdateStatus { appState.updateChecker.status }

    private var updateBusy: Bool {
        switch updateStatus {
        case .checking, .downloading: return true
        default: return false
        }
    }

    private var updateRow: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack {
                Button("Check for Updates") { Task { await appState.updateChecker.checkNow() } }
                    .disabled(updateBusy)
                if let line = statusLine {
                    Text(line).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
                }
                // Keyed on the staged update, not `.ready`: a later failed check must not hide it.
                if !updateBusy, let update = appState.updateChecker.downloadedUpdate {
                    Button("Restart to Update") {
                        switch appState.restartIntoUpdate(update) {
                        case .recording: restartBlockedByRecording = true
                        // The staged copy was cleaned up; fetch it again.
                        case .missing:
                            appState.updateChecker.discardMissingStagedUpdate()
                            Task { await appState.updateChecker.checkNow() }
                        case nil: break
                        }
                    }
                }
            }
            if restartBlockedByRecording {
                Text("Finish or stop the current recording before restarting to install the update.")
                    .font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private var statusLine: String? {
        switch updateStatus {
        case .idle: return nil
        case .checking: return "Checking…"
        case .upToDate(let v): return "Whimbrel is up to date (v\(v))."
        case .downloading(let v): return "Downloading v\(v)…"
        case .ready(let v): return "v\(v) is ready."
        case .failed: return "Couldn't check for updates. Try again later."
        }
    }
}
