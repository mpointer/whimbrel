import Foundation

// The models and actions of the Meetings, AI and Topics sections: plain values the app builds from its state, and what
// each control calls. The same structure as the Memory & Privacy section (`SettingsMemoryModel.swift`). The words are
// in `SettingsMeetingsAIStrings.swift`. A saved key is never part of any model: key rows carry only whether one is saved.

// MARK: Shared pieces

/// A line of status under a row, with an optional button ("Try again", "Start").
public struct WSettingNote: Identifiable, Equatable, Sendable {
    public enum Tone: Equatable, Sendable { case info, problem, warning }
    public let id: String
    public var text: String
    public var tone: Tone
    public var buttonTitle: String?
    public init(id: String, text: String, tone: Tone = .info, buttonTitle: String? = nil) {
        self.id = id
        self.text = text
        self.tone = tone
        self.buttonTitle = buttonTitle
    }
}

/// A model list's status under a provider: loading, an error, or how many models there are, with Refresh Models.
public struct WModelListStatus: Equatable, Sendable {
    public var text: String
    public var isLoading: Bool
    public var isError: Bool
    public init(text: String, isLoading: Bool = false, isError: Bool = false) {
        self.text = text
        self.isLoading = isLoading
        self.isError = isError
    }
}

public struct WPathModel: Identifiable, Equatable, Sendable {
    public var id: String { label }
    public var label: String
    public var path: String
    public var exists: Bool
    public init(label: String, path: String, exists: Bool) {
        self.label = label
        self.path = path
        self.exists = exists
    }
}

// MARK: Meetings

public struct WMeetingsAnchors: Equatable, Sendable {
    public var recordingNotice: String
    public var autoStart: String
    public var liveTranscript: String
    public var heuristic: String
    public var snapshots: String
    public var voices: String
    public var transcription: String
    public var multilingual: String

    public init(recordingNotice: String, autoStart: String, liveTranscript: String, heuristic: String, snapshots: String, voices: String, transcription: String, multilingual: String) {
        self.recordingNotice = recordingNotice
        self.autoStart = autoStart
        self.liveTranscript = liveTranscript
        self.heuristic = heuristic
        self.snapshots = snapshots
        self.voices = voices
        self.transcription = transcription
        self.multilingual = multilingual
    }

    public var all: [String] { [recordingNotice, autoStart, liveTranscript, heuristic, snapshots, voices, transcription, multilingual] }
}

public struct WCalendarChoice: Identifiable, Equatable, Sendable {
    public let id: String
    public var title: String
    public var isOn: Bool
    public init(id: String, title: String, isOn: Bool) {
        self.id = id
        self.title = title
        self.isOn = isOn
    }
}

public struct WVoicePerson: Identifiable, Equatable, Sendable {
    public let id: String
    public var name: String
    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

public enum WAwayModel: Equatable, Sendable {
    case available(enabled: Bool)
    case comingSoon
}

public struct WTranscriptionModel {
    public var qualityOptions: [WChoice]
    public var quality: String
    public var qualityDetail: String
    public var download: WModelDownloadState
    public var downloadText: String
    public var modelFile: WPathModel
    public var multilingual: Bool
    public var statusMessage: String?
    public var segmentation: WPathModel
    public var embedding: WPathModel
    public var diarizationStatus: String?

    public init(
        qualityOptions: [WChoice], quality: String, qualityDetail: String, download: WModelDownloadState = .none, downloadText: String = "",
        modelFile: WPathModel, multilingual: Bool, statusMessage: String? = nil, segmentation: WPathModel, embedding: WPathModel, diarizationStatus: String? = nil
    ) {
        self.qualityOptions = qualityOptions
        self.quality = quality
        self.qualityDetail = qualityDetail
        self.download = download
        self.downloadText = downloadText
        self.modelFile = modelFile
        self.multilingual = multilingual
        self.statusMessage = statusMessage
        self.segmentation = segmentation
        self.embedding = embedding
        self.diarizationStatus = diarizationStatus
    }
}

/// A row of the Recording notice card. The consent sentence is a row of its own, at the size of every other row, so it is
/// never shrunk to a card description.
public enum WNoticeRow: Equatable, Sendable {
    case consent(String)
    case toggle
    case customText
}

public struct WMeetingsModel {
    public var labels: WMeetingsLabels
    // Recording notice and consent
    public var noticeEnabled: Bool
    public var noticeCustomText: String
    /// The wording used while the custom text is blank.
    public var noticeDefaultText: String
    // Starting and stopping
    public var autoStart: Bool
    public var leadMinutes: Int
    public var heuristic: Bool
    public var checkInHours: Int
    public var checkInOptions: [WChoice]
    public var away: WAwayModel
    // What is saved
    public var liveTranscript: Bool
    public var snapshots: Bool
    public var screenRecordingGranted: Bool
    public var readingSize: Int
    // Question alerts
    public var hasNames: Bool
    public var liveQuestion: Bool
    public var liveQuestionEmail: Bool
    public var otherNames: String
    // Voices
    public var rememberVoices: Bool
    public var voices: [WVoicePerson]
    // Calendars
    public var calendars: [WCalendarChoice]
    public var transcription: WTranscriptionModel
    public var anchors: WMeetingsAnchors

