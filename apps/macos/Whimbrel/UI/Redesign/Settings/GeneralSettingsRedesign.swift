import SwiftUI
import WhimbrelUI

/// General in the redesigned Settings window: the model builders (pure, tested) and the view that feeds the package's
/// `WGeneralSettingsView` from the app's state and calls the same functions the classic General pane calls.
enum GeneralSettingsAdapter {
    /// The line under Check for Updates. The classic pane's own words (they are plain text there, not catalog keys).
    static func updateStatusLine(_ status: UpdateChecker.UpdateStatus) -> String? {
        switch status {
        case .idle: return nil
        case .checking: return String(localized: "Checking…")
        case .upToDate(let v): return String(localized: "Whimbrel is up to date (v\(v)).")
        case .downloading(let v): return String(localized: "Downloading v\(v)…")
        case .ready(let v): return String(localized: "v\(v) is ready.")
        case .failed: return String(localized: "Couldn't check for updates. Try again later.")
        }
    }

    static func updateBusy(_ status: UpdateChecker.UpdateStatus) -> Bool {
        switch status {
        case .checking, .downloading: return true
        default: return false
        }
    }

    /// Shown when notes are written in a language other than English by a local model.
    static func localModelNote(outputTag: String, usesOllama: Bool, languageName: (String) -> String) -> String? {
        guard !outputTag.isEmpty, !outputTag.hasPrefix("en"), usesOllama else { return nil }
        return String(localized: "Local models may write less well in \(languageName(outputTag)).")
    }

    static func languageName(_ tag: String) -> String { Locale.current.localizedString(forIdentifier: tag) ?? tag }

    static func labels() -> WGeneralLabels {
        var l = WGeneralLabels()
        l.general = String(localized: "General")
        l.appVersion = String(localized: "App version")
        l.coreVersion = String(localized: "Core version")
        l.checkForUpdates = String(localized: "Check for Updates")
        l.updatesLabel = String(localized: "Updates", comment: "Label of the Settings row whose button is Check for Updates.")
        l.restartToUpdate = String(localized: "Restart to Update")
        l.language = String(localized: "Language")
        l.writeIn = String(localized: "Write notes and summaries in")
        l.appearance = String(localized: "Appearance")
        l.skin = String(localized: "Skin")
        l.makeSkin = String(localized: "Make a skin")
        return l
    }

    struct Input {
        var appVersion: String?
        var coreVersion: String
        var update: UpdateChecker.UpdateStatus
        var hasStagedUpdate: Bool
        var restartBlockedByRecording: Bool
        var languageOverride: String
        var languageChanged: Bool
        var languageTags: [String]
        var writeIn: String
        var writeInTags: [String]
        var localModelNote: String?
        var appearance: AppearanceChoice
        var appearanceLockedCaption: String?
        var skins: WSkinGalleryModel
    }

    static func model(_ i: Input, languageName: (String) -> String = GeneralSettingsAdapter.languageName) -> WGeneralSettingsModel {
        WGeneralSettingsModel(
            labels: labels(),
            appVersion: i.appVersion ?? "unknown", coreVersion: i.coreVersion,
            updateStatus: updateStatusLine(i.update), updateBusy: updateBusy(i.update),
            restartAvailable: i.hasStagedUpdate,
            restartBlockedNote: i.restartBlockedByRecording ? String(localized: "Finish or stop the current recording before restarting to install the update.") : nil,
            languageOptions: [WChoice(id: "", title: String(localized: "Automatic"))] + i.languageTags.map { WChoice(id: $0, title: languageName($0)) },
            languageSelection: i.languageOverride,
            languageChangedNote: i.languageChanged ? String(localized: "Quit and reopen Whimbrel to use the new language.") : nil,
            writeInOptions: [WChoice(id: "", title: String(localized: "Same as app language"))] + i.writeInTags.map { WChoice(id: $0, title: languageName($0)) },
            writeInSelection: i.writeIn,
            routinesNote: String(localized: "Routines run on your primary device and write in that device's language."),
            localModelNote: i.localModelNote,
            appearanceOptions: AppearanceChoice.allCases.map { WChoice(id: $0.rawValue, title: $0.title) },
            appearanceSelection: i.appearance.rawValue, appearanceLockedCaption: i.appearanceLockedCaption,
            skins: i.skins, anchors: SettingsAnchorID.general)
    }
}

