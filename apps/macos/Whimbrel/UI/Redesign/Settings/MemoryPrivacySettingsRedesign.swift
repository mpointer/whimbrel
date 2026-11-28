import AppKit
import Combine
import SwiftUI
import WhimbrelUI

/// Memory & Privacy in the redesigned Settings window. This file feeds the package's `WMemoryPrivacySettingsView` from the
/// app's state and calls the same functions the classic panes call (`CaptureSettingsPane`, `YourDataSettingsPane`,
/// `RetentionSettingsPane`, `FreeUpSpaceSheet`, `EncryptionSettingsCard`). The classic panes are not changed.
/// Recording notice and consent reminder controls live in Meetings today, so they are not part of this section.

// MARK: Adapter (pure, tested)

enum MemoryPrivacyAdapter {
    /// The words for each retention class, from the naming table: Screen text; Meetings and recordings; Conversations.
    static func className(_ kind: WRetentionKind) -> String {
        switch kind {
        case .screenText: return String(localized: "Screen text", comment: "Name of a retention row in Settings: how long Memory keeps the text it saved from your screen.")
        case .meetings: return String(localized: "Meetings and recordings", comment: "Name of a retention row in Settings: how long meetings and their recordings are kept.")
        case .conversations: return String(localized: "Conversations")
        case .deleteAudio: return String(localized: "Delete audio after")
        }
    }

    /// Why memory is on but nothing is saved, in the menu bar's own words: a missing Accessibility permission, else the
    /// reason `MenuBarAdapter.memoryNeed` gives. Nil for any other state.
    static func memoryReason(memory: WMemoryState, accessibilityGranted: Bool, noStoreShown: Bool) -> String? {
        guard memory == .onNotCapturing else { return nil }
        if !accessibilityGranted {
            let needs = MenuBarAdapter.permissionNeeds(MenuBarAdapter.Permissions(screenRecording: true, microphone: true, accessibility: false))
            if let need = needs.first { return [need.title, need.detail].compactMap { $0 }.joined(separator: " ") }
        }
        if let need = MenuBarAdapter.memoryNeed(memory: memory, accessibilityGranted: accessibilityGranted, noStoreShown: noStoreShown) {
            return [need.title, need.detail].compactMap { $0 }.joined(separator: " ")
        }
        return nil
    }

    /// The companion word on the three retention rows and on Delete audio: the data is kept as it is, with no limit.
    static var foreverTitle: String { String(localized: "Keep forever") }
    /// The companion word on Compress audio and Shrink images. It does not mean the audio or the images are kept forever, it
    /// means they are never compressed or shrunk, so it says so.
    static var fullQualityTitle: String { String(localized: "Keep full quality", comment: "Checkbox on the Compress audio after and Shrink images after rows in Settings: when it is on, the audio or the images are never made smaller.") }

    /// The write for one of the three media rows (the flag and the days the classic pane writes).
    @MainActor
    static func applyMedia(_ id: String, days: Int?, forever: Bool, to m: MediaSettings) {
        switch id {
        case "compress": if let days { m.compressAfterDays = days }; m.keepFullAudioForever = forever
        case "deleteAudio": if let days { m.deleteAudioAfterDays = days }; m.neverDeleteAudio = forever
        case "shrink": if let days { m.shrinkImagesAfterDays = days }; m.keepFullImagesForever = forever
        default: break
        }
    }

    static func retentionModels(captureDays: Int, captureForever: Bool, meetingDays: Int, meetingForever: Bool, conversationDays: Int, conversationForever: Bool, firstAnchor: String) -> [WRetentionRowModel] {
        let values: [(String, WRetentionKind, Int, Bool)] = [
            ("screenText", .screenText, captureDays, captureForever), ("meetings", .meetings, meetingDays, meetingForever),
            ("conversations", .conversations, conversationDays, conversationForever),
        ]
        return values.enumerated().map { index, v in
            WRetentionRowModel(id: v.0, kind: v.1, label: className(v.1), days: v.2, keepForever: v.3, foreverTitle: foreverTitle, asksBeforeShortening: true, anchor: index == 0 ? firstAnchor : nil)
        }
    }

    static func mediaModel(
        folder: String, canReset: Bool, refusal: String?, compressDays: Int, compressForever: Bool, deleteDays: Int, deleteForever: Bool,
        shrinkDays: Int, shrinkForever: Bool, deleteNote: String?, usage: String?, preview: String?, progress: String?, fraction: Double?
    ) -> WMediaModel {
        WMediaModel(
            folderPath: folder, canReset: canReset, folderRefusal: refusal,
            compress: WRetentionRowModel(id: "compress", kind: nil, label: String(localized: "Compress audio after"), days: compressDays, keepForever: compressForever, foreverTitle: fullQualityTitle, asksBeforeShortening: true, effect: .recompressesAudio),
            deleteAudio: WRetentionRowModel(id: "deleteAudio", kind: .deleteAudio, label: String(localized: "Delete audio after"), days: deleteDays, keepForever: deleteForever, foreverTitle: foreverTitle, asksBeforeShortening: true),
            shrink: WRetentionRowModel(id: "shrink", kind: nil, label: String(localized: "Shrink images after"), days: shrinkDays, keepForever: shrinkForever, foreverTitle: fullQualityTitle, asksBeforeShortening: true, effect: .shrinksImages),
            deleteSwitchNote: deleteNote, usage: usage, preview: preview, progress: progress, progressFraction: fraction)
    }

    /// "3,904 screen captures are older than 30 days.": what the count counts, the number formatted for the language and the days
    /// in the catalog's plural forms. One key for one thing and one for several, because the noun changes with the count.
    static func countLine(kind: WRetentionKind?, count: Int64, days: Int) -> String {
        let n = count.formatted(), d = SettingsCopy.retentionDays(days), one = count == 1
        switch kind {
        case .meetings?:
            return one ? String(localized: "\(n) meeting is older than \(d).", comment: "Settings, retention question: one meeting is older than the new period. The first placeholder is the number 1, the second the period, such as 30 days.")
                : String(localized: "\(n) meetings are older than \(d).", comment: "Settings, retention question: how many meetings are older than the new period. The first placeholder is a count, the second the period, such as 30 days.")
        case .conversations?:
            return one ? String(localized: "\(n) conversation is older than \(d).", comment: "Settings, retention question: one conversation is older than the new period. The first placeholder is the number 1, the second the period, such as 30 days.")
                : String(localized: "\(n) conversations are older than \(d).", comment: "Settings, retention question: how many conversations are older than the new period. The first placeholder is a count, the second the period, such as 30 days.")
        case .deleteAudio?:
            return one ? String(localized: "\(n) recording is older than \(d).", comment: "Settings, retention question: one recording is older than the new period. The first placeholder is the number 1, the second the period, such as 30 days.")
                : String(localized: "\(n) recordings are older than \(d).", comment: "Settings, retention question: how many recordings are older than the new period. The first placeholder is a count, the second the period, such as 30 days.")
        case .screenText?, nil:
            return one ? String(localized: "\(n) screen capture is older than \(d).", comment: "Settings, retention question: one screen capture is older than the new period. The first placeholder is the number 1, the second the period, such as 30 days.")
                : String(localized: "\(n) screen captures are older than \(d).", comment: "Settings, retention question: how many screen captures are older than the new period. The first placeholder is a count, the second the period, such as 30 days.")
        }
    }

    // The clean-up sentences, worded without dashes.
    static let orphanNotice = String(localized: "Recording files with no matching meeting record at all, for example from a crashed or interrupted session. Retention above only removes files for meetings it actually deletes; this finds files that were never tracked by any meeting in the first place.", comment: "Settings clean-up: what orphaned recording files are.")
    static let duplicateNotice = String(localized: "A sync bug (fixed 2026-08-05) could duplicate a peer's meeting once per lifecycle update. This finds and removes any leftover duplicates from before the fix. It never affects this device's own meetings.", comment: "Settings clean-up: what duplicate meetings are.")
    static let noAudioNotice = String(localized: "Meeting entries that never captured any audio, for example a calendar-triggered meeting that started and ended before anything was recorded. Meetings that are still recording are never included.", comment: "Settings clean-up: what meetings without audio are.")
    static func duplicateConfirm(_ extra: Int) -> String {
        String(localized: "This will permanently delete \(extra) duplicate meeting(s). The most recently applied copy in each group is kept. This cannot be undone.", comment: "Settings clean-up: confirmation before duplicate meetings are deleted. The placeholder is a number.")
    }

    /// Every sentence the redesigned section writes of its own, for the copy tests.
    static var ownStrings: [String] {
        let l = labels()
        return [
            l.memoryExplainer, l.showScreenCapturesCaption, l.neverIntro, l.appsHelp, l.yourDataIntro, l.exportNotice, l.deleteRowHelp, l.keepingIntro,
            l.applyNowHelp, l.mediaIntro, l.cleanupIntro, l.pausedHint, l.exportFirstBody, l.fastSearchHelp, l.encrypting, l.periodNotSaved,
            orphanNotice, duplicateNotice, noAudioNotice, duplicateConfirm(2), l.applyConfirmTitle, l.applyConfirmMessage([.init(kind: .screenText, days: 30)]), l.applyNowNothing,
        ] + retentionAsks.flatMap { [l.askTitle($0), l.askMessage($0), l.askConfirm($0)] } + LibraryCopyRedesign.all
    }

    /// One question per effect and kind, with and without a count, for the copy tests.
    static var retentionAsks: [WRetentionAsk] {
        let m = mediaModel(folder: "/m", canReset: false, refusal: nil, compressDays: 30, compressForever: false, deleteDays: 30, deleteForever: false, shrinkDays: 30, shrinkForever: false, deleteNote: nil, usage: nil, preview: nil, progress: nil, fraction: nil)
        let named = retentionModels(captureDays: 30, captureForever: false, meetingDays: 30, meetingForever: false, conversationDays: 30, conversationForever: false, firstAnchor: "a")
        let rows: [(WRetentionEffect, WRetentionKind?, String)] = (named + [m.deleteAudio]).map { ($0.effect, $0.kind, $0.label) } + [(.recompressesAudio, nil, m.compress.label), (.shrinksImages, nil, m.shrink.label)]
        return rows.flatMap { effect, kind, label in
            [WRetentionAsk(effect: effect, kind: kind, name: label, days: 7, replacing: .forever, count: 5),
             WRetentionAsk(effect: effect, kind: kind, name: label, days: 7, replacing: .days(30), count: nil)]
        }
    }