    /// The Recording notice card's rows, in order: the consent sentence first, whether the reminder is on or off; then the
    /// switch; then the field for your own wording while the reminder is on.
    public var noticeRows: [WNoticeRow] {
        [.consent(labels.consentNote), .toggle] + (noticeEnabled ? [.customText] : [])
    }

    public init(
        labels: WMeetingsLabels = WMeetingsLabels(), noticeEnabled: Bool, noticeCustomText: String, noticeDefaultText: String, autoStart: Bool,
        leadMinutes: Int, heuristic: Bool, checkInHours: Int, checkInOptions: [WChoice], away: WAwayModel, liveTranscript: Bool, snapshots: Bool,
        screenRecordingGranted: Bool, readingSize: Int, hasNames: Bool, liveQuestion: Bool, liveQuestionEmail: Bool, otherNames: String,
        rememberVoices: Bool, voices: [WVoicePerson] = [], calendars: [WCalendarChoice], transcription: WTranscriptionModel, anchors: WMeetingsAnchors
    ) {
        self.labels = labels
        self.noticeEnabled = noticeEnabled
        self.noticeCustomText = noticeCustomText
        self.noticeDefaultText = noticeDefaultText
        self.autoStart = autoStart
        self.leadMinutes = leadMinutes
        self.heuristic = heuristic
        self.checkInHours = checkInHours
        self.checkInOptions = checkInOptions
        self.away = away
        self.liveTranscript = liveTranscript
        self.snapshots = snapshots
        self.screenRecordingGranted = screenRecordingGranted
        self.readingSize = readingSize
        self.hasNames = hasNames
        self.liveQuestion = liveQuestion
        self.liveQuestionEmail = liveQuestionEmail
        self.otherNames = otherNames
        self.rememberVoices = rememberVoices
        self.voices = voices
        self.calendars = calendars
        self.transcription = transcription
        self.anchors = anchors
    }
}

public struct WMeetingsActions {
    public var setNoticeEnabled: (Bool) -> Void
    public var setNoticeText: (String) -> Void
    public var setAutoStart: (Bool) -> Void
    public var setLeadMinutes: (Int) -> Void
    public var setHeuristic: (Bool) -> Void
    public var setCheckInHours: (Int) -> Void
    public var setAway: (Bool) -> Void
    public var setLiveTranscript: (Bool) -> Void
    public var setSnapshots: (Bool) -> Void
    public var setReadingSize: (Int) -> Void
    public var setLiveQuestion: (Bool) -> Void
    public var setLiveQuestionEmail: (Bool) -> Void
    public var setOtherNames: (String) -> Void
    public var setRememberVoices: (Bool) -> Void
    /// The voices sheet opened: the app loads the list.
    public var voicesOpened: () -> Void
    public var forgetVoice: (String) -> Void
    public var setCalendar: (String, Bool) -> Void
    public var setQuality: (String) -> Void
    public var retryDownload: () -> Void
    public var chooseModelFile: () -> Void
    public var setMultilingual: (Bool) -> Void
    public var chooseSegmentation: () -> Void
    public var chooseEmbedding: () -> Void

