import Foundation

/// The words of the Memory & Privacy section. English defaults, which the gallery shows; the app fills every field from
/// its string catalog. Text that depends on a number or a name is a closure, so a translation can put the words in its own order.
/// One retention question: what will happen, to what, from which period.
public struct WRetentionAsk: Equatable, Sendable {
    public var effect: WRetentionEffect
    public var kind: WRetentionKind?
    public var name: String
    public var days: Int
    public var replacing: WRetentionDecision.Replaces
    public var count: Int64?
    public init(effect: WRetentionEffect, kind: WRetentionKind?, name: String, days: Int, replacing: WRetentionDecision.Replaces, count: Int64? = nil) {
        self.effect = effect
        self.kind = kind
        self.name = name
        self.days = days
        self.replacing = replacing
        self.count = count
    }
}

public struct WMemoryPrivacyLabels {
    // Memory
    public var memoryHeading = "Memory"
    public var memoryExplainer = "Memory saves the text of the window you are working in, with its title, its app and, in a browser, the page address, so you can search for it and ask about it later. If a window gives almost no text, Whimbrel tries to read it from a picture of the window and keeps only the text. It saves nothing from apps you list under Never captured, or from listed sites in browsers that show Whimbrel the page address."
    public var pause = "Pause"
    public var turnOn = "Turn on"
    public var turnOff = "Turn off"
    public var pausedHint = "Choose Resume now in the Pause menu to start saving again."
    public var showScreenCaptures = "Show screen captures in Library"
    public var showScreenCapturesCaption = "Off by default: the Recent tab shows meetings only. Screen text is still saved and searchable; this only decides whether captures appear in the Recent list."

    // Never captured
    public var neverHeading = "Never captured"
    public var neverIntro = "Apps you list here are not read at all: the check happens before anything is taken from their windows. A site needs the page address first, so its text is read for a moment and dropped before it is saved or sent to text recognition."
    public var appsLabel = "Apps never captured"
    public var appsHelp = "Turn on an app to leave it out."
    public var appsSearchPlaceholder = "Search running apps…"
    public var noRunningApps = "No running apps found."
    public var noMatches = "No matches."
    public var sitesLabel = "Sites never captured"
    public var sitesHelp = "Pages on these sites, and their subdomains, are dropped before anything is saved. Works in browsers that expose the page address."
    public var domainPlaceholder = "example.com"
    public var domainRefused = "That doesn't look like a site. Enter a domain such as example.com."
    public var noSites = "No sites excluded."
    public var add = "Add"
    public var remove = "Remove"
    public var removeNamed: @Sendable (String) -> String = { "Remove \($0)" }
    public var addSiteLabel = "Add a site"

    // Your data
    public var yourDataHeading = "Your data"
    public var yourDataIntro = "What Whimbrel keeps on this Mac, what leaves it, and how to take it with you or remove it."
    public var openAISettings = "Open AI settings"
    public var counting = "Counting…"
    public var onThisMac = "On this Mac"
    public var dataFolder = "Data folder"
    public var mediaFolderLine = "Recordings and slide images"
    public var showInFinder = "Show in Finder"
    public var leavesHeading = "What leaves this Mac"
    public var privacyLink = "Read the full list on the privacy page"
    public var deleteForeign = "Delete these files"
    public var foreignConfirmTitle = "Delete these files?"
    public var foreignConfirmButton = "Delete"
    public var exportHeading = "Export and delete"
    public var exportLabel = "Export everything"
    public var exportButton = "Export everything…"
    public var exportingButton = "Exporting…"
    public var exportNotice = "Meetings with their transcripts as Markdown, screen text, email, calendar, conversations and routines as JSON, and topics as Obsidian notes. Readable without Whimbrel. Recordings and slide images are not exported."
    public var vaultLabel = "Keep an Obsidian vault up to date"
    public var chooseFolder = "Choose folder…"
    public var vaultFolder = "Vault folder"
    public var noFolderChosen = "No folder chosen yet."
    public var vaultFailed = "Couldn't update the vault. Check that the folder exists and Whimbrel can write to it."
    public var deleteLeftoversLead = "Delete everything could not remove these. Quit Whimbrel and remove them by hand:"
    public var dismiss = "Dismiss"
    public var deleteRowLabel = "Delete everything on this Mac"
    public var deleteRowHelp = "Removes your library and this Mac's recordings and slide images, then restarts Whimbrel as new. You are asked to confirm first."
    public var deleteButton = "Delete everything…"
    public var deleteSheetTitle = "Delete everything on this Mac?"
    public var alsoDeleteSync = "Also delete my sync account"
    public var typeDelete = "Type DELETE to confirm."
    public var deleteWord = "DELETE"
    public var cancel = "Cancel"
    public var deleteAndRestart = "Delete and restart"