    static func labels() -> WMemoryPrivacyLabels {
        var l = WMemoryPrivacyLabels()
        l.memoryHeading = String(localized: "Memory", comment: "Label next to a time range on a block that lists what Whimbrel saw in that time.")
        l.memoryExplainer = String(localized: "Memory saves the text of the window you are working in, with its title, its app and, in a browser, the page address, so you can search for it and ask about it later. If a window gives almost no text, Whimbrel tries to read it from a picture of the window and keeps only the text. It saves nothing from apps you list under Never captured, or from listed sites in browsers that show Whimbrel the page address.", comment: "Settings: one sentence saying what Memory saves.")
        l.pause = String(localized: "Pause", comment: "Button on the recording bar that pauses a recording.")
        l.turnOn = String(localized: "Turn on", comment: "Button in Settings that turns Memory on.")
        l.turnOff = String(localized: "Turn off", comment: "Button in Settings that turns Memory off.")
        l.pausedHint = String(localized: "Choose Resume now in the Pause menu to start saving again.", comment: "Shown in Settings while Memory is paused for a while.")
        l.showScreenCaptures = String(localized: "Show screen captures in Library")
        l.showScreenCapturesCaption = String(localized: "Off by default: the Recent tab shows meetings only. Screen text is still saved and searchable; this only decides whether captures appear in the Recent list.", comment: "Caption under the Show screen captures in Library switch in Settings.")
        l.neverHeading = String(localized: "Never captured", comment: "Heading of the Settings card that lists apps and sites Memory never reads.")
        l.neverIntro = String(localized: "Apps you list here are not read at all: the check happens before anything is taken from their windows. A site needs the page address first, so its text is read for a moment and dropped before it is saved or sent to text recognition.", comment: "Settings: what excluding an app or a site means.")
        l.appsLabel = String(localized: "Apps never captured", comment: "Label of the list of running apps that Memory never reads.")
        l.appsHelp = String(localized: "Turn on an app to leave it out.", comment: "Help under the list of apps in Never captured.")
        l.appsSearchPlaceholder = String(localized: "Search running apps…")
        l.noRunningApps = String(localized: "No running apps found.")
        l.noMatches = String(localized: "No matches.")
        l.sitesLabel = String(localized: "Sites never captured")
        l.sitesHelp = String(localized: "Pages on these sites, and their subdomains, are dropped before anything is saved. Works in browsers that expose the page address.")
        l.domainPlaceholder = String(localized: "example.com")
        l.domainRefused = String(localized: "That doesn't look like a site. Enter a domain such as example.com.")
        l.noSites = String(localized: "No sites excluded.")
        l.add = String(localized: "Add")
        l.remove = String(localized: "Remove")
        l.removeNamed = { name in String(localized: "Remove \(name)", comment: "Spoken name of the Remove button on a list row. The placeholder is the item's text.") }
        l.addSiteLabel = String(localized: "Add a site", comment: "Spoken name of the field that adds a site to Sites never captured.")
        l.yourDataHeading = String(localized: "Your data")
        l.yourDataIntro = String(localized: "What Whimbrel keeps on this Mac, what leaves it, and how to take it with you or remove it.")
        l.openAISettings = String(localized: "Open AI settings", comment: "Button in Your data: opens the AI settings page.")
        l.counting = String(localized: "Counting…")
        l.onThisMac = String(localized: "On this Mac")
        l.dataFolder = String(localized: "Data folder", comment: "Settings, Your data: the folder where Whimbrel keeps its library. The size follows on the next line.")
        l.mediaFolderLine = String(localized: "Recordings and slide images", comment: "Settings, Your data: where recordings and slide images are kept. The folder and size follow on the next line.")
        l.showInFinder = String(localized: "Show in Finder")
        l.leavesHeading = String(localized: "What leaves this Mac")
        l.privacyLink = String(localized: "Read the full list on the privacy page")
        l.deleteForeign = EncryptionCopy.foreignFilesDeleteAction
        l.foreignConfirmTitle = EncryptionCopy.foreignFilesConfirmTitle
        l.foreignConfirmButton = EncryptionCopy.foreignFilesConfirm
        l.exportHeading = String(localized: "Export and delete", comment: "Heading of the Settings card with Export everything, the Obsidian vault and Delete everything.")
        l.exportLabel = String(localized: "Export everything", comment: "Label of the Export everything row in Settings. The button on it has an ellipsis.")
        l.exportButton = String(localized: "Export everything…")
        l.exportingButton = String(localized: "Exporting…")
        l.exportNotice = String(localized: "Meetings with their transcripts as Markdown, screen text, email, calendar, conversations and routines as JSON, and topics as Obsidian notes. Readable without Whimbrel. Recordings and slide images are not exported.")
        l.vaultLabel = String(localized: "Keep an Obsidian vault up to date")
        l.vaultFolder = String(localized: "Vault folder", comment: "Settings: the folder of the Obsidian vault.")
        l.noFolderChosen = String(localized: "No folder chosen yet.", comment: "Settings: shown when no Obsidian vault folder is chosen.")
        l.chooseFolder = String(localized: "Choose folder…")
        l.vaultFailed = String(localized: "Couldn't update the vault. Check that the folder exists and Whimbrel can write to it.")
        l.deleteLeftoversLead = String(localized: "Delete everything could not remove these. Quit Whimbrel and remove them by hand:", comment: "Settings: lead-in to the list of files Delete everything could not remove.")
        l.dismiss = String(localized: "Dismiss")
        l.deleteRowLabel = String(localized: "Delete everything on this Mac", comment: "Label of the Delete everything row in Settings.")
        l.deleteRowHelp = String(localized: "Removes your library and this Mac's recordings and slide images, then restarts Whimbrel as new. You are asked to confirm first.", comment: "Help under the Delete everything row in Settings.")
        l.deleteButton = String(localized: "Delete everything…", comment: "Button in Settings that opens the Delete everything confirmation.")
        l.deleteSheetTitle = String(localized: "Delete everything on this Mac?")
        l.alsoDeleteSync = String(localized: "Also delete my sync account")
        l.typeDelete = String(localized: "Type DELETE to confirm.")
        l.deleteWord = "DELETE"
        l.cancel = String(localized: "Cancel")
        l.deleteAndRestart = String(localized: "Delete and restart")
        l.keepingHeading = String(localized: "Keeping data", comment: "Heading of the Settings card with retention periods and storage.")
        l.keepingIntro = String(localized: "Each kind of data has its own period. Retention deletes what is older than the period. It runs by itself shortly after Whimbrel starts and then once a day.", comment: "Settings: what the retention periods do.")
        l.storageMeasuring = String(localized: "Measuring what Whimbrel uses…", comment: "Shown in Settings while the storage size is measured.")
        l.freeUpLabel = String(localized: "Free up space")
        l.freeUpButton = String(localized: "Free Up Space…")
        l.applyNow = String(localized: "Apply Retention Now")
        l.applyNowHelp = String(localized: "Deletes what is older than the periods above right now. Whimbrel may then compact the database to give space back.", comment: "Help under the Apply Retention Now row in Settings.")
        l.applyNowNothing = String(localized: "Every period above is Keep forever, so there is nothing to delete.", comment: "Shown on the Apply Retention Now row when no period has a limit.")
        l.compactLabel = String(localized: "Compact Database")
        l.compactButton = String(localized: "Compact Database")
        l.days = { n in SettingsCopy.retentionDays(n) }
        l.savePeriod = { n in String(localized: "Keep for \(n) days", comment: "Button in Settings that saves a retention period. The placeholder is the number of days.") }
        l.periodNotSaved = String(localized: "Not saved yet. Choose how long to keep it, then save.", comment: "Shown under a retention row after Keep forever was turned off, until a period is saved.")
        l.cancelPeriod = String(localized: "Cancel")
        l.periodProblem = { problem in
            switch problem {
            case .empty: return String(localized: "Enter a number of days. Nothing was changed.", comment: "Settings, retention: shown when no number of days was given.")
            case .belowMinimum(let m): return String(localized: "Use \(m) days or more. Nothing was changed.", comment: "Settings, retention: the number of days is too small. The placeholder is the smallest allowed number.")
            case .aboveMaximum(let m): return String(localized: "Use \(m) days or fewer. Nothing was changed.", comment: "Settings, retention: the number of days is too large. The placeholder is the largest allowed number.")
            }
        }
        l.storedOutOfRange = { n in String(localized: "The saved period is \(n) days, which is outside the allowed range. The row shows the nearest allowed number. Nothing changes until you save a period.", comment: "Settings, retention: the stored number of days is not allowed. The placeholder is that number.") }
        l.askTitle = { ask in
            switch ask.effect {
            case .deletes:
                switch ask.kind {
                case .meetings?: return String(localized: "Delete meetings and recordings older than \(ask.days) days?", comment: "Question before a shorter period for meetings and their recordings is saved. The placeholder is the number of days.")
                case .conversations?: return String(localized: "Delete conversations older than \(ask.days) days?", comment: "Question before a shorter period for conversations is saved. The placeholder is the number of days.")
                case .deleteAudio?: return String(localized: "Delete audio from recordings older than \(ask.days) days?", comment: "Question before a shorter period for deleting the audio of recordings is saved. The placeholder is the number of days.")
                case .screenText?, nil: return String(localized: "Delete screen text older than \(ask.days) days?", comment: "Question before a shorter period for screen text is saved. The placeholder is the number of days.")
                }
            case .recompressesAudio: return String(localized: "Compress audio older than \(ask.days) days?", comment: "Question before a shorter period for compressing audio is saved. The placeholder is the number of days.")
            case .shrinksImages: return String(localized: "Shrink images older than \(ask.days) days?", comment: "Question before a shorter period for shrinking slide images is saved. The placeholder is the number of days.")
            }
        }
        l.askMessage = { ask in
            var text = ""
            if let count = ask.count { text += MemoryPrivacyAdapter.countLine(kind: ask.kind, count: count, days: ask.days) + " " }
            switch (ask.effect, ask.replacing) {
            case (.deletes, .forever):
                switch ask.kind {
                case .meetings?: text += String(localized: "Meetings and recordings are kept forever today.", comment: "Settings, retention question: the current setting for meetings and their recordings is Keep forever.")
                case .conversations?: text += String(localized: "Conversations are kept forever today.", comment: "Settings, retention question: the current setting for conversations is Keep forever.")
                case .deleteAudio?: text += String(localized: "Audio from recordings is kept forever today.", comment: "Settings, retention question: the current setting for the audio of recordings is Keep forever.")
                case .screenText?, nil: text += String(localized: "Screen text is kept forever today.", comment: "Settings, retention question: the current setting for screen text is Keep forever.")
                }
            case (.deletes, .days(let d)):
                switch ask.kind {
                case .meetings?: text += String(localized: "Meetings and recordings are kept for \(d) days today.", comment: "Settings, retention question: the current setting for meetings and their recordings. The placeholder is the number of days.")
                case .conversations?: text += String(localized: "Conversations are kept for \(d) days today.", comment: "Settings, retention question: the current setting for conversations. The placeholder is the number of days.")
                case .deleteAudio?: text += String(localized: "Audio from recordings is kept for \(d) days today.", comment: "Settings, retention question: the current setting for the audio of recordings. The placeholder is the number of days.")
                case .screenText?, nil: text += String(localized: "Screen text is kept for \(d) days today.", comment: "Settings, retention question: the current setting for screen text. The placeholder is the number of days.")
                }
            case (.recompressesAudio, .forever): text += String(localized: "Audio is kept at full quality today.", comment: "Settings, compress audio question: the current setting is to never compress.")
            case (.recompressesAudio, .days(let d)): text += String(localized: "Audio is compressed after \(d) days today.", comment: "Settings, compress audio question: the current setting. The placeholder is the number of days.")
            case (.shrinksImages, .forever): text += String(localized: "Images are kept at full quality today.", comment: "Settings, shrink images question: the current setting is to never shrink.")
            case (.shrinksImages, .days(let d)): text += String(localized: "Images are shrunk after \(d) days today.", comment: "Settings, shrink images question: the current setting. The placeholder is the number of days.")
            }
            text += " "
            switch ask.effect {
            case .recompressesAudio:
                return text + String(localized: "Audio older than \(ask.days) days is re-encoded at lower quality and the original recording is removed. This can't be undone.", comment: "Settings, compress audio question: what the new period does. The placeholder is the number of days.")
            case .shrinksImages:
                return text + String(localized: "Slide images older than \(ask.days) days are made smaller and the full-size image is removed. This can't be undone.", comment: "Settings, shrink images question: what the new period does. The placeholder is the number of days.")
            case .deletes:
                switch ask.kind {
                case .meetings?: return text + String(localized: "Meetings older than \(ask.days) days are deleted, with their transcripts, recordings and slide images, the next time retention runs. This can't be undone.", comment: "Settings, meetings retention question: what the new period deletes. The placeholder is the number of days.")
                case .conversations?: return text + String(localized: "Conversations older than \(ask.days) days are deleted, except starred ones, the next time retention runs. This can't be undone.", comment: "Settings, conversations retention question: what the new period deletes. The placeholder is the number of days.")
                case .deleteAudio?: return text + String(localized: "The audio of meetings older than \(ask.days) days is deleted. The meetings and their transcripts stay. This can't be undone.", comment: "Settings, delete audio question: what the new period deletes. The placeholder is the number of days.")
                case .screenText?, nil: return text + String(localized: "Screen text older than \(ask.days) days is deleted the next time retention runs. This can't be undone.", comment: "Settings, screen text retention question: what the new period deletes. The placeholder is the number of days.")
                }
            }
        }
        l.askConfirm = { ask in
            switch ask.effect {
            case .deletes: return String(localized: "Keep \(ask.days) days and delete older", comment: "Button in the retention question: saves the period and deletes what is older. The placeholder is the number of days.")
            case .recompressesAudio: return String(localized: "Compress after \(ask.days) days", comment: "Button in the compress audio question: saves the period. The placeholder is the number of days.")
            case .shrinksImages: return String(localized: "Shrink after \(ask.days) days", comment: "Button in the shrink images question: saves the period. The placeholder is the number of days.")
            }
        }
        l.applyConfirmTitle = String(localized: "Delete old data now?", comment: "Question before Apply Retention Now runs.")
        l.applyConfirmLine = { limit in
            switch limit.kind {
            case .meetings: return String(localized: "Meetings and recordings older than \(limit.days) days are deleted.", comment: "Settings: one line of the Apply Retention Now question, for meetings and their recordings. The placeholder is the number of days.")
            case .conversations: return String(localized: "Conversations older than \(limit.days) days are deleted, except starred ones.", comment: "Settings: one line of the Apply Retention Now question, for conversations. The placeholder is the number of days.")
            case .deleteAudio: return String(localized: "The audio of meetings older than \(limit.days) days is deleted.", comment: "Settings: one line of the Apply Retention Now question, for the audio of recordings. The placeholder is the number of days.")
            case .screenText: return String(localized: "Screen text older than \(limit.days) days is deleted.", comment: "Settings: one line of the Apply Retention Now question, for screen text. The placeholder is the number of days.")
            }
        }
        l.applyConfirmNothing = String(localized: "Every period is set to Keep forever, so nothing will be deleted.", comment: "Settings: Apply Retention Now question when no period is set.")
        l.cantBeUndone = String(localized: "This can't be undone.")
        l.applyConfirmButton = String(localized: "Delete now", comment: "Confirm button of the Apply Retention Now question.")
        l.mediaHeading = String(localized: "Recordings and slide images", comment: "Settings, Your data: where recordings and slide images are kept. The folder and size follow on the next line.")
        l.mediaIntro = String(localized: "Meeting audio and slide snapshots are kept in this folder, in Audio and Snapshots. Network shares work but are not recommended.")
        l.mediaFolder = String(localized: "Media folder")
        l.choose = String(localized: "Choose…")
        l.reset = String(localized: "Reset")
        l.cancelRun = String(localized: "Cancel")
        l.cleanUp = String(localized: "Clean Up")
        l.cleanUpNeedsScan = String(localized: "Scan first. Clean Up is off until the scan finds something.", comment: "Shown on a Settings clean-up row before its scan has run, to say why Clean Up is off.")
        l.cleanUpNothingFound = String(localized: "The scan found nothing to clean up.", comment: "Tooltip on the Clean Up button when a scan found nothing.")
        l.resetDisabled = String(localized: "Already using the default folder.", comment: "Tooltip on the Reset button of the media folder when the folder is already the default.")
        l.cleanupHeading = String(localized: "Clean up", comment: "Heading of the Settings card with scans for leftover files and duplicate meetings.")
        l.cleanupIntro = String(localized: "Find leftovers and remove them by hand. Each button here asks before it deletes anything. Whimbrel also removes orphaned recording files by itself once a day.", comment: "Settings: what the Clean up card does.")
        l.encryptionHeading = String(localized: "Library encryption", comment: "Heading of the Settings card about encrypting your library, the file where Whimbrel keeps what it saved.")
        l.encryptAction = String(localized: "Encrypt your library…", comment: "Button in Settings that starts encrypting your library.")
        l.exportFirstTitle = EncryptionCopy.exportFirstTitle
        l.exportFirstBody = EncryptionCopy.exportFirstBody
        l.exportAction = EncryptionCopy.exportAction
        l.continueWithoutExport = EncryptionCopy.continueWithoutExport
        l.continueAfterExport = EncryptionCopy.continueAfterExport
        l.checkingFirst = EncryptionCopy.checkingFirst
        l.encrypting = String(localized: "Encrypting your library…", comment: "Shown while the library is being encrypted.")
        l.restart = EncryptionCopy.restartAction
        l.ok = EncryptionCopy.dismiss
        l.fastSearch = EncryptionCopy.fastSearchToggle
        l.fastSearchHelp = LibraryCopyRedesign.map(EncryptionCopy.fastSearchHelp)
        l.speedCheck = EncryptionCopy.speedCheckAction
        l.speedChecking = EncryptionCopy.speedChecking
        l.speedFailed = EncryptionCopy.speedCheckFailed
        l.speedRecording = EncryptionCopy.speedCheckRecording
        l.speedPrivacy = EncryptionCopy.speedCheckPrivacy
        l.saveReport = EncryptionCopy.saveReportAction
        l.savedTo = { EncryptionCopy.savedTo($0) }
        l.hotkeyHeading = String(localized: "Hotkey panel")
        l.hotkeyHelp = String(localized: "Press this shortcut anywhere to ask about the window you are in.")
        l.hotkeyLabel = String(localized: "Shortcut", comment: "Label of the row that shows the Hotkey panel's keyboard shortcut.")
        l.freeUpTitle = String(localized: "Free up space")
        l.olderThan = String(localized: "Older than")
        l.nothingToFree = String(localized: "Nothing to free right now.")
        l.done = String(localized: "Done")
        l.freeUpConfirmButton = String(localized: "Delete")
        l.working = String(localized: "Working…", comment: "Shown while Settings works on a cleanup or a measurement.")
        return l
    }
}