    public init(
        setNoticeEnabled: @escaping (Bool) -> Void = { _ in }, setNoticeText: @escaping (String) -> Void = { _ in },
        setAutoStart: @escaping (Bool) -> Void = { _ in }, setLeadMinutes: @escaping (Int) -> Void = { _ in },
        setHeuristic: @escaping (Bool) -> Void = { _ in }, setCheckInHours: @escaping (Int) -> Void = { _ in },
        setAway: @escaping (Bool) -> Void = { _ in }, setLiveTranscript: @escaping (Bool) -> Void = { _ in },
        setSnapshots: @escaping (Bool) -> Void = { _ in }, setReadingSize: @escaping (Int) -> Void = { _ in },
        setLiveQuestion: @escaping (Bool) -> Void = { _ in }, setLiveQuestionEmail: @escaping (Bool) -> Void = { _ in },
        setOtherNames: @escaping (String) -> Void = { _ in }, setRememberVoices: @escaping (Bool) -> Void = { _ in },
        voicesOpened: @escaping () -> Void = {}, forgetVoice: @escaping (String) -> Void = { _ in },
        setCalendar: @escaping (String, Bool) -> Void = { _, _ in }, setQuality: @escaping (String) -> Void = { _ in },
        retryDownload: @escaping () -> Void = {}, chooseModelFile: @escaping () -> Void = {}, setMultilingual: @escaping (Bool) -> Void = { _ in },
        chooseSegmentation: @escaping () -> Void = {}, chooseEmbedding: @escaping () -> Void = {}
    ) {
        self.setNoticeEnabled = setNoticeEnabled
        self.setNoticeText = setNoticeText
        self.setAutoStart = setAutoStart
        self.setLeadMinutes = setLeadMinutes
        self.setHeuristic = setHeuristic
        self.setCheckInHours = setCheckInHours
        self.setAway = setAway
        self.setLiveTranscript = setLiveTranscript
        self.setSnapshots = setSnapshots
        self.setReadingSize = setReadingSize
        self.setLiveQuestion = setLiveQuestion
        self.setLiveQuestionEmail = setLiveQuestionEmail
        self.setOtherNames = setOtherNames
        self.setRememberVoices = setRememberVoices
        self.voicesOpened = voicesOpened
        self.forgetVoice = forgetVoice
        self.setCalendar = setCalendar
        self.setQuality = setQuality
        self.retryDownload = retryDownload
        self.chooseModelFile = chooseModelFile
        self.setMultilingual = setMultilingual
        self.chooseSegmentation = chooseSegmentation
        self.chooseEmbedding = chooseEmbedding
    }
}

// MARK: AI

public struct WAIAnchors: Equatable, Sendable {
    public var localOnly: String
    public var whimbrelAI: String
    public var anthropicKey: String
    public var openaiKey: String
    public var googleKey: String
    public var openrouterKey: String
    public var ollama: String
    public var provider: String
    public var model: String
    public var dailyCap: String
    public var spend: String
    public var usage: String
    public var typesafe: String
    public var prompts: String

    public init(
        localOnly: String, whimbrelAI: String, anthropicKey: String, openaiKey: String, googleKey: String, openrouterKey: String, ollama: String,
        provider: String, model: String, dailyCap: String, spend: String, usage: String, typesafe: String, prompts: String
    ) {
        self.localOnly = localOnly
        self.whimbrelAI = whimbrelAI
        self.anthropicKey = anthropicKey
        self.openaiKey = openaiKey
        self.googleKey = googleKey
        self.openrouterKey = openrouterKey
        self.ollama = ollama
        self.provider = provider
        self.model = model
        self.dailyCap = dailyCap
        self.spend = spend
        self.usage = usage
        self.typesafe = typesafe
        self.prompts = prompts
    }