    // Keeping data
    public var keepingHeading = "Keeping data"
    public var keepingIntro = "Each kind of data has its own period. Retention deletes what is older than the period. It runs by itself shortly after Whimbrel starts and then once a day."
    public var storageMeasuring = "Measuring what Whimbrel uses…"
    public var freeUpLabel = "Free up space"
    public var freeUpButton = "Free Up Space…"
    public var applyNow = "Apply Retention Now"
    public var applyNowHelp = "Deletes what is older than the periods above right now. Whimbrel may then compact the database to give space back."
    public var applyNowNothing = "Every period above is Keep forever, so there is nothing to delete."
    public var compactLabel = "Compact Database"
    public var compactButton = "Compact Database"
    /// "1 day", "30 days". The English default; the app's strings use the catalog's plural forms in every language.
    public static func englishDays(_ n: Int) -> String { n == 1 ? "1 day" : "\(n) days" }
    public var days: @Sendable (Int) -> String = { WMemoryPrivacyLabels.englishDays($0) }
    public var savePeriod: @Sendable (Int) -> String = { "Keep for \(WMemoryPrivacyLabels.englishDays($0))" }
    public var periodNotSaved = "Not saved yet. Choose how long to keep it, then save."
    public var cancelPeriod = "Cancel"
    public var periodProblem: @Sendable (WRetentionDecision.Problem) -> String = { problem in
        switch problem {
        case .empty: return "Enter a number of days. Nothing was changed."
        case .belowMinimum(let m): return "Use \(WMemoryPrivacyLabels.englishDays(m)) or more. Nothing was changed."
        case .aboveMaximum(let m): return "Use \(WMemoryPrivacyLabels.englishDays(m)) or fewer. Nothing was changed."
        }
    }
    public var storedOutOfRange: @Sendable (Int) -> String = { "The saved period is \(WMemoryPrivacyLabels.englishDays($0)), which is outside the allowed range. The row shows the nearest allowed number. Nothing changes until you save a period." }
    public var askTitle: @Sendable (WRetentionAsk) -> String = { ask in
        let d = WMemoryPrivacyLabels.englishDays(ask.days)
        switch ask.effect {
        case .deletes:
            switch ask.kind {
            case .meetings?: return "Delete meetings and recordings older than \(d)?"
            case .conversations?: return "Delete conversations older than \(d)?"
            case .deleteAudio?: return "Delete audio from recordings older than \(d)?"
            case .screenText?, nil: return "Delete screen text older than \(d)?"
            }
        case .recompressesAudio: return "Compress audio older than \(d)?"
        case .shrinksImages: return "Shrink images older than \(d)?"
        }
    }
    /// What the count counts, said as a sentence: "3,904 screen captures are older than 30 days." The count is formatted for
    /// the language; the thing is named for the kind.
    public static func englishCountLine(_ ask: WRetentionAsk, count: Int64) -> String {
        let n = count.formatted(), d = englishDays(ask.days), one = count == 1
        switch ask.kind {
        case .meetings?: return one ? "\(n) meeting is older than \(d)." : "\(n) meetings are older than \(d)."
        case .conversations?: return one ? "\(n) conversation is older than \(d)." : "\(n) conversations are older than \(d)."
        case .deleteAudio?: return one ? "\(n) recording is older than \(d)." : "\(n) recordings are older than \(d)."
        case .screenText?, nil: return one ? "\(n) screen capture is older than \(d)." : "\(n) screen captures are older than \(d)."
        }
    }
    /// Says plainly what will happen to what, so the question makes sense without a count.
    public var askMessage: @Sendable (WRetentionAsk) -> String = { ask in
        var text = ""
        let d = WMemoryPrivacyLabels.englishDays(ask.days)
        if let count = ask.count { text += englishCountLine(ask, count: count) + " " }
        switch (ask.effect, ask.replacing) {
        case (.deletes, .forever):
            switch ask.kind {
            case .meetings?: text += "Meetings and recordings are kept forever today. "
            case .conversations?: text += "Conversations are kept forever today. "
            case .deleteAudio?: text += "Audio from recordings is kept forever today. "
            case .screenText?, nil: text += "Screen text is kept forever today. "
            }
        case (.deletes, .days(let n)):
            let t = WMemoryPrivacyLabels.englishDays(n)
            switch ask.kind {
            case .meetings?: text += "Meetings and recordings are kept for \(t) today. "
            case .conversations?: text += "Conversations are kept for \(t) today. "
            case .deleteAudio?: text += "Audio from recordings is kept for \(t) today. "
            case .screenText?, nil: text += "Screen text is kept for \(t) today. "
            }
        case (.recompressesAudio, .forever): text += "Audio is kept at full quality today. "
        case (.recompressesAudio, .days(let n)): text += "Audio is compressed after \(WMemoryPrivacyLabels.englishDays(n)) today. "
        case (.shrinksImages, .forever): text += "Images are kept at full quality today. "
        case (.shrinksImages, .days(let n)): text += "Images are shrunk after \(WMemoryPrivacyLabels.englishDays(n)) today. "
        }
        switch ask.effect {
        case .recompressesAudio:
            return text + "Audio older than \(d) is re-encoded at lower quality and the original recording is removed. This can't be undone."
        case .shrinksImages:
            return text + "Slide images older than \(d) are made smaller and the full-size image is removed. This can't be undone."
        case .deletes:
            switch ask.kind {
            case .meetings?: return text + "Meetings older than \(d) are deleted, with their transcripts, recordings and slide images, the next time retention runs. This can't be undone."
            case .conversations?: return text + "Conversations older than \(d) are deleted, except starred ones, the next time retention runs. This can't be undone."
            case .deleteAudio?: return text + "The audio of meetings older than \(d) is deleted. The meetings and their transcripts stay. This can't be undone."
            case .screenText?, nil: return text + "Screen text older than \(d) is deleted the next time retention runs. This can't be undone."
            }
        }
    }
    public var askConfirm: @Sendable (WRetentionAsk) -> String = { ask in
        let d = WMemoryPrivacyLabels.englishDays(ask.days)
        switch ask.effect {
        case .deletes: return "Keep \(d) and delete older"
        case .recompressesAudio: return "Compress after \(d)"
        case .shrinksImages: return "Shrink after \(d)"
        }
    }
    public var applyConfirmTitle = "Delete old data now?"
    /// One full sentence per kind that has a period, one line each; a kind kept forever is not listed.
    public var applyConfirmLine: @Sendable (WRetentionApplyPlan.Limit) -> String = { limit in
        let d = WMemoryPrivacyLabels.englishDays(limit.days)
        switch limit.kind {
        case .meetings: return "Meetings and recordings older than \(d) are deleted."
        case .conversations: return "Conversations older than \(d) are deleted, except starred ones."
        case .deleteAudio: return "The audio of meetings older than \(d) is deleted."
        case .screenText: return "Screen text older than \(d) is deleted."
        }
    }
    public var applyConfirmNothing = "Every period is set to Keep forever, so nothing will be deleted."
    public var cantBeUndone = "This can't be undone."
    /// The question's body: a sentence for each kind with a period, then that it cannot be undone.
    public func applyConfirmMessage(_ limits: [WRetentionApplyPlan.Limit]) -> String {
        if limits.isEmpty { return applyConfirmNothing }
        return (limits.map(applyConfirmLine) + [cantBeUndone]).joined(separator: "\n")
    }
    public var applyConfirmButton = "Delete now"
    public var mediaHeading = "Recordings and slide images"
    public var mediaIntro = "Meeting audio and slide snapshots are kept in this folder, in Audio and Snapshots. Network shares work but are not recommended."
    public var mediaFolder = "Media folder"
    public var choose = "Choose…"
    public var reset = "Reset"
    public var cancelRun = "Cancel"
    public var cleanUp = "Clean Up"
    /// Why Clean Up is off before a scan, and after a scan that found nothing.
    public var cleanUpNeedsScan = "Scan first. Clean Up is off until the scan finds something."
    public var cleanUpNothingFound = "The scan found nothing to clean up."
    public var resetDisabled = "Already using the default folder."
    public var cleanupHeading = "Clean up"
    public var cleanupIntro = "Find leftovers and remove them by hand. Each button here asks before it deletes anything. Whimbrel also removes orphaned recording files by itself once a day."