// MARK: Library encryption: "your library", never "memory database" (decision 3)

/// The redesigned card's sentences for the library, in place of `EncryptionCopy`'s "memory database" wording, which the
/// classic pane keeps. The model and the launch notes hand over `EncryptionCopy` strings; `map` swaps each one that says
/// "memory database" for its redesigned sentence and leaves every other one as it is.
enum LibraryCopyRedesign {
    static let pairs: [(classic: String, redesigned: String)] = [
        (EncryptionCopy.settingsNotEncryptedBody, String(localized: "Your library is an ordinary file on this Mac. Encrypting it makes a copy of the file unreadable without a key kept in your Mac Keychain.", comment: "Library encryption card, not encrypted: what the library is.")),
        (EncryptionCopy.settingsEncryptedBody, String(localized: "Your library on this Mac is encrypted with a key kept in your Mac Keychain. A copy of the library file is unreadable without it. Backups made before you turned this on, such as Time Machine, still hold a readable copy.", comment: "Library encryption card, encrypted.")),
        (EncryptionCopy.encryptDone, String(localized: "Your library is now encrypted.", comment: "Library encryption card: encryption finished.")),
        (EncryptionCopy.encryptAtNextLaunch, String(localized: "Whimbrel will finish encrypting your library the next time it starts.", comment: "Library encryption card: encryption continues at the next start.")),
        (EncryptionCopy.skipLowDisk, String(localized: "There is not enough free disk space to encrypt your library. Free up some space and try again.", comment: "Library encryption card: not enough disk space.")),
        (EncryptionCopy.skipOpenElsewhere, String(localized: "Another program has your library open. Close it and try again.", comment: "Library encryption card: another program has the library file open.")),
        (EncryptionCopy.attentionNeedsKey, String(localized: "This Mac has no key for your encrypted library. Use your recovery key to open it.", comment: "Library encryption card, needs attention.")),
        (EncryptionCopy.attentionKeyRejected, String(localized: "The key in your Keychain does not open your library. Use your recovery key to open it.", comment: "Library encryption card, needs attention.")),
        (EncryptionCopy.attentionMigrationInterrupted, String(localized: "Encrypting your library was interrupted. Your data is still there. Restart Whimbrel to finish or undo it.", comment: "Library encryption card, needs attention.")),
        (EncryptionCopy.attentionDamaged, String(localized: "Your library file looks damaged. Whimbrel has not changed it.", comment: "Library encryption card, needs attention.")),
        (EncryptionCopy.attentionStartupFailed, String(localized: "Your library opened, but Whimbrel could not start with it.", comment: "Library encryption card, needs attention.")),
        (EncryptionCopy.noteKeyStoreUnavailable, String(localized: "Whimbrel could not read your Keychain when it started, so encryption cannot start yet. Your library opened as usual.", comment: "Library encryption card note.")),
        (EncryptionCopy.noteSkippedOpenElsewhere, String(localized: "Encryption did not start because another program had your library open. Whimbrel will try again the next time it starts.", comment: "Library encryption card note.")),
        (EncryptionCopy.notePlainBackupKept, String(localized: "An unencrypted copy of your library from before encryption is still on this Mac.", comment: "Library encryption card note.")),
        (EncryptionCopy.noteRestoredCopy, String(localized: "Whimbrel put back the unencrypted copy of your library. It is not encrypted.", comment: "Library encryption card note.")),
        (EncryptionCopy.noteRecoveredKeyNotStored, String(localized: "Your recovery key opened your library, but Whimbrel could not save it in your Keychain. Keep your recovery key at hand: Whimbrel will ask for it again the next time it starts.", comment: "Library encryption card note.")),
        (EncryptionCopy.noteRecoveredKeyNoSlot, String(localized: "Your recovery key opened your library, but your Keychain already holds as many old Whimbrel keys as Whimbrel keeps, so it was not saved. Whimbrel will ask for it again the next time it starts.", comment: "Library encryption card note.")),
        (EncryptionCopy.noteRecoveredKeyUnverified, String(localized: "Your recovery key opened your library, but Whimbrel could not confirm it was saved in your Keychain. If Whimbrel asks for it the next time it starts, use it again.", comment: "Library encryption card note.")),
        (EncryptionCopy.notePendingKeyNotPromoted, String(localized: "Whimbrel could not finish saving its encryption key in your Keychain. It will try again the next time it starts. Your library opened normally.", comment: "Library encryption card note.")),
        (EncryptionCopy.notePendingKeyKept, String(localized: "Whimbrel kept a spare encryption key it found in your Keychain. Your library opened normally.", comment: "Library encryption card note.")),
        (EncryptionCopy.noteStoreUnavailableHint, String(localized: "Whimbrel is running without your library, so nothing new is saved. Restart Whimbrel to try again.", comment: "Library encryption card note.")),
        (EncryptionCopy.fastSearchHelp, String(localized: "Gives search up to 1 GB of extra memory. Takes effect after you restart Whimbrel, and only applies once your library is encrypted.", comment: "Library encryption card: Faster search help.")),
        (EncryptionCopy.encryptContinue, String(localized: "Encrypt your library", comment: "Key setup sheet: the button that starts encrypting the library.")),
        (EncryptionCopy.speedEncrypted, String(localized: "Library: encrypted", comment: "Library encryption card, speed check line.")),
        (EncryptionCopy.speedNotEncrypted, String(localized: "Library: not encrypted", comment: "Library encryption card, speed check line.")),
        // The launch, key-setup and recovery screens, and Your data.
        (EncryptionCopy.recoveryTitle, String(localized: "Whimbrel can't open your library", comment: "Recovery screen title: the library could not be opened.")),
        (EncryptionCopy.recoveryKeyMissing, String(localized: "This Mac has no key for your encrypted library. That happens after a new user account, a Keychain reset, or when the data folder came from another Mac.", comment: "Recovery screen: why the library cannot be opened.")),
        (EncryptionCopy.recoveryKeyRejected, String(localized: "The key in your Mac Keychain does not open your library.", comment: "Recovery screen: why the library cannot be opened.")),
        (EncryptionCopy.recoveryNeverDeletes, String(localized: "Whimbrel will not delete or replace your library. It stays on this Mac, unchanged, until you unlock it.", comment: "Recovery screen: nothing is deleted.")),
        (EncryptionCopy.restored, String(localized: "Unlocked. Whimbrel is opening your library.", comment: "Recovery screen: the recovery key worked.")),
        (EncryptionCopy.keyForAnotherDatabase, String(localized: "That recovery key does not open your library. It may belong to another Mac. Try another recovery file or set of words.", comment: "Recovery screen: the recovery key is for another library.")),
        (EncryptionCopy.recoveryLostBoth, lostBoth),
        (EncryptionCopy.setupIntro, setupIntro),
        (EncryptionCopy.sameDiskWarning, sameDiskWarning),
        (EncryptionCopy.wordsAreTheKey, wordsAreTheKey),
        (EncryptionCopy.noStoreNotice, noStoreNotice),
        (EncryptionCopy.recordingBlockedNoStore, recordingBlockedNoStore),
        (EncryptionCopy.recordingNotSaved, recordingNotSaved),
        (EncryptionCopy.printFooter, printFooter),
        (EncryptionCopy.unavailableBody, String(localized: "macOS did not let Whimbrel read the key for your library from your Keychain. Your key and your data are most likely still there. This is not the same as a lost key, and you do not need your recovery key for it.", comment: "Launch screen: the Keychain could not be read.")),
        (EncryptionCopy.migratingTitle, String(localized: "Encrypting your library", comment: "Launch screen title while the library is being encrypted.")),
        (EncryptionCopy.migratingBody, String(localized: "This can take a few minutes on a large library. Keep Whimbrel open and your Mac awake until it finishes.", comment: "Launch screen while the library is being encrypted.")),
        (EncryptionCopy.keyStorageBody, String(localized: "Whimbrel keeps the key for your library in your Mac Keychain, and saves it there before it creates the library. macOS did not allow that.", comment: "Launch screen: the key could not be saved in the Keychain.")),
        (EncryptionCopy.interruptedTitle, String(localized: "Encrypting your library was interrupted", comment: "Launch screen title: encryption did not finish.")),
        (EncryptionCopy.damagedBody, String(localized: "Your library file looks damaged, so Whimbrel cannot open it. Whimbrel has not changed it and will not delete or replace it.", comment: "Launch screen: the library file is damaged.")),
        (EncryptionCopy.noAccessBody, String(localized: "macOS did not let Whimbrel read your library file. Whimbrel has not changed it.", comment: "Launch screen: the library file cannot be read.")),
        (EncryptionCopy.openElsewhereBody, String(localized: "Another program has your library file open, so Whimbrel cannot read it. Whimbrel has not changed it.", comment: "Launch screen: another program has the library file open.")),
        (EncryptionCopy.missingTitle, String(localized: "Whimbrel can't find your library", comment: "Launch screen title: the library file is missing.")),
        (EncryptionCopy.missingWithBackupBody, String(localized: "Your encrypted library is missing, but the unencrypted copy from before encryption is still on this Mac. Whimbrel can put that copy back and open it.", comment: "Launch screen: the library is missing and a copy exists.")),
        (EncryptionCopy.missingEncryptedCopyOnlyBody, String(localized: "Your library is missing. Only an unfinished encrypted copy is left, and Whimbrel will not change it.", comment: "Launch screen: the library is missing and only an unfinished copy exists.")),
        (EncryptionCopy.missingNoCopyResetBody, String(localized: "Whimbrel found no library and no copy of it. Reset sets aside Whimbrel's record of the earlier encryption; nothing is deleted.", comment: "Launch screen: the library is missing and there is no copy.")),
        (EncryptionCopy.missingNoCopyHint, String(localized: "If you have a backup of your library, quit Whimbrel, put the backup back, and open Whimbrel again. If not, use Reset. For help, write to hello@whimbrelapp.com.", comment: "Launch screen: what to do when the library and its copy are gone.")),
        (EncryptionCopy.resetConfirmBody, String(localized: "Whimbrel will start with an empty library. Nothing is deleted: only Whimbrel's record of the earlier encryption is set aside. If you can find your library, for example in a backup, quit Whimbrel, put it back, and open Whimbrel again.", comment: "Launch screen: confirmation before Reset.")),
        (EncryptionCopy.continueWithoutStore, String(localized: "Continue without your library", comment: "Launch screen button: open Whimbrel with no library.")),
        (EncryptionCopy.errorStoppedSafely, String(localized: "Whimbrel stopped before opening your library, to keep it safe.", comment: "Launch screen: Whimbrel stopped on purpose.")),
        (EncryptionCopy.errorOpenFailed, String(localized: "Your library could not be opened. Whimbrel has not changed it.", comment: "Launch screen: the library did not open.")),
        (EncryptionCopy.errorFileGoneForRecoveryKey, String(localized: "Whimbrel has your recovery key, but your library file is not there now. Put the file back, then try again. Whimbrel keeps your recovery key until you quit.", comment: "Launch screen: the library file is missing after the recovery key was accepted.")),
        (EncryptionCopy.noteStoreUnavailable, String(localized: "Your library could not be opened.", comment: "Library encryption card, needs attention.")),
        (EncryptionCopy.recordingOffCaption, String(localized: "Recording is off because your library could not be opened.", comment: "Settings, Memory row: why Turn on and recording are unavailable.")),
        (EncryptionCopy.foreignFilesHelp, String(localized: "Files Whimbrel set aside beside your library while undoing or repairing encryption. Whimbrel does not use them.", comment: "Settings, Your data: what the set-aside files are.")),
    ]