    public var all: [String] {
        [localOnly, whimbrelAI, anthropicKey, openaiKey, googleKey, openrouterKey, ollama, provider, model, dailyCap, spend, usage, typesafe, prompts]
    }
}

/// A link under a provider's heading ("Get a key", "Data policy"). The app opens the address.
public struct WProviderLink: Identifiable, Equatable, Sendable {
    public var id: String { url }
    public var title: String
    public var url: String
    public init(title: String, url: String) {
        self.title = title
        self.url = url
    }
}

/// One API key row. Only whether a key is saved; the key never reaches a model.
public struct WKeyRowModel: Equatable, Sendable {
    public var title: String
    public var isSaved: Bool
    public var placeholder: String
    public var status: WSettingsStatus?
    public var anchor: String?
    /// The key is saved and the provider's last check said it refuses it. The chip says so instead of "Saved".
    public var refused: Bool
    public init(title: String, isSaved: Bool, placeholder: String = "", status: WSettingsStatus? = nil, anchor: String? = nil, refused: Bool = false) {
        self.refused = refused
        self.title = title
        self.isSaved = isSaved
        self.placeholder = placeholder
        self.status = status
        self.anchor = anchor
    }
}

/// One hosted provider: its key, its notes, its budget, its sync switch and its model list.
public struct WProviderModel: Identifiable, Equatable, Sendable {
    public let id: String
    public var name: String
    public var links: [WProviderLink]
    public var key: WKeyRowModel
    /// Extra line under the key (OpenRouter's note about picking a model).
    public var keyNote: String?
    public var costLine: String?
    public var termsLine: String?
    /// "This month: $12.40 of $25.00 (50%)"; nil until the usage figures load.
    public var monthText: String?
    public var monthFraction: Double?
    public var monthOver: Bool
    public var budgetDollars: Double?
    public var syncOn: Bool
    public var models: WModelListStatus?
    /// Google only: the billing confirmation switch.
    public var billingConfirmed: Bool?

    public init(
        id: String, name: String, links: [WProviderLink] = [], key: WKeyRowModel, keyNote: String? = nil, costLine: String? = nil,
        termsLine: String? = nil, monthText: String? = nil, monthFraction: Double? = nil, monthOver: Bool = false, budgetDollars: Double? = nil,
        syncOn: Bool, models: WModelListStatus? = nil, billingConfirmed: Bool? = nil
    ) {
        self.id = id
        self.name = name
        self.links = links
        self.key = key
        self.keyNote = keyNote
        self.costLine = costLine
        self.termsLine = termsLine
        self.monthText = monthText
        self.monthFraction = monthFraction
        self.monthOver = monthOver
        self.budgetDollars = budgetDollars
        self.syncOn = syncOn
        self.models = models
        self.billingConfirmed = billingConfirmed
    }
}

public struct WCreditPack: Identifiable, Equatable, Sendable {
    public let id: String
    public var title: String
    public init(id: String, title: String) {
        self.id = id
        self.title = title
    }
}

public struct WWhimbrelAIModel: Equatable, Sendable {
    public var signedIn: Bool
    public var allowanceLine: String?
    public var balanceLine: String?
    public var lowWarning: String?
    public var packs: [WCreditPack]
    public var billingError: String?
    public var models: WModelListStatus?
    public var terms: String?
    public init(
        signedIn: Bool, allowanceLine: String? = nil, balanceLine: String? = nil, lowWarning: String? = nil, packs: [WCreditPack] = [],
        billingError: String? = nil, models: WModelListStatus? = nil, terms: String? = nil
    ) {
        self.signedIn = signedIn
        self.allowanceLine = allowanceLine
        self.balanceLine = balanceLine
        self.lowWarning = lowWarning
        self.packs = packs
        self.billingError = billingError
        self.models = models
        self.terms = terms
    }
}

/// The card shown when nothing can answer yet: no local model, no hosted key.
public struct WOnRampModel: Equatable, Sendable {
    public var installOllamaURL: String
    public var geminiKeyURL: String?
    public init(installOllamaURL: String, geminiKeyURL: String?) {
        self.installOllamaURL = installOllamaURL
        self.geminiKeyURL = geminiKeyURL
    }
}

public struct WOllamaModel: Equatable, Sendable {
    public var useLocalModel: Bool
    public var useLocalCaption: String
    public var models: WModelListStatus
    public var missingChoice: String?
    public var remoteGoogleData: Bool
    public init(useLocalModel: Bool, useLocalCaption: String, models: WModelListStatus, missingChoice: String? = nil, remoteGoogleData: Bool) {
        self.useLocalModel = useLocalModel
        self.useLocalCaption = useLocalCaption
        self.models = models
        self.missingChoice = missingChoice
        self.remoteGoogleData = remoteGoogleData
    }
}

public struct WTypeSafeModel: Equatable, Sendable {
    public var key: WKeyRowModel
    public var hasKey: Bool
    public var rankEmail: Bool
    public var rankRejected: Bool
    public var sortTopics: Bool
    public var sortNotes: [WSettingNote]
    public var titleCheck: Bool
    public init(key: WKeyRowModel, hasKey: Bool, rankEmail: Bool, rankRejected: Bool, sortTopics: Bool, sortNotes: [WSettingNote], titleCheck: Bool) {
        self.key = key
        self.hasKey = hasKey
        self.rankEmail = rankEmail
        self.rankRejected = rankRejected
        self.sortTopics = sortTopics
        self.sortNotes = sortNotes
        self.titleCheck = titleCheck
    }
}

/// How the default model is chosen, by which provider is the default.
public enum WDefaultModelControl: Equatable, Sendable {
    case picker(options: [String], selection: String)
    /// Ollama with nothing loaded: type a model name.
    case text(value: String)
    /// Nothing loaded yet and nothing to type.
    case none
}

public struct WRoutingModel: Equatable, Sendable {
    public var providers: [WChoice]
    public var provider: String
    public var model: WDefaultModelControl
    public var unavailableNotes: [String]
    public var userCapCents: Int?
    public var globalCapCents: Int?
    public var spendText: String
    public init(
        providers: [WChoice], provider: String, model: WDefaultModelControl, unavailableNotes: [String] = [], userCapCents: Int?,
        globalCapCents: Int?, spendText: String
    ) {
        self.providers = providers
        self.provider = provider
        self.model = model
        self.unavailableNotes = unavailableNotes
        self.userCapCents = userCapCents
        self.globalCapCents = globalCapCents
        self.spendText = spendText
    }
}

public struct WUsageModel: Equatable, Sendable {
    public var totalsLine: String?
    public var days: [WUsageDayModel]
    public var showsAll: Bool
    public init(totalsLine: String?, days: [WUsageDayModel], showsAll: Bool) {
        self.totalsLine = totalsLine
        self.days = days
        self.showsAll = showsAll
    }
}

public struct WPromptsModel: Equatable, Sendable {
    public var surfaces: [WChoice]
    public var selected: String
    public var text: String
    public var help: String
    public init(surfaces: [WChoice], selected: String, text: String, help: String) {
        self.surfaces = surfaces
        self.selected = selected
        self.text = text
        self.help = help
    }
}

public struct WAIModel {
    public var labels: WAILabels
    public var localOnly: Bool
    public var onRamp: WOnRampModel?
    public var localModel: WLocalModelCard
    public var whimbrel: WWhimbrelAIModel
    public var providers: [WProviderModel]
    /// "Includes your other computers." once the usage figures are there.
    public var includesOtherComputers: Bool
    public var ollama: WOllamaModel
    public var openRouterModel: WDefaultModelControl
    public var cheapFirst: Bool
    public var imageSearchKey: WKeyRowModel
    public var typeSafe: WTypeSafeModel
    public var routing: WRoutingModel
    public var usage: WUsageModel
    public var prompts: WPromptsModel
    public var maxBudgetCents: Int
    public var anchors: WAIAnchors

