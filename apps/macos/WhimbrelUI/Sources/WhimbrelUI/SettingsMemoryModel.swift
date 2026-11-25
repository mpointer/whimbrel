import Foundation

// The models and actions of the Memory & Privacy section: plain values the app builds from its state, and what each
// control calls. The same structure as the first slice (`SettingsPanes.swift`). The words are in `SettingsMemoryStrings.swift`.

// MARK: Anchors

/// Every search anchor id the section declares. The app fills them from its registry; the view applies each one exactly once.
public struct WMemoryPrivacyAnchors: Equatable, Sendable {
    public var yourData: String
    public var export: String
    public var delete: String
    public var obsidian: String
    public var encryption: String
    public var screenCaptures: String
    public var excludeApps: String
    public var excludeSites: String
    public var hotkey: String
    public var retention: String
    public var media: String
    public var storage: String

    public init(
        yourData: String, export: String, delete: String, obsidian: String, encryption: String, screenCaptures: String,
        excludeApps: String, excludeSites: String, hotkey: String, retention: String, media: String, storage: String
    ) {
        self.yourData = yourData
        self.export = export
        self.delete = delete
        self.obsidian = obsidian
        self.encryption = encryption
        self.screenCaptures = screenCaptures
        self.excludeApps = excludeApps
        self.excludeSites = excludeSites
        self.hotkey = hotkey
        self.retention = retention
        self.media = media
        self.storage = storage
    }

    public var all: [String] {
        [yourData, export, delete, obsidian, encryption, screenCaptures, excludeApps, excludeSites, hotkey, retention, media, storage]
    }
}

// MARK: Pieces

public struct WOutboundRow: Identifiable, Equatable, Sendable {
    public let id: String
    public var title: String
    public var state: String
    public var when: String
    public var sees: String
    /// The last-30-days spend line, on the AI providers row only.
    public var spend: String?
    public var active: Bool
    public init(id: String, title: String, state: String, when: String, sees: String, spend: String? = nil, active: Bool) {
        self.id = id
        self.title = title
        self.state = state
        self.when = when
        self.sees = sees
        self.spend = spend
        self.active = active
    }
}

/// The set-aside files row of Your data; the model carries it only while such files exist.
public struct WForeignFilesModel: Equatable, Sendable {
    public var summary: String
    public var help: String
    public var confirmMessage: String
    public var deleting: Bool
    public init(summary: String, help: String, confirmMessage: String, deleting: Bool = false) {
        self.summary = summary
        self.help = help
        self.confirmMessage = confirmMessage
        self.deleting = deleting
    }
}

public struct WVaultModel: Equatable, Sendable {
    public var enabled: Bool
    public var path: String
    /// "Last updated 3 minutes ago"; nil when there is nothing to say.
    public var lastUpdated: String?
    public var failed: Bool
    public init(enabled: Bool, path: String, lastUpdated: String? = nil, failed: Bool = false) {
        self.enabled = enabled
        self.path = path
        self.lastUpdated = lastUpdated
        self.failed = failed
    }
}

/// What the Delete everything sheet shows.
public struct WDeleteEverythingModel: Equatable, Sendable {
    public var body: String
    /// Sync is on (signed in and ready): the sheet offers "Also delete my sync account".
    public var offersSyncAccount: Bool
    /// What deleting the sync account does, and what happens to sync if it is kept.
    public var syncNoteIfDeleted: String
    public var syncNoteIfKept: String
    public var failure: String?
    public var deleting: Bool
    public init(body: String, offersSyncAccount: Bool = false, syncNoteIfDeleted: String = "", syncNoteIfKept: String = "", failure: String? = nil, deleting: Bool = false) {
        self.body = body
        self.offersSyncAccount = offersSyncAccount
        self.syncNoteIfDeleted = syncNoteIfDeleted
        self.syncNoteIfKept = syncNoteIfKept
        self.failure = failure
        self.deleting = deleting
    }
}

/// One retention row (a limit on a class of data, with its "Keep forever" companion).
public struct WRetentionRowModel: Identifiable, Equatable, Sendable {
    public var kind: WRetentionKind?
    public let id: String
    public var label: String
    public var description: String?
    public var days: Int
    public var keepForever: Bool
    public var foreverTitle: String
    /// Shortening it, or turning "Keep forever" off, deletes data later. Such a row saves nothing until a period is
    /// committed, and asks before a shorter one. Every row asks, because compressing and shrinking also replace the original.
    public var asksBeforeShortening: Bool
    public var anchor: String?
    /// What passing the period does. Compressing audio and shrinking images ask first, like deleting, because both replace the original.
    public var effect: WRetentionEffect