struct GeneralSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let openAISettings: () -> Void
    let highlighted: String?

    var body: some View {
        GeneralSettingsRedesignBody(
            appState: appState, updateChecker: appState.updateChecker, appearance: appState.appearanceSettings,
            skinChat: appState.skinChat, openAISettings: openAISettings, highlighted: highlighted)
    }
}

private struct GeneralSettingsRedesignBody: View {
    let appState: AppState
    @ObservedObject var updateChecker: UpdateChecker
    @ObservedObject var appearance: AppearanceSettings
    @ObservedObject var skinChat: SkinChatSession
    let openAISettings: () -> Void
    let highlighted: String?
    @ObservedObject private var skins = SkinStore.shared
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.colorScheme) private var colorScheme
    @State private var restartBlockedByRecording = false
    @State private var languageOverride = AppLanguage.override ?? ""
    @State private var languageChanged = false
    @AppStorage(AppLanguage.writeInKey) private var writeIn = ""

    private var model: WGeneralSettingsModel {
        let outputTag = AppLanguage.outputTag(uiTag: AppLanguage.uiTag(), writeIn: writeIn, multilingual: AppLanguage.multilingual())
        let note = GeneralSettingsAdapter.localModelNote(
            outputTag: outputTag, usesOllama: appState.routing.preferredProvider == .ollama, languageName: GeneralSettingsAdapter.languageName)
        return GeneralSettingsAdapter.model(GeneralSettingsAdapter.Input(
            appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
            coreVersion: CoreMemoryStore.coreVersion, update: updateChecker.status,
            hasStagedUpdate: updateChecker.downloadedUpdate != nil, restartBlockedByRecording: restartBlockedByRecording,
            languageOverride: languageOverride, languageChanged: languageChanged, languageTags: AppLanguage.shipped,
            writeIn: writeIn, writeInTags: AppLanguage.writeInOptions, localModelNote: note,
            appearance: appearance.choice, appearanceLockedCaption: appearance.lockedCaption,
            skins: SkinGalleryAdapter.model(
                skins: skins.skins, redesign: RedesignFlag.isOn(), selectedSkinId: skins.selected.id,
                selectedPaletteId: skins.selectedPaletteId, palettes: { skins.palettes(for: $0) },
                dark: colorScheme == .dark, isBusy: skinChat.isRunning)))
    }

    private var actions: WGeneralSettingsActions {
        WGeneralSettingsActions(
            checkForUpdates: { Task { await updateChecker.checkNow() } },
            restartToUpdate: restart,
            setLanguage: { tag in
                languageOverride = tag
                AppLanguage.setOverride(tag.isEmpty ? nil : tag)
                languageChanged = true
            },
            setWriteIn: { writeIn = $0 },
            setAppearance: { id in if let choice = AppearanceChoice(rawValue: id) { appearance.choice = choice } },
            skins: SkinGalleryActionsAdapter(skins: skins, themes: themes, colorScheme: colorScheme).actions())
    }

    /// Keyed on the staged update, not `.ready`: a later failed check must not hide it.
    private func restart() {
        guard let update = updateChecker.downloadedUpdate else { return }
        switch appState.restartIntoUpdate(update) {
        case .recording: restartBlockedByRecording = true
        // The staged copy was cleaned up; fetch it again.
        case .missing:
            updateChecker.discardMissingStagedUpdate()
            Task { await updateChecker.checkNow() }
        case nil: break
        }
    }

    var body: some View {
        WGeneralSettingsView(
            model: model, actions: actions, highlighted: highlighted,
            makeSkin: AnyView(SkinChatView(session: skinChat, setUp: openAISettings)),
            permissions: AnyView(PermissionChecklistRedesignView().environmentObject(appState.permissions)))
    }
}