    public init(
        labels: WAILabels = WAILabels(), localOnly: Bool, onRamp: WOnRampModel? = nil, localModel: WLocalModelCard = .notInstalled, whimbrel: WWhimbrelAIModel,
        providers: [WProviderModel], includesOtherComputers: Bool, openRouterModel: WDefaultModelControl = .none, ollama: WOllamaModel, cheapFirst: Bool, imageSearchKey: WKeyRowModel,
        typeSafe: WTypeSafeModel, routing: WRoutingModel, usage: WUsageModel, prompts: WPromptsModel, maxBudgetCents: Int = 100_000_000, anchors: WAIAnchors
    ) {
        self.labels = labels
        self.localOnly = localOnly
        self.onRamp = onRamp
        self.localModel = localModel
        self.whimbrel = whimbrel
        self.providers = providers
        self.includesOtherComputers = includesOtherComputers
        self.openRouterModel = openRouterModel
        self.ollama = ollama
        self.cheapFirst = cheapFirst
        self.imageSearchKey = imageSearchKey
        self.typeSafe = typeSafe
        self.routing = routing
        self.usage = usage
        self.prompts = prompts
        self.maxBudgetCents = maxBudgetCents
        self.anchors = anchors
    }
}

public struct WAIActions {
    public var setLocalOnly: (Bool) -> Void
    public var removeLocalModel: () -> Void
    public var openURL: (String) -> Void
    /// Saves a key (the text the person typed). The app stores it and checks it; the view never keeps it.
    /// Answers whether the key was saved; the row keeps what was typed when it was not.
    public var saveKey: (_ keyID: String, _ value: String) -> Bool
    public var removeKey: (_ keyID: String) -> Void
    /// A format hint for what is typed right now; nil when it looks right.
    public var keyHint: (_ keyID: String, _ typed: String) -> String?
    public var setSync: (_ providerID: String, Bool) -> Void
    public var refreshModels: (_ id: String) -> Void
    public var setBudget: (_ providerID: String, WNumberCommit) -> Void
    public var setGoogleBilling: (Bool) -> Void
    public var setOpenRouterModel: (String) -> Void
    public var buyCredit: (_ packID: String) -> Void
    public var setUseLocalModel: (Bool) -> Void
    public var setRemoteGoogleData: (Bool) -> Void
    public var setCheapFirst: (Bool) -> Void
    public var setRankEmail: (Bool) -> Void
    public var setSortTopics: (Bool) -> Void
    public var setTitleCheck: (Bool) -> Void
    /// A button on a note line ("sortRetry", "sortStart").
    public var noteAction: (_ noteID: String) -> Void
    public var setProvider: (String) -> Void
    public var setModel: (String) -> Void
    public var setUserCap: (WNumberCommit) -> Void
    public var setGlobalCap: (WNumberCommit) -> Void
    public var refreshSpend: () -> Void
    public var toggleUsageRange: () -> Void
    public var refreshUsage: () -> Void
    public var selectPrompt: (String) -> Void
    public var setPromptText: (String) -> Void
    public var resetPrompt: () -> Void