    public init(
        id: String, kind: WRetentionKind?, label: String, description: String? = nil, days: Int, keepForever: Bool, foreverTitle: String,
        asksBeforeShortening: Bool, anchor: String? = nil, effect: WRetentionEffect = .deletes
    ) {
        self.effect = effect
        self.id = id
        self.kind = kind
        self.label = label
        self.description = description
        self.days = days
        self.keepForever = keepForever
        self.foreverTitle = foreverTitle
        self.asksBeforeShortening = asksBeforeShortening
        self.anchor = anchor
    }

    public var stored: WRetentionStored { WRetentionStored(days: days, keepForever: keepForever) }
}

public struct WMediaModel: Equatable, Sendable {
    public var folderPath: String
    public var canReset: Bool
    public var folderRefusal: String?
    public var compress: WRetentionRowModel
    public var deleteAudio: WRetentionRowModel
    public var shrink: WRetentionRowModel
    public var deleteSwitchNote: String?
    public var usage: String?
    public var preview: String?
    /// "Compressing… 3 of 12", while a run is going.
    public var progress: String?
    public var progressFraction: Double?
    public init(
        folderPath: String, canReset: Bool, folderRefusal: String? = nil, compress: WRetentionRowModel, deleteAudio: WRetentionRowModel,
        shrink: WRetentionRowModel, deleteSwitchNote: String? = nil, usage: String? = nil, preview: String? = nil, progress: String? = nil,
        progressFraction: Double? = nil
    ) {
        self.folderPath = folderPath
        self.canReset = canReset
        self.folderRefusal = folderRefusal
        self.compress = compress
        self.deleteAudio = deleteAudio
        self.shrink = shrink
        self.deleteSwitchNote = deleteSwitchNote
        self.usage = usage
        self.preview = preview
        self.progress = progress
        self.progressFraction = progressFraction
    }
}

/// A scan and its clean-up: orphaned recording files, duplicate meetings, meetings without audio.
public struct WCleanupModel: Identifiable, Equatable, Sendable {
    public let id: String
    public var title: String
    public var notice: String
    public var scanTitle: String
    /// "No orphaned files found." and the like.
    public var status: String?
    /// The question asked before the clean-up runs; nil until a scan found something.
    public var confirm: Confirm?

    public struct Confirm: Equatable, Sendable {
        public var title: String
        public var message: String
        public var button: String
        public init(title: String, message: String, button: String) {
            self.title = title
            self.message = message
            self.button = button
        }
    }

    public init(id: String, title: String, notice: String, scanTitle: String, status: String? = nil, confirm: Confirm? = nil) {
        self.id = id
        self.title = title
        self.notice = notice
        self.scanTitle = scanTitle
        self.status = status
        self.confirm = confirm
    }
}

public struct WMemoryHotkeyModel: Equatable, Sendable {
    public var keys: [String]
    public var isRecording: Bool
    public var refusal: String?
    public var canReset: Bool
    public init(keys: [String], isRecording: Bool = false, refusal: String? = nil, canReset: Bool = false) {
        self.keys = keys
        self.isRecording = isRecording
        self.refusal = refusal
        self.canReset = canReset
    }
}

// MARK: Library encryption

public struct WLibraryEncryptionModel: Equatable, Sendable {
    public enum State: Equatable, Sendable { case notEncrypted, encrypted, needsAttention }

    public enum Phase: Equatable, Sendable {
        case idle(notice: String?, canStart: Bool)
        case offeringExport(exportOffered: Bool)
        case checking
        case encrypting(fraction: Double)
        case finished(message: String, offersRestart: Bool)
        case failed(message: String)
    }

    public struct Note: Identifiable, Equatable, Sendable {
        public let id: Int
        public var lines: [String]
        public var buttonTitle: String?
        public var feedback: String?
        public init(id: Int, lines: [String], buttonTitle: String? = nil, feedback: String? = nil) {
            self.id = id
            self.lines = lines
            self.buttonTitle = buttonTitle
            self.feedback = feedback
        }
    }

    public enum SpeedPhase: Equatable, Sendable {
        case idle
        case running(fraction: Double)
        case done(lines: [String])
        case failed
        case blockedByRecording
    }

    public var state: State
    public var stateText: String
    public var body: [String]
    public var plainBackupNotice: String?
    public var phase: Phase
    public var notes: [Note]
    public var notesBusy: Bool
    /// Faster search; nil when this Mac has too little memory to offer it, or without an open library.
    public var fastSearch: Bool?
    public var offersSpeedCheck: Bool
    public var speed: SpeedPhase
    public var canCheckSpeed: Bool
    public var savedReportPath: String?