    /// Classic sentences that say "memory database" and are not swapped: the launch screens' own "Encrypting" word only. The
    /// no-library notice (menu bar and banner), the two recording-refused messages and the printed recovery page now have
    /// a redesigned sentence each, in `pairs`, and the shared code that draws them calls `map`.
    static let classicOnly: [String] = [EncryptionCopy.encrypting]

    static let noStoreNotice = String(localized: "Your library could not be opened. Recording is off. Restart Whimbrel to try again.", comment: "Notice at the top of the window and the menu bar while Whimbrel runs without its library.")
    static let recordingBlockedNoStore = String(localized: "Whimbrel can't record meetings because your library could not be opened.", comment: "Notification and alert: a recording did not start because the library is not open.")
    static let recordingNotSaved = String(localized: "Whimbrel didn't start recording because the meeting could not be saved to your library. Try again in a moment.", comment: "Notification and alert: a recording did not start because the meeting could not be saved.")
    static let printFooter = String(localized: "Keep this page somewhere safe and offline. Anyone who has it can read the library of the Mac it was made on.", comment: "Footer of the printed recovery page.")

    // The key-setup sheet's sentences (`KeySetupStrings.library`), in place of `EncryptionCopy`'s.
    static let setupIntro = String(localized: "Whimbrel will encrypt your library on this Mac with a key kept in your Mac Keychain. If that key is ever lost, your recovery key is the only way back into your library. Make it now, before anything is encrypted.", comment: "Key setup sheet: what encryption does to the library.")
    static let sameDiskWarning = String(localized: "This file is on the same disk as your library, so it may be lost with it. Keep a copy somewhere other than this Mac as well.", comment: "Key setup sheet: the recovery file is on the same disk as the library.")
    static let wordsAreTheKey = String(localized: "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read your library. Write them down or print them, and keep them somewhere safe and offline.", comment: "Key setup sheet: the 24 words are the key.")
    static let lostBoth = String(localized: "You only need the recovery key if this Mac loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock your library, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.", comment: "Key setup sheet: what happens if every copy of the recovery key is lost.")
    static let encryptContinue = String(localized: "Encrypt your library", comment: "Key setup sheet: the button that starts encrypting the library.")

    /// "Whimbrel got an unexpected answer when opening your library (code 42)." for any code.
    static func unexpected(_ code: Int32) -> String {
        String(localized: "Whimbrel got an unexpected answer when opening your library (code \(Int(code))).", comment: "Library encryption card, needs attention. The placeholder is a number.")
    }

    /// The code in a classic "unexpected answer" sentence, found by what two sentences for different codes have in common,
    /// so it works in every language. Nil when the text is not that sentence.
    static func unexpectedCode(in text: String) -> Int32? {
        let a = EncryptionCopy.attentionUnexpected(10), b = EncryptionCopy.attentionUnexpected(20)
        let head = String(zip(a, b).prefix { $0 == $1 }.map(\.0))
        let suffix = String(String(zip(a.reversed(), b.reversed()).prefix { $0 == $1 }.map(\.0)).reversed())
        let tail = suffix.hasPrefix("0") ? String(suffix.dropFirst()) : suffix
        guard text.hasPrefix(head), text.hasSuffix(tail), text.count > head.count + tail.count else { return nil }
        return Int32(text.dropFirst(head.count).dropLast(tail.count))
    }

    /// The redesigned sentence for a classic one, or the sentence itself when it never said "memory database".
    static func map(_ classic: String) -> String {
        if let found = pairs.first(where: { $0.classic == classic }) { return found.redesigned }
        if let code = unexpectedCode(in: classic) { return unexpected(code) }
        return classic
    }

    static var all: [String] { pairs.map(\.redesigned) + [unexpected(42)] }
}

// MARK: Runtime state

/// The state and the effects behind the section: what the classic panes keep in `@State`, with the same calls.
@MainActor
final class MemoryPrivacyRuntime: ObservableObject {
    unowned let appState: AppState
    init(appState: AppState) { self.appState = appState }