    public init(
        setLocalOnly: @escaping (Bool) -> Void = { _ in }, removeLocalModel: @escaping () -> Void = {}, openURL: @escaping (String) -> Void = { _ in },
        saveKey: @escaping (String, String) -> Bool = { _, _ in true }, removeKey: @escaping (String) -> Void = { _ in },
        keyHint: @escaping (String, String) -> String? = { _, _ in nil }, setSync: @escaping (String, Bool) -> Void = { _, _ in },
        refreshModels: @escaping (String) -> Void = { _ in }, setBudget: @escaping (String, WNumberCommit) -> Void = { _, _ in },
        setGoogleBilling: @escaping (Bool) -> Void = { _ in }, setOpenRouterModel: @escaping (String) -> Void = { _ in },
        buyCredit: @escaping (String) -> Void = { _ in }, setUseLocalModel: @escaping (Bool) -> Void = { _ in },
        setRemoteGoogleData: @escaping (Bool) -> Void = { _ in }, setCheapFirst: @escaping (Bool) -> Void = { _ in },
        setRankEmail: @escaping (Bool) -> Void = { _ in }, setSortTopics: @escaping (Bool) -> Void = { _ in },
        setTitleCheck: @escaping (Bool) -> Void = { _ in }, noteAction: @escaping (String) -> Void = { _ in },
        setProvider: @escaping (String) -> Void = { _ in }, setModel: @escaping (String) -> Void = { _ in },
        setUserCap: @escaping (WNumberCommit) -> Void = { _ in }, setGlobalCap: @escaping (WNumberCommit) -> Void = { _ in },
        refreshSpend: @escaping () -> Void = {}, toggleUsageRange: @escaping () -> Void = {}, refreshUsage: @escaping () -> Void = {},
        selectPrompt: @escaping (String) -> Void = { _ in }, setPromptText: @escaping (String) -> Void = { _ in }, resetPrompt: @escaping () -> Void = {}
    ) {
        self.setLocalOnly = setLocalOnly
        self.removeLocalModel = removeLocalModel
        self.openURL = openURL
        self.saveKey = saveKey
        self.removeKey = removeKey
        self.keyHint = keyHint
        self.setSync = setSync
        self.refreshModels = refreshModels
        self.setBudget = setBudget
        self.setGoogleBilling = setGoogleBilling
        self.setOpenRouterModel = setOpenRouterModel
        self.buyCredit = buyCredit
        self.setUseLocalModel = setUseLocalModel
        self.setRemoteGoogleData = setRemoteGoogleData
        self.setCheapFirst = setCheapFirst
        self.setRankEmail = setRankEmail
        self.setSortTopics = setSortTopics
        self.setTitleCheck = setTitleCheck
        self.noteAction = noteAction
        self.setProvider = setProvider
        self.setModel = setModel
        self.setUserCap = setUserCap
        self.setGlobalCap = setGlobalCap
        self.refreshSpend = refreshSpend
        self.toggleUsageRange = toggleUsageRange
        self.refreshUsage = refreshUsage
        self.selectPrompt = selectPrompt
        self.setPromptText = setPromptText
        self.resetPrompt = resetPrompt
    }
}

// MARK: Topics

public struct WTopicsAnchors: Equatable, Sendable {
    public var list: String
    public var add: String
    public var names: String
    public init(list: String, add: String, names: String) {
        self.list = list
        self.add = add
        self.names = names
    }
    public var all: [String] { [list, add, names] }
}

public struct WTopicSeedModel: Identifiable, Equatable, Sendable {
    /// The topic's key.
    public let id: String
    public var name: String
    public var aliases: [String]
    public var kind: String?
    public var kindLabel: String?
    public var itemsText: String
    public init(id: String, name: String, aliases: [String], kind: String?, kindLabel: String?, itemsText: String) {
        self.id = id
        self.name = name
        self.aliases = aliases
        self.kind = kind
        self.kindLabel = kindLabel
        self.itemsText = itemsText
    }
}

public struct WTopicSuggestionModel: Identifiable, Equatable, Sendable {
    public let id: String
    public var name: String
    public var itemsText: String
    public init(id: String, name: String, itemsText: String) {
        self.id = id
        self.name = name
        self.itemsText = itemsText
    }
}

public struct WTopicsModel {
    public var labels: WTopicsLabels
    public var error: String?
    public var seeds: [WTopicSeedModel]
    public var kindChoices: [WChoice]
    public var suggestions: [WTopicSuggestionModel]
    public var showsPlaceholderNote: Bool
    public var names: [String]
    public var nameChips: [String]
    public var stepsOn: Bool
    public var stepsDisabledReason: String?
    public var stepsNotes: [WSettingNote]
    public var anchors: WTopicsAnchors

