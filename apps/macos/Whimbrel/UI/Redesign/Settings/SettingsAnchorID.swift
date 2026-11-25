import Foundation
import WhimbrelUI

/// Every Settings search anchor id, typed. The raw value is the registry id (`SettingsSearch.entries`), so a redrawn
/// section takes its ids from here instead of repeating the literal the classic panes carry. A repeat of that literal
/// anywhere under `Whimbrel/UI` would break `SettingsSearchTests.testEveryEntryAnchorExistsInSource`, which wants each one
/// exactly once, so no file in this folder writes it. `SettingsRedesignTests` checks the enum against the registry.
enum SettingsAnchorID: String, CaseIterable {
    case generalUpdate = "general.update"
    case generalLanguage = "general.language"
    case generalWriteIn = "general.writeIn"
    case generalAppearance = "general.appearance"
    case generalPermissions = "general.permissions"
    case generalSkin = "general.skin"
    case profileUse = "profile.use"
    case profileSuggest = "profile.suggest"
    case profileAbout = "profile.about"
    case profileFacts = "profile.facts"
    case planSummary = "plan.summary"
    case planManage = "plan.manage"
    case planProvider = "plan.provider"
    case captureYourData = "capture.yourData"
    case captureExport = "capture.export"
    case captureDelete = "capture.delete"
    case captureObsidian = "capture.obsidian"
    case captureEncryption = "capture.encryption"
    case captureScreenCaptures = "capture.screenCaptures"
    case captureExcludeApps = "capture.excludeApps"
    case captureExcludeSites = "capture.excludeSites"
    case captureHotkey = "capture.hotkey"
    case captureRetention = "capture.retention"
    case captureMedia = "capture.media"
    case captureStorage = "capture.storage"
    case meetingsRecordingNotice = "meetings.recordingNotice"
    case meetingsAutoStart = "meetings.autoStart"
    case meetingsLiveTranscript = "meetings.liveTranscript"
    case meetingsHeuristic = "meetings.heuristic"
    case meetingsSnapshots = "meetings.snapshots"
    case meetingsVoices = "meetings.voices"
    case meetingsTranscription = "meetings.transcription"
    case meetingsMultilingual = "meetings.multilingual"
    case aiLocalOnly = "ai.localOnly"
    case aiWhimbrelAI = "ai.whimbrelAI"
    case aiAnthropicKey = "ai.anthropicKey"
    case aiOpenaiKey = "ai.openaiKey"
    case aiGoogleKey = "ai.googleKey"
    case aiOpenrouterKey = "ai.openrouterKey"
    case aiOllama = "ai.ollama"
    case aiProvider = "ai.provider"
    case aiModel = "ai.model"
    case aiDailyCap = "ai.dailyCap"
    case aiSpend = "ai.spend"
    case aiUsage = "ai.usage"
    case aiTypesafe = "ai.typesafe"
    case aiPrompts = "ai.prompts"
    case topicsList = "topics.list"
    case topicsAdd = "topics.add"
    case topicsNames = "topics.names"
    case devicesOverview = "devices.overview"
    case devicesSyncNow = "devices.syncNow"
    case devicesName = "devices.name"
    case devicesPhone = "devices.phone"
    case devicesDeleteAccount = "devices.deleteAccount"
    case connectorsCalendar = "connectors.calendar"
    case connectorsGmail = "connectors.gmail"
    case connectorsOutlook = "connectors.outlook"
    case connectorsOwnClient = "connectors.ownClient"
    case connectorsMcp = "connectors.mcp"

    /// The anchors the redrawn General section applies, in the form the package view takes.
    static var general: WGeneralAnchors {
        WGeneralAnchors(update: generalUpdate.rawValue, language: generalLanguage.rawValue, writeIn: generalWriteIn.rawValue,
                        appearance: generalAppearance.rawValue, skin: generalSkin.rawValue, permissions: generalPermissions.rawValue)
    }

    static var profile: WProfileAnchors {
        WProfileAnchors(use: profileUse.rawValue, suggest: profileSuggest.rawValue, about: profileAbout.rawValue, facts: profileFacts.rawValue)
    }

    static var plan: WPlanAnchors {
        WPlanAnchors(summary: planSummary.rawValue, manage: planManage.rawValue, provider: planProvider.rawValue)
    }

    static var memoryPrivacy: WMemoryPrivacyAnchors {
        WMemoryPrivacyAnchors(
            yourData: captureYourData.rawValue, export: captureExport.rawValue, delete: captureDelete.rawValue, obsidian: captureObsidian.rawValue,
            encryption: captureEncryption.rawValue, screenCaptures: captureScreenCaptures.rawValue, excludeApps: captureExcludeApps.rawValue,
            excludeSites: captureExcludeSites.rawValue, hotkey: captureHotkey.rawValue, retention: captureRetention.rawValue,
            media: captureMedia.rawValue, storage: captureStorage.rawValue)
    }

    static var meetings: WMeetingsAnchors {
        WMeetingsAnchors(
            recordingNotice: meetingsRecordingNotice.rawValue, autoStart: meetingsAutoStart.rawValue, liveTranscript: meetingsLiveTranscript.rawValue,
            heuristic: meetingsHeuristic.rawValue, snapshots: meetingsSnapshots.rawValue, voices: meetingsVoices.rawValue,
            transcription: meetingsTranscription.rawValue, multilingual: meetingsMultilingual.rawValue)
    }

    static var ai: WAIAnchors {
        WAIAnchors(
            localOnly: aiLocalOnly.rawValue, whimbrelAI: aiWhimbrelAI.rawValue, anthropicKey: aiAnthropicKey.rawValue, openaiKey: aiOpenaiKey.rawValue,
            googleKey: aiGoogleKey.rawValue, openrouterKey: aiOpenrouterKey.rawValue, ollama: aiOllama.rawValue, provider: aiProvider.rawValue,
            model: aiModel.rawValue, dailyCap: aiDailyCap.rawValue, spend: aiSpend.rawValue, usage: aiUsage.rawValue, typesafe: aiTypesafe.rawValue,
            prompts: aiPrompts.rawValue)
    }

    static var topics: WTopicsAnchors {
        WTopicsAnchors(list: topicsList.rawValue, add: topicsAdd.rawValue, names: topicsNames.rawValue)
    }

    static var sync: WSyncAnchors {
        WSyncAnchors(
            overview: devicesOverview.rawValue, syncNow: devicesSyncNow.rawValue, name: devicesName.rawValue, phone: devicesPhone.rawValue,
            deleteAccount: devicesDeleteAccount.rawValue)
    }

    static var connectors: WConnectorsAnchors {
        WConnectorsAnchors(
            calendar: connectorsCalendar.rawValue, gmail: connectorsGmail.rawValue, outlook: connectorsOutlook.rawValue,
            ownClient: connectorsOwnClient.rawValue, mcp: connectorsMcp.rawValue)
    }
}