    // Your data
    @Published var counts: CoreDataCounts?
    @Published var dataFolderBytes: Int64 = 0
    @Published var mediaBytes: Int64 = 0
    @Published var usageTotals: CoreUsageTotals?
    @Published var providerRows: [CoreProviderInfo] = CoreMemoryStore.providers()
    @Published var foreignFiles: [CoreForeignFiles.File] = []
    @Published var deletingForeign = false
    @Published var foreignDeleteMessage: String?
    @Published var exporting = false
    @Published var exportStatus: String?
    @Published var exportFolder: URL?
    @Published var deleting = false
    @Published var deleteFailure: String?
    // Keeping data
    @Published var retentionStatus: String?
    @Published var compactStatus: String?
    @Published var footprint: StorageFootprint?
    @Published var lowVolumes: [StorageFootprint.LowVolume] = []
    @Published var compacting = false
    @Published var mediaFolderRefusal: String?
    @Published var orphanScan: CoreOrphanScan?
    @Published var orphanStatus: String?
    @Published var duplicateScan: [CoreMemoryStore.DuplicateMeetingGroup]?
    @Published var duplicateStatus: String?
    @Published var noAudioScan: [CoreMemoryStore.MeetingWithoutAudioSummary]?
    @Published var noAudioStatus: String?
    // Never captured, hotkey
    @Published var apps: [(bundleId: String, name: String)] = []
    @Published var recordingHotkey = false
    @Published var hotkeyRefusal: String?
    private var monitor: Any?

    private func fmt(_ bytes: Int64) -> String { ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file) }

    func appear() {
        apps = CaptureExclusions.shared.candidateApps()
        refreshData()
        measureStorage()
        appState.mediaRetention.refreshPlan()
    }

    func disappear() { stopRecordingHotkey() }

    // MARK: Your data (as YourDataSettingsPane)

    func refreshData() {
        let db = LaunchEnvironment.dbPath(in: SpikePaths.root)
        DispatchQueue.global(qos: .utility).async {
            let files = CoreForeignFiles.list(dbPath: db)
            DispatchQueue.main.async { self.foreignFiles = files }
        }
        guard let store = appState.coreStore else { return }
        let media = YourData.mediaEntriesToDelete(audio: appState.mediaSettings.audioURL(deviceId: appState.deviceId), snapshots: appState.mediaSettings.snapshotsURL)
        CoreMemoryStore.accessQueue.async {
            let c = store.dataCounts()
            let totals = store.usageTotals()
            let bytes = YourData.folderBytes(SpikePaths.root)
            let mBytes = media.reduce(Int64(0)) { $0 + YourData.folderBytes($1) }
            DispatchQueue.main.async {
                self.counts = c
                self.usageTotals = totals
                self.dataFolderBytes = bytes
                self.mediaBytes = mBytes
            }
        }
    }

    func deleteForeignFiles() {
        deletingForeign = true
        let db = LaunchEnvironment.dbPath(in: SpikePaths.root)
        DispatchQueue.global(qos: .userInitiated).async {
            let result = CoreForeignFiles.deleteAll(dbPath: db)
            let left = CoreForeignFiles.list(dbPath: db)
            DispatchQueue.main.async {
                self.deletingForeign = false
                self.foreignDeleteMessage = YourData.foreignFilesDeleteMessage(result)
                self.foreignFiles = left
            }
        }
    }

    func exportEverything() {
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
                self.exporting = false
                if let report {
                    self.exportFolder = URL(fileURLWithPath: report.folder)
                    self.exportStatus = String(localized: "Exported \(report.meetings) meeting(s), \(report.captures) screen capture(s), \(report.emails) email(s) and \(report.conversations) conversation(s): \(report.files) files, \(YourData.formatBytes(report.bytes)).")
                } else {
                    self.exportFolder = nil
                    self.exportStatus = String(localized: "The export failed. Check that the folder can be written to and try again.")
                }
            }
        }
    }

    func chooseVaultFolder() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url.path
    }

    /// The same steps as `YourDataSettingsPane.deleteEverything`, in the same order.
    func deleteEverything(alsoDeleteSyncAccount: Bool, leftovers: @escaping (String) -> Void) async {
        deleting = true
        defer { deleting = false }
        let kept: [String: KeptGoogleAccounts.Connector]? = appState.coreStore == nil ? nil : [
            "calendar": .init(emails: appState.calendarConnectorState.accounts.map(\.email), defaultEmail: appState.calendarConnectorState.defaultEmail),
            "gmail": .init(emails: appState.gmailConnectorState.accounts.map(\.email), defaultEmail: appState.gmailConnectorState.defaultEmail),
        ]
        let keep = KeptGoogleAccounts.keepForDelete(kept, in: SpikePaths.root)
        if keep == .failed {
            deleteFailure = YourData.googleAccountsNotKeptFailure
            return
        }
        let dropKept = {
            if keep == .wrote { try? FileManager.default.removeItem(at: SpikePaths.root.appendingPathComponent(KeptGoogleAccounts.fileName)) }
        }
        if alsoDeleteSyncAccount, case .ready = appState.syncState.status {
            await appState.syncState.deleteSyncAccount()
            if case .ready = appState.syncState.status {
                dropKept()
                deleteFailure = appState.syncState.actionError
                    ?? String(localized: "The sync account could not be deleted. Nothing was removed; try again.")
                return
            }
        }
        appState.captureDidDeleteEverything()
        appState.endActiveMeetingIfNeeded()
        let entries = YourData.entriesToDelete(in: SpikePaths.root)
        do {
            try YourData.scheduleDelete(entries, in: SpikePaths.root)
            let media = YourData.deleteMediaNow(audio: appState.mediaSettings.audioURL(deviceId: appState.deviceId), snapshots: appState.mediaSettings.snapshotsURL)
            leftovers(YourData.mediaDeleteMessage(media) ?? "")
            AppRestart.relaunchAfterExit(waitingAlsoFor: appState.mcpServerProcess.processIdentifier, messages: YourData.restartMessages)
        } catch {
            dropKept()
            deleteFailure = String(localized: "Could not start the deletion: \(error.localizedDescription). Quit Whimbrel and remove the data folder by hand.")
        }
    }

    // MARK: Keeping data (as RetentionSettingsPane)

    func measureStorage() {
        let media = appState.mediaSettings.folderURL
        DispatchQueue.global(qos: .utility).async {
            let measured = StorageFootprint.measure(dataRoot: SpikePaths.root, mediaFolder: media)
            let low = StorageFootprint.lowDiskVolumes(dataRoot: SpikePaths.root, mediaFolder: media)
            DispatchQueue.main.async {
                self.footprint = measured
                self.lowVolumes = low
            }
        }
    }

    func applyRetentionNow() {
        appState.retention.applyNow(store: appState.coreStore, backgroundStore: appState.backgroundStore,
                                    compactAllowed: !appState.audio.micActive && !appState.audio.systemAudioActive) { result in
            guard let result else {
                self.retentionStatus = String(localized: "Retention run failed (store unavailable or DB error).")
                return
            }
            if result.skippedForClientDelete {
                self.retentionStatus = String(localized: "Put off while a client delete runs. Try again shortly.", comment: "Client controls (X6, Task 10).")
                return
            }
            self.retentionStatus = String(localized: "Deleted \(result.captureItemsDeleted) capture item(s), \(result.meetingsDeleted) meeting(s), \(result.conversationsDeleted) conversation(s).")
        }
    }

    func compactNow() {
        guard let store = appState.coreStore else { return }
        compacting = true
        compactStatus = String(localized: "Compacting… this can take a minute.")
        DispatchQueue.global(qos: .userInitiated).async {
            let result = StorageCompaction.run(store, forced: true)
            let free = StorageVolume.writableFreeBytes(at: SpikePaths.root)
            DispatchQueue.main.async {
                self.compacting = false
                self.compactStatus = CompactCopy.message(result, volumeFree: free)
            }
        }
    }

    func chooseMediaFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        mediaFolderRefusal = MediaSettings.refusal(for: url, dataFolder: SpikePaths.root)
        if mediaFolderRefusal == nil { setMediaFolder(url.path) }
    }

    func resetMediaFolder() {
        mediaFolderRefusal = nil
        setMediaFolder(nil)
    }

    private func setMediaFolder(_ path: String?) {
        appState.mediaSettings.folderPath = path
        appState.mediaRetention.run(mode: .moveOnly)
    }

    /// Counts the screen captures a shorter period would delete, the figure Free up space already computes.
    func countOlder(_ kind: WRetentionKind, days: Int, done: @escaping (Int64?) -> Void) {
        guard kind == .screenText, let store = appState.coreStore else { done(nil); return }
        let cutoff = Int64(Date().timeIntervalSince1970 * 1000) - Int64(days) * 86_400_000
        CoreMemoryStore.accessQueue.async {
            let found = store.capturesOlderThan(cutoffUnixMs: cutoff)
            DispatchQueue.main.async { done(found?.count) }
        }
    }

    func setRetention(_ id: String, days: Int?, forever: Bool) {
        let r = appState.retention
        let m = appState.mediaSettings
        switch id {
        case "screenText": if let days { r.captureItemDays = days }; r.captureItemKeepForever = forever
        case "meetings": if let days { r.meetingDays = days }; r.meetingKeepForever = forever
        case "conversations": if let days { r.conversationDays = days }; r.conversationKeepForever = forever
        default: MemoryPrivacyAdapter.applyMedia(id, days: days, forever: forever, to: m)
        }
    }

    // MARK: Clean-ups

    func scan(_ id: String) {
        switch id {
        case "orphans": scanForOrphans()
        case "duplicates": scanForDuplicates()
        default: scanForNoAudio()
        }
    }

    func cleanUp(_ id: String) {
        switch id {
        case "orphans": deleteOrphans()
        case "duplicates": cleanUpDuplicates()
        default: cleanUpNoAudio()
        }
    }

    private func scanForOrphans() {
        let liveWriteURLs = Set([appState.audio.systemAudioFileURL, appState.audio.micAudioFileURL].compactMap { $0 })
        let mediaAudioDir = appState.mediaSettings.audioURL(deviceId: appState.deviceId)
        guard let store = appState.coreStore else { orphanStatus = String(localized: "Scan failed: store unavailable."); return }
        CoreMemoryStore.accessQueue.async {
            guard let result = store.scanOrphanedRecordings(recordingsDir: SpikePaths.root, mediaAudioDir: mediaAudioDir, excluding: liveWriteURLs) else {
                DispatchQueue.main.async { self.orphanStatus = String(localized: "Scan failed: could not read referenced file paths from the database.") }
                return
            }
            DispatchQueue.main.async {
                self.orphanScan = result
                self.orphanStatus = result.orphaned.isEmpty
                    ? String(localized: "No orphaned files found.")
                    : "\(result.orphaned.count) orphaned file(s) found, \(self.fmt(result.totalBytes)) total."
            }
        }
    }

    private func deleteOrphans() {
        guard let result = orphanScan else { return }
        let paths = result.orphaned
        CoreMemoryStore.accessQueue.async {
            let deleted = CoreMemoryStore.deleteOrphanedRecordings(paths)
            DispatchQueue.main.async {
                self.orphanStatus = String(localized: "Deleted \(deleted) file(s).")
                self.orphanScan = nil
            }
        }
    }

    private func scanForDuplicates() {
        guard let store = appState.coreStore else { duplicateStatus = String(localized: "Scan failed: store unavailable."); return }
        let own = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            guard let groups = store.findDuplicateMeetingGroups(ownDeviceId: own) else {
                DispatchQueue.main.async { self.duplicateStatus = String(localized: "Scan failed: could not read meetings from the database.") }
                return
            }
            DispatchQueue.main.async {
                self.duplicateScan = groups
                let extra = groups.reduce(0) { $0 + ($1.meetingIds.count - 1) }
                self.duplicateStatus = groups.isEmpty ? String(localized: "No duplicate meetings found.") : "\(groups.count) duplicate set(s) found, \(extra) extra copy/copies."
            }
        }
    }

    private func cleanUpDuplicates() {
        guard let result = duplicateScan, let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            var deleted = 0
            for group in result {
                guard let keep = group.meetingIds.last else { continue }
                for id in group.meetingIds.dropLast() where store.foldDuplicateMeeting(keep: keep, drop: id) { deleted += 1 }
            }
            DispatchQueue.main.async {
                self.duplicateStatus = String(localized: "Deleted \(deleted) duplicate meeting(s).")
                self.duplicateScan = nil
            }
        }
    }

    private func scanForNoAudio() {
        guard let store = appState.coreStore else { noAudioStatus = String(localized: "Scan failed: store unavailable."); return }
        let own = appState.deviceId
        CoreMemoryStore.accessQueue.async {
            guard let results = store.findMeetingsWithoutAudio(ownDeviceId: own) else {
                DispatchQueue.main.async { self.noAudioStatus = String(localized: "Scan failed: could not read meetings from the database.") }
                return
            }
            DispatchQueue.main.async {
                self.noAudioScan = results
                self.noAudioStatus = results.isEmpty ? String(localized: "No meetings without audio found.") : String(localized: "\(results.count) meeting(s) without audio found.")
            }
        }
    }

    private func cleanUpNoAudio() {
        guard let results = noAudioScan, let store = appState.coreStore else { return }
        MeetingLifecycle.forgetMeetings(store: store, meetingIds: results.map(\.id)) { deleted in
            self.noAudioStatus = String(localized: "Deleted \(deleted) meeting(s).")
            self.noAudioScan = nil
        }
    }

    // MARK: Hotkey (as HotkeyPanelRow)

    func toggleRecordingHotkey() {
        if recordingHotkey { stopRecordingHotkey(); return }
        hotkeyRefusal = nil
        recordingHotkey = true
        let hotkey = appState.hotkeyService
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            if event.keyCode == 53 { self.stopRecordingHotkey(); return nil }
            let candidate = Shortcut(keyCode: UInt32(event.keyCode), modifiers: Shortcut.carbonModifiers(event.modifierFlags))
            let change = hotkey.setShortcut(candidate)
            self.show(change, for: candidate)
            if case .rejected = change {} else { self.stopRecordingHotkey() }
            return nil
        }
    }

    func stopRecordingHotkey() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        recordingHotkey = false
    }

    func resetHotkey() {
        stopRecordingHotkey()
        show(appState.hotkeyService.resetToDefault(), for: HotkeyService.defaultShortcut)
    }

    private func show(_ change: HotkeyService.Change, for candidate: Shortcut) {
        let hotkey = appState.hotkeyService
        switch change {
        case .applied: hotkeyRefusal = nil
        case .rejected(.noModifier):
            hotkeyRefusal = String(localized: "\(candidate.displayString) has no modifier key, so it would block normal typing. Include Control or Option.")
        case .rejected(.needsControlOrOption):
            hotkeyRefusal = String(localized: "\(candidate.displayString) would take a shortcut apps use themselves. Include Control or Option, or use two modifier keys.")
        case .failed(let error):
            let why = error == .alreadyInUse
                ? String(localized: "\(candidate.displayString) is already used by another app.")
                : String(localized: "macOS wouldn't accept \(candidate.displayString).")
            hotkeyRefusal = hotkey.status == .registered ? why + " " + String(localized: "Still using \(hotkey.shortcut.displayString).") : why
        }
    }
}