    public init(
        labels: WTopicsLabels = WTopicsLabels(), error: String? = nil, seeds: [WTopicSeedModel], kindChoices: [WChoice], suggestions: [WTopicSuggestionModel],
        showsPlaceholderNote: Bool, names: [String], nameChips: [String], stepsOn: Bool, stepsDisabledReason: String? = nil, stepsNotes: [WSettingNote] = [],
        anchors: WTopicsAnchors
    ) {
        self.labels = labels
        self.error = error
        self.seeds = seeds
        self.kindChoices = kindChoices
        self.suggestions = suggestions
        self.showsPlaceholderNote = showsPlaceholderNote
        self.names = names
        self.nameChips = nameChips
        self.stepsOn = stepsOn
        self.stepsDisabledReason = stepsDisabledReason
        self.stepsNotes = stepsNotes
        self.anchors = anchors
    }
}

public struct WTopicsActions {
    /// Saves the editor: the topic's key (nil when adding), name, aliases and kind.
    public var saveTopic: (_ key: String?, _ name: String, _ aliases: [String], _ kind: String?) -> Void
    public var merge: (_ keep: String, _ absorb: String) -> Void
    public var acceptSuggestion: (_ id: String, _ name: String) -> Void
    public var dismissSuggestion: (_ id: String) -> Void
    public var addName: (String) -> Void
    public var removeName: (String) -> Void
    public var setSteps: (Bool) -> Void
    public var noteAction: (String) -> Void

    public init(
        saveTopic: @escaping (String?, String, [String], String?) -> Void = { _, _, _, _ in }, merge: @escaping (String, String) -> Void = { _, _ in },
        acceptSuggestion: @escaping (String, String) -> Void = { _, _ in }, dismissSuggestion: @escaping (String) -> Void = { _ in },
        addName: @escaping (String) -> Void = { _ in }, removeName: @escaping (String) -> Void = { _ in },
        setSteps: @escaping (Bool) -> Void = { _ in }, noteAction: @escaping (String) -> Void = { _ in }
    ) {
        self.saveTopic = saveTopic
        self.merge = merge
        self.acceptSuggestion = acceptSuggestion
        self.dismissSuggestion = dismissSuggestion
        self.addName = addName
        self.removeName = removeName
        self.setSteps = setSteps
        self.noteAction = noteAction
    }
}