    public init(
        state: State, stateText: String, body: [String], plainBackupNotice: String? = nil, phase: Phase, notes: [Note] = [], notesBusy: Bool = false,
        fastSearch: Bool? = nil, offersSpeedCheck: Bool = false, speed: SpeedPhase = .idle, canCheckSpeed: Bool = true, savedReportPath: String? = nil
    ) {
        self.state = state
        self.stateText = stateText
        self.body = body
        self.plainBackupNotice = plainBackupNotice
        self.phase = phase
        self.notes = notes
        self.notesBusy = notesBusy
        self.fastSearch = fastSearch
        self.offersSpeedCheck = offersSpeedCheck
        self.speed = speed
        self.canCheckSpeed = canCheckSpeed
        self.savedReportPath = savedReportPath
    }
}

public struct WLibraryEncryptionActions {
    public var startEncrypt: () -> Void
    public var exportFirst: () -> Void
    public var continueToSetup: () -> Void
    public var cancel: () -> Void
    public var dismissFailure: () -> Void
    public var restart: () -> Void
    public var performNote: (Int) -> Void
    public var setFastSearch: (Bool) -> Void
    public var checkSpeed: () -> Void
    public var cancelSpeed: () -> Void
    public var saveReport: () -> Void

    public init(
        startEncrypt: @escaping () -> Void = {}, exportFirst: @escaping () -> Void = {}, continueToSetup: @escaping () -> Void = {},
        cancel: @escaping () -> Void = {}, dismissFailure: @escaping () -> Void = {}, restart: @escaping () -> Void = {},
        performNote: @escaping (Int) -> Void = { _ in }, setFastSearch: @escaping (Bool) -> Void = { _ in },
        checkSpeed: @escaping () -> Void = {}, cancelSpeed: @escaping () -> Void = {}, saveReport: @escaping () -> Void = {}
    ) {
        self.startEncrypt = startEncrypt
        self.exportFirst = exportFirst
        self.continueToSetup = continueToSetup
        self.cancel = cancel
        self.dismissFailure = dismissFailure
        self.restart = restart
        self.performNote = performNote
        self.setFastSearch = setFastSearch
        self.checkSpeed = checkSpeed
        self.cancelSpeed = cancelSpeed
        self.saveReport = saveReport
    }
}

// MARK: Free up space

public struct WFreeUpOption: Identifiable, Equatable, Sendable {
    public let id: String
    public var title: String
    public var detail: String
    public var buttonTitle: String
    /// The question asked first; nil runs it at once.
    public var confirmQuestion: String?
    public init(id: String, title: String, detail: String, buttonTitle: String, confirmQuestion: String? = nil) {
        self.id = id
        self.title = title
        self.detail = detail
        self.buttonTitle = buttonTitle
        self.confirmQuestion = confirmQuestion
    }
}

public struct WFreeUpModel: Equatable, Sendable {
    public var dayChoices: [Int]
    public var days: Int
    public var loading: Bool
    public var working: Bool
    public var options: [WFreeUpOption]
    public var result: String?
    public init(dayChoices: [Int], days: Int, loading: Bool = false, working: Bool = false, options: [WFreeUpOption] = [], result: String? = nil) {
        self.dayChoices = dayChoices
        self.days = days
        self.loading = loading
        self.working = working
        self.options = options
        self.result = result
    }
}

// MARK: The section

public struct WMemoryPrivacyModel {
    public var labels: WMemoryPrivacyLabels

    // Memory
    public var memory: WMemoryState
    /// Why memory is on but nothing is being saved, when it is; the same reasons the menu bar gives.
    public var memoryReason: String?
    /// Why the Memory controls are off (the library is closed).
    public var memoryDisabledReason: String?
    public var storeOpen: Bool
    public var showScreenCaptures: Bool

    // Never captured
    public var apps: [WExcludedApp]
    public var sites: [String]

    // Your data
    public var localOnlyLine: String
    /// nil while the counts load.
    public var counts: [WSettingsValueItem]?
    public var dataFolderValue: String
    public var mediaFolderValue: String
    public var foreign: WForeignFilesModel?
    public var foreignMessage: String?
    public var outbound: [WOutboundRow]
    public var vault: WVaultModel
    public var exporting: Bool
    public var exportStatus: String?
    public var canShowExportFolder: Bool
    public var deleteLeftovers: String?
    public var delete: WDeleteEverythingModel