/// The encryption card's models, made once from the launch's wiring (as `EncryptionSettingsCard`), with their changes forwarded.
/// Empty until the launch has opened the store (the classic window waits for it too).
@MainActor
final class LibraryEncryptionBox: ObservableObject {
    private(set) var live: EncryptionSettingsLive?
    private(set) var model: EncryptionSettingsModel?
    private var cancellables = Set<AnyCancellable>()

    init(live: EncryptionSettingsLive?) {
        attach(live)
    }

    /// Picks up the launch wiring if it arrived after this box was made, so the card appears without leaving the section
    /// and coming back. The classic window waits for the same thing (`EncryptionSettingsPane.live`).
    func attachIfNeeded(_ available: EncryptionSettingsLive? = EncryptionSettingsPane.live) {
        guard live == nil, let available else { return }
        attach(available)
        objectWillChange.send()
    }

    private func attach(_ live: EncryptionSettingsLive?) {
        self.live = live
        guard let live else { model = nil; return }
        let model = EncryptionSettingsModel(outcome: live.outcome, actions: live.actions, pendingNextLaunch: live.pendingNextLaunch, attention: live.attention)
        self.model = model
        model.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        live.notes.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        live.speedCheck.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &cancellables)
        live.notes.$rows.sink { [weak model] rows in
            model?.plainBackupKeptByNote = rows.contains { $0.action == .deletePlainBackup }
            model?.refreshPlainBackup()
        }.store(in: &cancellables)
    }

    var showsSetup: Bool { model?.setupModel != nil }

    func card() -> WLibraryEncryptionModel? {
        guard let live, let model else { return nil }
        let state: WLibraryEncryptionModel.State
        let stateText: String
        var body: [String]
        switch model.state {
        case .notEncrypted:
            state = .notEncrypted; stateText = EncryptionCopy.stateNotEncrypted
            body = [LibraryCopyRedesign.map(EncryptionCopy.settingsNotEncryptedBody)]
        case .encrypted:
            state = .encrypted; stateText = EncryptionCopy.stateEncrypted
            body = [LibraryCopyRedesign.map(EncryptionCopy.settingsEncryptedBody)]
        case .needsAttention(let message):
            state = .needsAttention; stateText = EncryptionCopy.stateNeedsAttention
            body = [LibraryCopyRedesign.map(message)]
        }
        body += [EncryptionCopy.notEncryptedList, EncryptionCopy.notProtectedAgainst]
        let phase: WLibraryEncryptionModel.Phase
        switch model.phase {
        case .idle: phase = .idle(notice: model.notice.map(LibraryCopyRedesign.map), canStart: model.canStartEncrypt)
        case .offeringExport: phase = .offeringExport(exportOffered: model.exportOffered)
        case .checking, .settingUp: phase = .checking
        case .encrypting(let f): phase = .encrypting(fraction: f)
        case .finished(let m): phase = .finished(message: LibraryCopyRedesign.map(m), offersRestart: model.pendingNextLaunch)
        case .failed(let m): phase = .failed(message: LibraryCopyRedesign.map(m))
        }
        let notes = live.notes.rows.map { row in
            WLibraryEncryptionModel.Note(
                id: row.id, lines: row.text.map(LibraryCopyRedesign.map), buttonTitle: row.action.map(EncryptionNotesModel.title),
                feedback: row.action.flatMap { live.notes.feedback[$0] })
        }
        let speed: WLibraryEncryptionModel.SpeedPhase
        switch live.speedCheck.phase {
        case .idle: speed = .idle
        case .running(let f): speed = .running(fraction: f)
        case .done(let lines): speed = .done(lines: lines.map(LibraryCopyRedesign.map))
        case .failed: speed = .failed
        case .blockedByRecording: speed = .blockedByRecording
        }
        return WLibraryEncryptionModel(
            state: state, stateText: stateText, body: body, plainBackupNotice: model.showsPlainBackupNotice ? EncryptionCopy.plainBackupNotice : nil,
            phase: phase, notes: notes, notesBusy: live.notes.cleanupRunning, fastSearch: live.showsStoreTools ? live.fastSearch?.enabled : nil,
            offersSpeedCheck: live.showsStoreTools, speed: speed, canCheckSpeed: live.speedCheck.canCheck, savedReportPath: live.speedCheck.savedPath)
    }

    func actions() -> WLibraryEncryptionActions {
        guard let live, let model else { return WLibraryEncryptionActions() }
        return WLibraryEncryptionActions(
            startEncrypt: { model.startEncrypt() }, exportFirst: { model.exportEverything() },
            continueToSetup: { Task { await model.continueToSetup() } }, cancel: { model.cancel() },
            dismissFailure: { model.dismissResult() }, restart: { model.restart() },
            performNote: { id in if let row = live.notes.rows.first(where: { $0.id == id }), let action = row.action { live.notes.perform(action) } },
            setFastSearch: { on in live.fastSearch?.enabled = on },
            checkSpeed: { Task { await live.speedCheck.check() } }, cancelSpeed: { live.speedCheck.cancel() },
            saveReport: { live.speedCheck.saveReport() })
    }
}

// MARK: The view

struct MemoryPrivacySettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let openAISettings: () -> Void
    let highlighted: String?

    var body: some View {
        MemoryPrivacySettingsRedesignBody(appState: appState, openAISettings: openAISettings, highlighted: highlighted)
    }
}

private struct MemoryPrivacySettingsRedesignBody: View {
    @ObservedObject var appState: AppState
    let openAISettings: () -> Void
    let highlighted: String?
    @StateObject private var runtime: MemoryPrivacyRuntime
    @StateObject private var freeUp: FreeUpRedesignModel
    @StateObject private var box: LibraryEncryptionBox
    @ObservedObject private var exclusions = CaptureExclusions.shared
    @ObservedObject private var retention: RetentionSettings
    @ObservedObject private var media: MediaSettings
    @ObservedObject private var mediaRetention: MediaRetentionRunner
    @ObservedObject private var hotkey: HotkeyService
    @AppStorage(YourData.deleteLeftoversKey) private var deleteLeftovers = ""
    @AppStorage(VaultSettings.enabledKey) private var vaultEnabled = false
    @AppStorage(VaultSettings.pathKey) private var vaultPath = ""
    @AppStorage(VaultSettings.lastUpdatedKey) private var vaultLastUpdated = 0.0
    @AppStorage(VaultSettings.failedKey) private var vaultFailed = false
    @State private var tickAnchor = Date(timeIntervalSince1970: (Date().timeIntervalSince1970 / 60).rounded(.down) * 60)

    init(appState: AppState, openAISettings: @escaping () -> Void, highlighted: String?) {
        _appState = ObservedObject(wrappedValue: appState)
        self.openAISettings = openAISettings
        self.highlighted = highlighted
        _runtime = StateObject(wrappedValue: MemoryPrivacyRuntime(appState: appState))
        _freeUp = StateObject(wrappedValue: FreeUpRedesignModel(appState: appState))
        _box = StateObject(wrappedValue: LibraryEncryptionBox(live: EncryptionSettingsPane.live))
        _retention = ObservedObject(wrappedValue: appState.retention)
        _media = ObservedObject(wrappedValue: appState.mediaSettings)
        _mediaRetention = ObservedObject(wrappedValue: appState.mediaRetention)
        _hotkey = ObservedObject(wrappedValue: appState.hotkeyService)
    }