    // Library encryption
    public var encryptionHeading = "Library encryption"
    public var encryptAction = "Encrypt your library…"
    public var exportFirstTitle = "Export everything first?"
    public var exportFirstBody = "Before encrypting, you can export a readable copy of your data. An export is an ordinary folder and is not encrypted, so keep it somewhere safe."
    public var exportAction = "Export everything…"
    public var continueWithoutExport = "Continue without exporting"
    public var continueAfterExport = "Continue"
    public var checkingFirst = "Checking that encryption can start…"
    public var encrypting = "Encrypting your library…"
    public var restart = "Restart Whimbrel"
    public var ok = "OK"
    public var fastSearch = "Faster search (uses more memory)"
    public var fastSearchHelp = "Gives search up to 1 GB of extra memory. Takes effect after you restart Whimbrel, and only applies once your library is encrypted."
    public var speedCheck = "Check encryption speed"
    public var speedChecking = "Checking speed…"
    public var speedFailed = "The speed check could not finish. Try again."
    public var speedRecording = "A meeting is being recorded. Check the speed when it ends."
    public var speedPrivacy = "The report stays on this Mac. It holds timings and the kind of Mac only, no content, and you decide whether to send it."
    public var saveReport = "Save report…"
    public var savedTo: @Sendable (String) -> String = { "Saved to \($0)" }

    // Hotkey
    public var hotkeyHeading = "Hotkey panel"
    public var hotkeyHelp = "Press this shortcut anywhere to ask about the window you are in."
    public var hotkeyLabel = "Shortcut"

    // Free up space
    public var freeUpTitle = "Free up space"
    public var olderThan = "Older than"
    public var nothingToFree = "Nothing to free right now."
    public var done = "Done"
    public var freeUpConfirmButton = "Delete"
    public var working = "Working…"

    public init() {}
}