    // Keeping data
    public var storageTotal: String?
    public var storageParts: String?
    public var lowDisk: [String]
    public var canFreeUp: Bool
    public var retention: [WRetentionRowModel]
    public var retentionStatus: String?
    public var compacting: Bool
    public var canCompact: Bool
    public var compactStatus: String?
    public var media: WMediaModel
    public var cleanups: [WCleanupModel]

    // Library encryption and the hotkey
    public var encryption: WLibraryEncryptionModel?
    public var hotkey: WMemoryHotkeyModel

    public var anchors: WMemoryPrivacyAnchors

    public init(
        labels: WMemoryPrivacyLabels = WMemoryPrivacyLabels(), memory: WMemoryState, memoryReason: String? = nil, memoryDisabledReason: String? = nil,
        storeOpen: Bool = true, showScreenCaptures: Bool, apps: [WExcludedApp], sites: [String], localOnlyLine: String, counts: [WSettingsValueItem]?,
        dataFolderValue: String, mediaFolderValue: String, foreign: WForeignFilesModel? = nil, foreignMessage: String? = nil, outbound: [WOutboundRow],
        vault: WVaultModel, exporting: Bool = false, exportStatus: String? = nil, canShowExportFolder: Bool = false, deleteLeftovers: String? = nil,
        delete: WDeleteEverythingModel, storageTotal: String?, storageParts: String? = nil, lowDisk: [String] = [], canFreeUp: Bool = true,
        retention: [WRetentionRowModel], retentionStatus: String? = nil, compacting: Bool = false, canCompact: Bool = true, compactStatus: String? = nil,
        media: WMediaModel, cleanups: [WCleanupModel], encryption: WLibraryEncryptionModel? = nil, hotkey: WMemoryHotkeyModel, anchors: WMemoryPrivacyAnchors
    ) {
        self.labels = labels
        self.memory = memory
        self.memoryReason = memoryReason
        self.memoryDisabledReason = memoryDisabledReason
        self.storeOpen = storeOpen
        self.showScreenCaptures = showScreenCaptures
        self.apps = apps
        self.sites = sites
        self.localOnlyLine = localOnlyLine
        self.counts = counts
        self.dataFolderValue = dataFolderValue
        self.mediaFolderValue = mediaFolderValue
        self.foreign = foreign
        self.foreignMessage = foreignMessage
        self.outbound = outbound
        self.vault = vault
        self.exporting = exporting
        self.exportStatus = exportStatus
        self.canShowExportFolder = canShowExportFolder
        self.deleteLeftovers = deleteLeftovers
        self.delete = delete
        self.storageTotal = storageTotal
        self.storageParts = storageParts
        self.lowDisk = lowDisk
        self.canFreeUp = canFreeUp
        self.retention = retention
        self.retentionStatus = retentionStatus
        self.compacting = compacting
        self.canCompact = canCompact
        self.compactStatus = compactStatus
        self.media = media
        self.cleanups = cleanups
        self.encryption = encryption
        self.hotkey = hotkey
        self.anchors = anchors
    }
}

public struct WMemoryPrivacyActions {
    // Memory
    public var pauseMemory: (CapturePauseChoice) -> Void
    public var resumeMemory: () -> Void
    public var toggleMemory: () -> Void
    public var setShowScreenCaptures: (Bool) -> Void
    // Never captured
    public var setAppExcluded: (_ bundleId: String, _ excluded: Bool) -> Void
    /// A message when the text is not a site; nil when it is one.
    public var domainProblem: (String) -> String?
    public var addDomain: (String) -> Void
    public var removeDomain: (String) -> Void
    // Your data
    public var openAISettings: () -> Void
    public var showDataFolder: () -> Void
    public var openPrivacyPage: () -> Void
    public var deleteForeignFiles: () -> Void
    public var setVaultEnabled: (Bool) -> Void
    public var chooseVaultFolder: () -> Void
    public var exportEverything: () -> Void
    public var showExportFolder: () -> Void
    public var dismissDeleteLeftovers: () -> Void
    /// Called when the Delete everything sheet opens (the app refreshes the sync balance for the forfeit line).
    public var deleteSheetOpened: () -> Void
    public var deleteEverything: (_ alsoDeleteSyncAccount: Bool) -> Void
    // Keeping data
    public var openFreeUpSpace: () -> Void
    /// Saves a retention setting. `days` nil leaves the stored number as it is.
    public var setRetention: (_ rowID: String, _ days: Int?, _ keepForever: Bool) -> Void
    /// Counts what a shorter period would delete, when the app can; calls back once, with nil when it cannot.
    public var countOlder: (_ kind: WRetentionKind, _ days: Int, _ done: @escaping (Int64?) -> Void) -> Void
    public var applyRetentionNow: () -> Void
    public var compactDatabase: () -> Void
    public var chooseMediaFolder: () -> Void
    public var resetMediaFolder: () -> Void
    public var cancelMediaRun: () -> Void
    public var scan: (_ cleanupID: String) -> Void
    public var cleanUp: (_ cleanupID: String) -> Void
    // Hotkey
    public var recordHotkey: () -> Void
    public var stopRecordingHotkey: () -> Void
    public var resetHotkey: () -> Void
    // Library encryption
    public var encryption: WLibraryEncryptionActions