    /// While memory is on, not capturing and inside its hold, the moment the hold ends: the Memory row redraws then, so "Memory on"
    /// becomes "Memory on, but not capturing" at once rather than at the next minute tick (the menu and Home do the same).
    private var memoryHoldEnd: Date? {
        guard appState.activityCaptureEnabled else { return nil }
        return WMemoryHold.holdEnds(capturing: appState.screenText.isRunning, since: appState.memoryOnSince, now: Date())
    }

    var body: some View {
        SwiftUI.TimelineView(WRedrawSchedule(anchor: tickAnchor, interval: 60, extra: memoryHoldEnd)) { context in content(now: context.date) }
            .onAppear { runtime.appear() }
            .onDisappear { runtime.disappear() }
            // The encryption card appears as soon as the launch has opened the store, if that happens while Settings is open.
            .task {
                while box.live == nil, !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 500_000_000)
                    box.attachIfNeeded()
                }
            }
            // "Export everything…" on the encryption card runs this section's export.
            .onChange(of: appState.pendingYourDataExport) { _, wanted in
                guard wanted else { return }
                appState.pendingYourDataExport = false
                runtime.exportEverything()
            }
            .onChange(of: media.policy) { _, _ in appState.mediaRetention.refreshPlan() }
            .onChange(of: mediaRetention.progress == nil) { _, idle in if idle { appState.mediaRetention.refreshPlan() } }
            .sheet(isPresented: Binding(get: { box.showsSetup }, set: { if !$0 { box.model?.cancel() } })) { setupSheet }
            .sheet(isPresented: Binding(get: { appState.showFreeUpSpace }, set: { appState.showFreeUpSpace = $0; if !$0 { runtime.measureStorage() } })) {
                WFreeUpSpaceSheet(model: freeUp.model(), labels: MemoryPrivacyAdapter.labels(), actions: freeUp.actions { appState.showFreeUpSpace = false })
                    .padding(16)
                    .settingsRedesignThemed()
                    .onAppear { freeUp.refresh() }
            }
    }

    @ViewBuilder
    private var setupSheet: some View {
        if let model = box.model, let setup = model.setupModel {
            KeySetupRedesignView(model: setup, leave: .init(title: EncryptionCopy.cancel, isEscape: true, isEnabled: true, run: { model.cancel() }))
                .frame(width: 680, height: 720)
        }
    }

    private func content(now: Date) -> some View {
        let memory = HomeRedesignAdapter.memoryState(
            enabled: appState.activityCaptureEnabled, resumeAt: appState.captureResumeAt, now: now, capturing: appState.screenText.isRunning, since: appState.memoryOnSince)
        let noStoreShown = appState.recordingGate.showsNotice
        let controls = appState.recordingControls
        let l = MemoryPrivacyAdapter.labels()
        let model = WMemoryPrivacyModel(
            labels: l, memory: memory,
            memoryReason: MemoryPrivacyAdapter.memoryReason(memory: memory, accessibilityGranted: appState.permissions.accessibilityGranted, noStoreShown: noStoreShown),
            memoryDisabledReason: controls.caption.map(LibraryCopyRedesign.map), storeOpen: controls.enabled, showScreenCaptures: appState.showScreenCaptures,
            apps: runtime.apps.map { WExcludedApp(id: $0.bundleId, name: $0.name, excluded: exclusions.isExcluded($0.bundleId)) },
            sites: Array(exclusions.excludedDomains),
            localOnlyLine: LocalOnlyLine.text(localOnly: appState.routing.localOnlyMode),
            counts: runtime.counts.map(countItems),
            dataFolderValue: YourData.formatBytes(runtime.dataFolderBytes),
            mediaFolderValue: "\(appState.mediaSettings.folderURL.path), \(YourData.formatBytes(runtime.mediaBytes))",
            foreign: YourData.foreignFilesSummary(runtime.foreignFiles).map {
                WForeignFilesModel(summary: $0, help: LibraryCopyRedesign.map(EncryptionCopy.foreignFilesHelp), confirmMessage: YourData.foreignFilesConfirmBody(runtime.foreignFiles), deleting: runtime.deletingForeign)
            },
            foreignMessage: runtime.foreignDeleteMessage, outbound: outbound(),
            vault: WVaultModel(
                enabled: vaultEnabled, path: vaultPath,
                lastUpdated: vaultEnabled && !vaultPath.isEmpty && vaultLastUpdated > 0
                    ? String(localized: "Last updated \(RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: vaultLastUpdated), relativeTo: Date()))") : nil,
                failed: vaultEnabled && !vaultPath.isEmpty && vaultFailed),
            exporting: runtime.exporting, exportStatus: runtime.exportStatus, canShowExportFolder: runtime.exportFolder != nil,
            deleteLeftovers: deleteLeftovers,
            delete: deleteModel(),
            storageTotal: runtime.footprint.map(StorageCopy.total), storageParts: runtime.footprint.map(StorageCopy.parts),
            lowDisk: runtime.lowVolumes.map(StorageCopy.lowDisk), canFreeUp: appState.coreStore != nil,
            retention: MemoryPrivacyAdapter.retentionModels(
                captureDays: retention.captureItemDays, captureForever: retention.captureItemKeepForever, meetingDays: retention.meetingDays,
                meetingForever: retention.meetingKeepForever, conversationDays: retention.conversationDays, conversationForever: retention.conversationKeepForever,
                firstAnchor: SettingsAnchorID.captureRetention.rawValue),
            retentionStatus: runtime.retentionStatus, compacting: runtime.compacting, canCompact: appState.coreStore != nil, compactStatus: runtime.compactStatus,
            media: mediaModel(), cleanups: cleanups(),
            encryption: box.card(),
            hotkey: WMemoryHotkeyModel(
                keys: hotkeyKeys(), isRecording: runtime.recordingHotkey,
                refusal: runtime.hotkeyRefusal ?? { if case .failed = hotkey.status { return String(localized: "Another app already uses this shortcut") } else { return nil } }(),
                canReset: hotkey.shortcut != HotkeyService.defaultShortcut),
            anchors: SettingsAnchorID.memoryPrivacy)
        return WMemoryPrivacySettingsView(model: model, actions: actions(), highlighted: highlighted)
    }

    /// Each modifier is its own key cap; the key name is the last one.
    private func hotkeyKeys() -> [String] {
        let text = hotkey.shortcut.displayString
        guard let last = text.firstIndex(where: { !"⌃⌥⇧⌘".contains($0) }) else { return text.map(String.init) }
        return text[..<last].map(String.init) + [String(text[last...])]
    }

    private func countItems(_ c: CoreDataCounts) -> [WSettingsValueItem] {
        func item(_ label: String, _ n: Int64) -> WSettingsValueItem { WSettingsValueItem(label: label, value: n.formatted()) }
        return [
            item(String(localized: "Meetings"), c.meetings), item(String(localized: "Transcript lines"), c.transcriptLines),
            item(String(localized: "Screen captures"), c.captures), item(String(localized: "Slides"), c.snapshots), item(String(localized: "Emails"), c.emails),
            item(String(localized: "Calendar events"), c.calendarEvents), item(String(localized: "Email threads"), c.threads),
            item(String(localized: "Activity episodes"), c.episodes), item(String(localized: "Conversations"), c.conversations),
            item(String(localized: "Routine runs"), c.routineRuns), item(String(localized: "Topics"), c.topics),
            item(String(localized: "People and organizations"), c.entities),
        ]
    }

    private func outbound() -> [WOutboundRow] {
        var providers: [String] = []
        if appState.routing.hasKey(for: .anthropic) { providers.append("Anthropic") }
        if appState.routing.hasKey(for: .openai) { providers.append("OpenAI") }
        if appState.routing.hasKey(for: .google) { providers.append("Google") }
        if appState.routing.hasKey(for: .openrouter) { providers.append("OpenRouter") }
        let whimbrelAI = appState.routing.preferredProvider == .whimbrel && appState.routing.hasKey(for: .whimbrel)
        if whimbrelAI { providers.append("Whimbrel AI") }
        var syncOn = false
        if case .ready = appState.syncState.status { syncOn = true }
        let accounts = Set(appState.gmailConnectorState.accounts.map(\.email) + appState.calendarConnectorState.accounts.map(\.email)).sorted()
        let inputs = YourData.Inputs(
            hostedProviders: providers, localOnly: appState.routing.localOnlyMode, syncOn: syncOn, syncDevices: appState.syncState.devices.count,
            googleAccounts: accounts, typesafeOn: KeychainService.load(for: .typesafe)?.isEmpty == false,
            whimbrelAITerms: whimbrelAI ? runtime.providerRows.first(where: { $0.id == "whimbrel" })?.dataTerms?.localizedText : nil)
        return YourData.outbound(inputs).map { row in
            WOutboundRow(
                id: row.id.rawValue, title: row.title, state: row.state, when: row.when, sees: row.sees,
                spend: row.channel == .providers ? YourData.spendLine(runtime.usageTotals) : nil, active: row.active)
        }
    }

    private func deleteModel() -> WDeleteEverythingModel {
        var ready = false
        if case .ready = appState.syncState.status { ready = true }
        return WDeleteEverythingModel(
            body: YourData.deleteSheetBody(dataFolder: YourData.formatBytes(runtime.dataFolderBytes), mediaFolder: appState.mediaSettings.folderURL.path, media: YourData.formatBytes(runtime.mediaBytes)),
            offersSyncAccount: ready,
            syncNoteIfDeleted: [String(localized: "The encrypted copy on the sync server is deleted and every device is signed out of sync. Your other devices keep their own data."), appState.syncState.aiCreditForfeitNote].compactMap { $0 }.joined(separator: " "),
            syncNoteIfKept: String(localized: "Sync is on. This Mac will download everything again from your other devices unless you also delete your sync account."),
            failure: runtime.deleteFailure, deleting: runtime.deleting)
    }

    private func mediaModel() -> WMediaModel {
        let s = appState.mediaSettings
        let plan = appState.mediaRetention.lastPlan
        let progress = appState.mediaRetention.progress
        return MemoryPrivacyAdapter.mediaModel(
            folder: s.folderURL.path, canReset: s.folderPath != nil, refusal: runtime.mediaFolderRefusal,
            compressDays: s.compressAfterDays, compressForever: s.keepFullAudioForever, deleteDays: s.deleteAudioAfterDays, deleteForever: s.neverDeleteAudio,
            shrinkDays: s.shrinkImagesAfterDays, shrinkForever: s.keepFullImagesForever,
            deleteNote: plan.flatMap { MediaRetentionCopy.deleteSwitchNote($0.usage, deleteOn: !s.neverDeleteAudio) },
            usage: plan.map { MediaRetentionCopy.usage($0.usage) }, preview: plan.map { MediaRetentionCopy.preview($0, policy: s.policy) },
            progress: progress.map { MediaRetentionCopy.progress(step: appState.mediaRetention.step, done: $0.done, total: $0.total) },
            fraction: progress.map { $0.total > 0 ? Double($0.done) / Double($0.total) : 0 })
    }

    private func cleanups() -> [WCleanupModel] {
        let orphan = runtime.orphanScan.flatMap { $0.orphaned.isEmpty ? nil : $0 }
        let dups = runtime.duplicateScan.flatMap { $0.isEmpty ? nil : $0 }
        let noAudio = runtime.noAudioScan.flatMap { $0.isEmpty ? nil : $0 }
        return [
            WCleanupModel(
                id: "orphans", title: String(localized: "Orphaned recording files"),
                notice: MemoryPrivacyAdapter.orphanNotice,
                scanTitle: String(localized: "Scan for Orphaned Files"), status: runtime.orphanStatus,
                confirm: orphan.map { r in .init(title: String(localized: "Delete Orphaned Files?"), message: String(localized: "This will permanently delete \(r.orphaned.count) file(s) totaling \(ByteCountFormatter.string(fromByteCount: r.totalBytes, countStyle: .file)). This cannot be undone."), button: String(localized: "Delete \(r.orphaned.count) File(s)")) }),
            WCleanupModel(
                id: "duplicates", title: String(localized: "Duplicate meetings"),
                notice: MemoryPrivacyAdapter.duplicateNotice,
                scanTitle: String(localized: "Scan for Duplicate Meetings"), status: runtime.duplicateStatus,
                confirm: dups.map { g in
                    let extra = g.reduce(0) { $0 + ($1.meetingIds.count - 1) }
                    return .init(title: String(localized: "Delete Duplicate Meetings?"), message: MemoryPrivacyAdapter.duplicateConfirm(extra), button: String(localized: "Delete \(extra) Duplicate(s)"))
                }),
            WCleanupModel(
                id: "noAudio", title: String(localized: "Meetings without audio"),
                notice: MemoryPrivacyAdapter.noAudioNotice,
                scanTitle: String(localized: "Scan for Meetings Without Audio"), status: runtime.noAudioStatus,
                confirm: noAudio.map { r in .init(title: String(localized: "Delete Meetings Without Audio?"), message: String(localized: "This will permanently delete \(r.count) meeting(s) with no recorded audio, along with any transcript/notes/summary they have. This cannot be undone.") + "\n\n" + String(localized: "This removes them from all your devices."), button: String(localized: "Delete \(r.count) Meeting(s)")) }),
        ]
    }

    private func actions() -> WMemoryPrivacyActions {
        WMemoryPrivacyActions(
            pauseMemory: { appState.pauseCapture($0) }, resumeMemory: { appState.resumeCaptureNow() },
            // Deferred, as the menu's toggle is.
            toggleMemory: { Task { @MainActor in appState.toggleActivityCapture() } },
            setShowScreenCaptures: { appState.showScreenCaptures = $0 },
            setAppExcluded: { exclusions.setExcluded($1, for: $0) },
            domainProblem: { CaptureExclusions.normalizedDomain($0) == nil ? "x" : nil },
            addDomain: { exclusions.addExcludedDomain($0) },
            removeDomain: { exclusions.removeExcludedDomain($0) },
            openAISettings: openAISettings,
            showDataFolder: { NSWorkspace.shared.open(SpikePaths.root) },
            openPrivacyPage: { NSWorkspace.shared.open(URL(string: "https://whimbrelapp.com/privacy/#leaves")!) },
            deleteForeignFiles: { runtime.deleteForeignFiles() },
            setVaultEnabled: { vaultEnabled = $0 },
            chooseVaultFolder: {
                guard let path = runtime.chooseVaultFolder() else { return }
                if path != vaultPath { vaultLastUpdated = 0 }
                vaultPath = path
                vaultFailed = false
            },
            exportEverything: { runtime.exportEverything() },
            showExportFolder: { if let url = runtime.exportFolder { NSWorkspace.shared.activateFileViewerSelecting([url]) } },
            dismissDeleteLeftovers: { deleteLeftovers = "" },
            deleteSheetOpened: {
                runtime.deleteFailure = nil
                Task { await appState.syncState.refreshAIBalance() }
            },
            deleteEverything: { also in Task { await runtime.deleteEverything(alsoDeleteSyncAccount: also) { deleteLeftovers = $0 } } },
            openFreeUpSpace: { appState.showFreeUpSpace = true },
            setRetention: { id, days, forever in runtime.setRetention(id, days: days, forever: forever) },
            countOlder: { kind, days, done in runtime.countOlder(kind, days: days, done: done) },
            applyRetentionNow: { runtime.applyRetentionNow() },
            compactDatabase: { runtime.compactNow() },
            chooseMediaFolder: { runtime.chooseMediaFolder() },
            resetMediaFolder: { runtime.resetMediaFolder() },
            cancelMediaRun: { appState.mediaRetention.cancel() },
            scan: { runtime.scan($0) }, cleanUp: { runtime.cleanUp($0) },
            recordHotkey: { runtime.toggleRecordingHotkey() }, stopRecordingHotkey: { runtime.stopRecordingHotkey() }, resetHotkey: { runtime.resetHotkey() },
            encryption: box.actions())
    }
}

// MARK: Free up space (as FreeUpSpaceSheet)

@MainActor
final class FreeUpRedesignModel: ObservableObject {
    unowned let appState: AppState
    @Published var days = FreeUpSpaceModel.defaultDays
    @Published var options: [FreeUpSpaceOption] = []
    @Published var loading = true
    @Published var working = false
    @Published var result: String?
    private var before: Int64?
    private var generation = RefreshGeneration()

    init(appState: AppState) { self.appState = appState }

    static func key(_ kind: FreeUpSpaceOption.Kind) -> String {
        switch kind {
        case .compactDatabase: return "compact"
        case .compressAudio: return "compress"
        case .deleteOldAudio: return "audio"
        case .deleteOldCaptures: return "captures"
        }
    }

    private func title(_ kind: FreeUpSpaceOption.Kind) -> String {
        switch kind {
        case .compactDatabase: String(localized: "Compact database")
        case .compressAudio: String(localized: "Compress recordings")
        case .deleteOldAudio: String(localized: "Delete old recordings")
        case .deleteOldCaptures: String(localized: "Delete old screen captures")
        }
    }

    private func button(_ kind: FreeUpSpaceOption.Kind) -> String {
        switch kind {
        case .compactDatabase: String(localized: "Compact")
        case .compressAudio: String(localized: "Compress all")
        case .deleteOldAudio, .deleteOldCaptures: String(localized: "Delete…")
        }
    }

    private func question(_ kind: FreeUpSpaceOption.Kind) -> String? {
        switch kind {
        case .deleteOldAudio(let d): String(localized: "Delete the audio of meetings older than \(d) days? This can't be undone.")
        case .deleteOldCaptures(let d): String(localized: "Delete screen captures older than \(d) days from this Mac and your other devices? This can't be undone.")
        default: nil
        }
    }

    func model() -> WFreeUpModel {
        WFreeUpModel(
            dayChoices: FreeUpSpaceModel.dayChoices, days: days, loading: loading, working: working,
            options: options.map { WFreeUpOption(id: Self.key($0.kind), title: title($0.kind), detail: $0.detail, buttonTitle: button($0.kind), confirmQuestion: question($0.kind)) },
            result: result)
    }

    func actions(done: @escaping () -> Void) -> WFreeUpActions {
        WFreeUpActions(
            setDays: { [weak self] in self?.days = $0; self?.refresh() },
            run: { [weak self] id in
                guard let self, let option = self.options.first(where: { Self.key($0.kind) == id }) else { return }
                self.run(option.kind)
            },
            done: done)
    }

    func refresh() {
        guard let store = appState.coreStore else { loading = false; return }
        loading = true
        let token = generation.begin()
        let days = days, media = appState.mediaSettings
        let preview = CoreMediaPolicy(compressAfterDays: nil, deleteAudioAfterDays: nil, shrinkImagesAfterDays: nil, deleteAudioPreviewAfterDays: days)
        let busy = appState.mediaRetention.busyMeetingIds()
        DispatchQueue.global(qos: .userInitiated).async {
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            let (space, plan, captures) = CoreMemoryStore.accessQueue.sync {
                (store.dbSpace(), store.mediaPlan(policy: preview, nowUnixMs: nowMs, mediaRoot: media.folderURL, busy: busy),
                 store.capturesOlderThan(cutoffUnixMs: nowMs - Int64(days) * 86_400_000))
            }
            let total = StorageFootprint.measure(dataRoot: SpikePaths.root, mediaFolder: media.folderURL).total
            let built = FreeUpSpaceModel.options(
                space: space, usage: plan?.usage, deletableAudio: { _ in plan?.usage.audioDeletableBytes ?? 0 },
                oldCaptures: { _ in captures ?? (0, 0) }, days: days)
            DispatchQueue.main.async {
                guard self.generation.isCurrent(token) else { return }
                self.options = built
                self.loading = false
                if !self.working { self.before = total }
            }
        }
    }

    private func run(_ kind: FreeUpSpaceOption.Kind) {
        guard let store = appState.coreStore else { return }
        working = true
        result = nil
        switch kind {
        case .compactDatabase:
            compactThenFinish(store, forced: true)
        case .compressAudio:
            runOneOff(CoreMediaPolicy(compressAfterDays: 0, deleteAudioAfterDays: nil, shrinkImagesAfterDays: 0, deleteAudioPreviewAfterDays: nil))
        case .deleteOldAudio(let d):
            runOneOff(CoreMediaPolicy(compressAfterDays: nil, deleteAudioAfterDays: d, shrinkImagesAfterDays: nil, deleteAudioPreviewAfterDays: d))
        case .deleteOldCaptures(let d):
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
            let lane = appState.backgroundStore.map { (store: $0, queue: CoreMemoryStore.backgroundQueue) }
                ?? (store: store, queue: CoreMemoryStore.accessQueue)
            MediaMover.queue.async {
                let deleted = lane.queue.sync {
                    lane.store.applyRetention(captureItemDays: d, meetingDays: nil, conversationDays: nil, nowUnixMs: nowMs, recordingsDir: SpikePaths.root)
                }
                DispatchQueue.main.async { self.compactThenFinish(store, forced: true, retentionFailed: deleted == nil) }
            }
        }
    }

    private func runOneOff(_ policy: CoreMediaPolicy) {
        guard let store = appState.coreStore else { return }
        let runner = appState.mediaRetention
        let target = runner.oneOffRunsFinished + 1
        runner.run(mode: .oneOff(policy))
        Task { @MainActor in
            while runner.oneOffRunsFinished < target { try? await Task.sleep(nanoseconds: 500_000_000) }
            compactThenFinish(store, forced: false)
        }
    }

    private func compactThenFinish(_ store: CoreMemoryStore, forced: Bool, retentionFailed: Bool = false) {
        let media = appState.mediaSettings.folderURL
        let before = before
        DispatchQueue.global(qos: .userInitiated).async {
            let compacted = retentionFailed ? nil : StorageCompaction.run(store, forced: forced)
            let free = StorageVolume.writableFreeBytes(at: SpikePaths.root)
            let after = StorageFootprint.measure(dataRoot: SpikePaths.root, mediaFolder: media).total
            DispatchQueue.main.async {
                self.working = false
                self.result = FreeUpSpaceModel.finishMessage(retentionFailed: retentionFailed, forced: forced, compacted: compacted, volumeFree: free, before: before, after: after)
                self.refresh()
            }
        }
    }
}