    public init(
        pauseMemory: @escaping (CapturePauseChoice) -> Void = { _ in }, resumeMemory: @escaping () -> Void = {}, toggleMemory: @escaping () -> Void = {},
        setShowScreenCaptures: @escaping (Bool) -> Void = { _ in }, setAppExcluded: @escaping (String, Bool) -> Void = { _, _ in },
        domainProblem: @escaping (String) -> String? = { _ in nil }, addDomain: @escaping (String) -> Void = { _ in }, removeDomain: @escaping (String) -> Void = { _ in },
        openAISettings: @escaping () -> Void = {}, showDataFolder: @escaping () -> Void = {}, openPrivacyPage: @escaping () -> Void = {},
        deleteForeignFiles: @escaping () -> Void = {}, setVaultEnabled: @escaping (Bool) -> Void = { _ in }, chooseVaultFolder: @escaping () -> Void = {},
        exportEverything: @escaping () -> Void = {}, showExportFolder: @escaping () -> Void = {}, dismissDeleteLeftovers: @escaping () -> Void = {},
        deleteSheetOpened: @escaping () -> Void = {}, deleteEverything: @escaping (Bool) -> Void = { _ in },
        openFreeUpSpace: @escaping () -> Void = {}, setRetention: @escaping (String, Int?, Bool) -> Void = { _, _, _ in },
        countOlder: @escaping (WRetentionKind, Int, @escaping (Int64?) -> Void) -> Void = { _, _, done in done(nil) },
        applyRetentionNow: @escaping () -> Void = {}, compactDatabase: @escaping () -> Void = {}, chooseMediaFolder: @escaping () -> Void = {},
        resetMediaFolder: @escaping () -> Void = {}, cancelMediaRun: @escaping () -> Void = {}, scan: @escaping (String) -> Void = { _ in },
        cleanUp: @escaping (String) -> Void = { _ in }, recordHotkey: @escaping () -> Void = {}, stopRecordingHotkey: @escaping () -> Void = {},
        resetHotkey: @escaping () -> Void = {}, encryption: WLibraryEncryptionActions = WLibraryEncryptionActions()
    ) {
        self.pauseMemory = pauseMemory
        self.resumeMemory = resumeMemory
        self.toggleMemory = toggleMemory
        self.setShowScreenCaptures = setShowScreenCaptures
        self.setAppExcluded = setAppExcluded
        self.domainProblem = domainProblem
        self.addDomain = addDomain
        self.removeDomain = removeDomain
        self.openAISettings = openAISettings
        self.showDataFolder = showDataFolder
        self.openPrivacyPage = openPrivacyPage
        self.deleteForeignFiles = deleteForeignFiles
        self.setVaultEnabled = setVaultEnabled
        self.chooseVaultFolder = chooseVaultFolder
        self.exportEverything = exportEverything
        self.showExportFolder = showExportFolder
        self.dismissDeleteLeftovers = dismissDeleteLeftovers
        self.deleteSheetOpened = deleteSheetOpened
        self.deleteEverything = deleteEverything
        self.openFreeUpSpace = openFreeUpSpace
        self.setRetention = setRetention
        self.countOlder = countOlder
        self.applyRetentionNow = applyRetentionNow
        self.compactDatabase = compactDatabase
        self.chooseMediaFolder = chooseMediaFolder
        self.resetMediaFolder = resetMediaFolder
        self.cancelMediaRun = cancelMediaRun
        self.scan = scan
        self.cleanUp = cleanUp
        self.recordHotkey = recordHotkey
        self.stopRecordingHotkey = stopRecordingHotkey
        self.resetHotkey = resetHotkey
        self.encryption = encryption
    }
}

public struct WFreeUpActions {
    public var setDays: (Int) -> Void
    public var run: (String) -> Void
    public var done: () -> Void
    public init(setDays: @escaping (Int) -> Void = { _ in }, run: @escaping (String) -> Void = { _ in }, done: @escaping () -> Void = {}) {
        self.setDays = setDays
        self.run = run
        self.done = done
    }
}
