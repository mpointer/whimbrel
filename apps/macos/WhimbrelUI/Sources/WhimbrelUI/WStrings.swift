import SwiftUI

/// The words the first run, launch and setup screens draw themselves. Text that belongs to one app
/// screen (titles, paragraphs, button names) arrives in that screen's model instead.
public struct WOnboardingStrings: Sendable {
    // Permission rows
    public var allowed: String
    public var notYet: String
    public var denied: String
    public var allow: String
    public var openSystemSettings: String
    /// "Allow Microphone", the spoken name of the Allow button.
    public var allowNamed: @Sendable (String) -> String
    /// "Open System Settings for Microphone".
    public var openSettingsNamed: @Sendable (String) -> String
    /// The picture caption under the real recording indicator on the first run screen.
    public var recordingCaption: String
    // Actions
    public var moreOptions: String
    // Step line and word list
    /// "Step 2 of 3".
    public var stepOf: @Sendable (Int, Int) -> String
    /// "Word 3 of 24, apple": the spoken form of one recovery word.
    public var wordItem: @Sendable (Int, Int, String) -> String

    public init(
        allowed: String, notYet: String, denied: String, allow: String, openSystemSettings: String,
        allowNamed: @escaping @Sendable (String) -> String,
        openSettingsNamed: @escaping @Sendable (String) -> String,
        recordingCaption: String, moreOptions: String,
        stepOf: @escaping @Sendable (Int, Int) -> String,
        wordItem: @escaping @Sendable (Int, Int, String) -> String
    ) {
        self.allowed = allowed
        self.notYet = notYet
        self.denied = denied
        self.allow = allow
        self.openSystemSettings = openSystemSettings
        self.allowNamed = allowNamed
        self.openSettingsNamed = openSettingsNamed
        self.recordingCaption = recordingCaption
        self.moreOptions = moreOptions
        self.stepOf = stepOf
        self.wordItem = wordItem
    }

    public static let english = WOnboardingStrings(
        allowed: "Allowed", notYet: "Not yet", denied: "Denied", allow: "Allow", openSystemSettings: "Open System Settings",
        allowNamed: { "Allow \($0)" },
        openSettingsNamed: { "Open System Settings for \($0)" },
        recordingCaption: "This is what you see while Whimbrel is recording.",
        moreOptions: "More options",
        stepOf: { "Step \($0) of \($1)" },
        wordItem: { "Word \($0) of \($1), \($2)" }
    )
}

/// The words Home and the app shell draw themselves: the search filters and the sidebar's name.
/// Everything else on Home (titles, times, section labels, button names) arrives in the model.
public struct WHomeStrings: Sendable {
    public var peopleFilter: String
    public var anyPerson: String
    public var appsFilter: String
    public var datesFilter: String
    public var anyApp: String
    public var anyDate: String
    public var today: String
    public var yesterday: String
    public var lastSevenDays: String
    public var customRange: String
    public var rangeFrom: String
    public var rangeTo: String
    public var applyRange: String
    /// Spoken name of the group of filter chips.
    public var searchFilters: String
    /// Spoken name of the sidebar's list of sections.
    public var sections: String
    /// The pause choices and Resume now, for the capture state menu.
    public var pauseFifteenMinutes: String
    public var pauseOneHour: String
    public var pauseUntilTomorrow: String
    public var resumeNow: String
    /// The sidebar's list of parts of the day, and the collapsed row for the week before yesterday.
    public var thread: String
    public var lastWeek: String
    /// Spoken state of a collapsed row.
    public var expanded: String
    public var collapsed: String

    public init(
        peopleFilter: String, anyPerson: String, appsFilter: String, datesFilter: String, anyApp: String, anyDate: String, today: String, yesterday: String,
        lastSevenDays: String, customRange: String, rangeFrom: String, rangeTo: String, applyRange: String,
        searchFilters: String, sections: String, pauseFifteenMinutes: String, pauseOneHour: String,
        pauseUntilTomorrow: String, resumeNow: String, thread: String, lastWeek: String, expanded: String, collapsed: String
    ) {
        self.peopleFilter = peopleFilter
        self.anyPerson = anyPerson
        self.appsFilter = appsFilter
        self.datesFilter = datesFilter
        self.anyApp = anyApp
        self.anyDate = anyDate
        self.today = today
        self.yesterday = yesterday
        self.lastSevenDays = lastSevenDays
        self.customRange = customRange
        self.rangeFrom = rangeFrom
        self.rangeTo = rangeTo
        self.applyRange = applyRange
        self.searchFilters = searchFilters
        self.sections = sections
        self.pauseFifteenMinutes = pauseFifteenMinutes
        self.pauseOneHour = pauseOneHour
        self.pauseUntilTomorrow = pauseUntilTomorrow
        self.resumeNow = resumeNow
        self.thread = thread
        self.lastWeek = lastWeek
        self.expanded = expanded
        self.collapsed = collapsed
    }

    public static let english = WHomeStrings(
        peopleFilter: "People", anyPerson: "Any person", appsFilter: "Apps", datesFilter: "Dates", anyApp: "Any app", anyDate: "Any date", today: "Today",
        yesterday: "Yesterday", lastSevenDays: "Last 7 days", customRange: "Custom range", rangeFrom: "From",
        rangeTo: "To", applyRange: "Apply", searchFilters: "Search filters", sections: "Sections",
        pauseFifteenMinutes: "Pause for 15 minutes", pauseOneHour: "Pause for 1 hour",
        pauseUntilTomorrow: "Pause until tomorrow", resumeNow: "Resume now",
        thread: "Thread", lastWeek: "Earlier", expanded: "Expanded", collapsed: "Collapsed"
    )
}

/// The words the live meeting screen and the in-meeting window draw themselves. Names that carry a time are closures.
public struct WLiveStrings: Sendable {
    public var recordingPaused: String
    public var markThisMoment: String
    public var momentsYouMarked: String
    public var audioStaysOnMac: String
    public var tidiesNotes: String
    public var stillPaused: String
    public var showInMeetingWindow: String
    public var audioArriving: String
    public var noAudioNow: String
    public var meetingWindowName: String
    public var askAboutMeeting: String
    public var started: String
    public var notes: String
    public var closeWindow: String
    /// "Moment at 10:12", the spoken name of one marked moment.
    public var momentAt: @Sendable (String) -> String
    /// "Remove moment at 10:12".
    public var removeMomentAt: @Sendable (String) -> String
    /// "Mark this moment" announced after it worked: "Moment marked at 10:12".
    public var momentMarked: @Sendable (String) -> String
    /// The count next to "Moments you marked".
    public var count: @Sendable (Int) -> String

    public init(
        recordingPaused: String, markThisMoment: String, momentsYouMarked: String, audioStaysOnMac: String, tidiesNotes: String,
        stillPaused: String, showInMeetingWindow: String, audioArriving: String, noAudioNow: String, meetingWindowName: String,
        askAboutMeeting: String, started: String, notes: String, closeWindow: String,
        momentAt: @escaping @Sendable (String) -> String, removeMomentAt: @escaping @Sendable (String) -> String,
        momentMarked: @escaping @Sendable (String) -> String, count: @escaping @Sendable (Int) -> String
    ) {
        self.recordingPaused = recordingPaused
        self.markThisMoment = markThisMoment
        self.momentsYouMarked = momentsYouMarked
        self.audioStaysOnMac = audioStaysOnMac
        self.tidiesNotes = tidiesNotes
        self.stillPaused = stillPaused
        self.showInMeetingWindow = showInMeetingWindow
        self.audioArriving = audioArriving
        self.noAudioNow = noAudioNow
        self.meetingWindowName = meetingWindowName
        self.askAboutMeeting = askAboutMeeting
        self.started = started
        self.notes = notes
        self.closeWindow = closeWindow
        self.momentAt = momentAt
        self.removeMomentAt = removeMomentAt
        self.momentMarked = momentMarked
        self.count = count
    }

    public static let english = WLiveStrings(
        recordingPaused: "Recording paused", markThisMoment: "Mark this moment", momentsYouMarked: "Moments you marked",
        audioStaysOnMac: "Audio stays on this Mac.", tidiesNotes: "Whimbrel tidies your notes when you stop.",
        stillPaused: "This recording is still paused.", showInMeetingWindow: "Show in-meeting window",
        audioArriving: "Audio is arriving", noAudioNow: "No audio right now", meetingWindowName: "Whimbrel, in-meeting window",
        askAboutMeeting: "Ask about this meeting", started: "Started", notes: "Notes", closeWindow: "Close in-meeting window",
        momentAt: { "Moment at \($0)" }, removeMomentAt: { "Remove moment at \($0)" },
        momentMarked: { "Moment marked at \($0)" }, count: { "\($0)" }
    )
}

/// The words Library's Find page and its other views draw themselves (docs/ui-redesign/screens/ask.md). Text that belongs to
/// one app screen (the notes under an answer, the messages of a failed ask, the line while asking) arrives in the model.
/// Every field has its English default, so the gallery and the tests need nothing; the app sets each one from its catalog.
public struct WFindStrings: Sendable {
    // Library views
    public var viewSwitchName: String = "Library view"
    public var viewFind: String = "Find"
    public var viewRecent: String = "Recent"
    public var viewEpisodes: String = "Episodes"
    public var viewDigest: String = "Digest"
    // Kinds of result
    public var kindSlide: String = "Slide"
    public var kindEmailThread: String = "Email thread"
    public var kindEpisode: String = "Episode"
    public var kindAutomationRun: String = "Automation run"
    public var kindMeeting: String = "Meeting"
    /// What a transcript moment comes from, in the place an app name takes on other cards.
    public var meetingAudio: String = "Meeting audio"
    // Header of the results
    public var resultCount: @Sendable (Int) -> String = { "Results: \($0)" }
    public var sourceCount: @Sendable (Int) -> String = { $0 == 1 ? "1 source found" : "\($0) sources found" }
    public var moreMayExist: String = "More may exist"
    public var orderName: String = "Order"
    public var orderByDay: String = "By day"
    public var orderBestFirst: String = "Best matches first"
    // The Ask row
    public var askButton: String = "Ask"
    public var askOnThisMac: String = "Ask, on this Mac"
    public var askProvider: @Sendable (String) -> String = { "Ask \($0)" }
    public var askHint: String = "Enter searches. Ask writes an answer from what it finds."
    public var newConversationShortcut: String = "\u{2318}N"
    /// Date templates for the day headings (this year, another year). Notation, not words.
    public static let dayTemplate = "EEEMMMd"
    public static let dayTemplateWithYear = "yMMMEEEd"
    public var askShortcutHint: String = "\u{2318}\u{21A9}"
    // States
    public var emptyTitle: String = "Find anything you worked on"
    public var emptyHelp: String = "Type a few words or a question. Results come from this device: meetings, what was on screen, slides, email threads and episodes."
    public var startersName: String = "Ideas to start with"
    public var searching: String = "Searching"
    public var noResultsTitle: String = "Nothing found"
    public var noResultsHelp: String = "Nothing on this device matches those words. Try fewer words, or a different spelling."
    public var noResultsWithFilters: @Sendable (String) -> String = { "Nothing matches with these filters on: \($0)." }
    public var clearFilters: String = "Clear filters"
    public var searchFailedTitle: String = "The search did not finish"
    public var noMeaningSearch: String = "The search model is not installed, so results match your words only."
    public var answeredOnThisMac: String = "Answered on this Mac."
    /// The same note on an answer opened from a saved conversation: only the saved flag is known, not where.
    public var answeredByLocalModel: String = "Answered by a local model."
    public var stoppedNote: String = "Stopped. Your question is back in the field."
    /// After an ask was held back: the button had said where the answer would come from, and a fresh look at the route says otherwise.
    public var routeChanged: String = "Nothing was sent. The answer would no longer come from where the Ask button said. The button now shows where it would come from. Check it, then ask again."
    public var newConversationNote: String = "You changed the filters, so this is a new conversation."
    // The answer
    public var answerTitle: String = "Answer"
    public var fromMemory: String = "From your work memory"
    public var fromMemoryRange: @Sendable (String) -> String = { "From your work memory, \($0)" }
    public var copyAnswer: String = "Copy answer"
    public var copiedAnswer: String = "Copied"
    public var answersCanBeWrong: String = "Answers can be wrong. Open the sources to check."
    public var answerIn: @Sendable (String) -> String = { "Answer: \($0)" }
    public var yourQuestion: @Sendable (String) -> String = { "Your question: \($0)" }
    public var legacySourcesHeading: String = "Sources"
    // The time line
    public var sourcesHeading: String = "Sources for this answer"
    public var otherResultsHeading: String = "Other results"
    public var mentionsHeading: @Sendable (String) -> String = { "Mentions of \($0)" }
    public var mentionsHelp: String = "Screen captures whose text holds the name. They are listed apart from meetings."
    public var loadMore: String = "Load more"
    public var loadingMore: String = "Loading more"
    // Cards
    public var open: String = "Open"
    public var opensSavedCapture: String = "Opens the saved capture"
    public var opensNoteAt: @Sendable (String) -> String = { "Opens the note at \($0)" }
    public var opensMeeting: String = "Opens the meeting"
    public var opensEpisode: String = "Opens the episode"
    public var opensThread: String = "Opens the thread summary"
    public var noLongerOnDevice: String = "No longer on this device"
    /// A source whose row could not be read just now: not the same as one that is gone.
    public var couldNotRead: String = "Could not read this just now."
    /// The heading of the group those sources sit in on the time line.
    public var couldNotReadHeading: String = "Could not read just now"
    public var showMore: String = "Show more"
    public var showLess: String = "Show less"
    public var spokenBy: @Sendable (String) -> String = { "Spoken by \($0)" }
    public var inMeetingTheyAttended: String = "In a meeting they attended"
    // Follow up and conversations
    public var followUpPlaceholder: String = "Ask a follow-up question"
    public var followUpButton: String = "Ask"
    public var conversationsHeading: String = "Conversations"
    public var showConversations: String = "Show Conversations"
    public var hideConversations: String = "Hide Conversations"
    public var newConversation: String = "New Conversation"
    public var noConversations: String = "No conversations yet."
    public var starConversation: String = "Star conversation"
    public var unstarConversation: String = "Unstar conversation"
    public var renameConversation: String = "Rename"
    public var deleteConversation: String = "Delete"
    /// The reason Delete is off for a conversation that is being answered.
    public var deleteBusyHint: String = "An answer is being written in this conversation. Delete it when the answer is done."
    /// The reason the header's Ask button is off while an answer is being written.
    public var askBusyReason: String = "An answer is being written. Ask again when it is done."
    // Recent, Episodes and Digest
    public var showScreenCaptures: String = "Show screen captures"
    public var showHiddenEpisodes: String = "Show hidden episodes"
    public var hiddenEpisode: String = "Hidden"
    public var openInNewWindow: String = "Open in new window"
    public var forgetMeeting: String = "Forget meeting"
    public var generateDigest: String = "Generate Today's Digest"
    public var regenerateDigest: String = "Regenerate today's digest"
    public var openLoops: String = "Open loops"

    public init() {}

    public static let english = WFindStrings()
}

/// The words the finished note page and its transcript view draw themselves. Text that belongs to one meeting
/// (the title, the notices, a placeholder) arrives in the model instead.
public struct WNoteStrings: Sendable {
    public var note: String
    /// The spoken name of the Note and Transcript switch.
    public var viewSwitch: String
    public var copy: String
    /// The transcript view's copy control, so the page never has two controls both named Copy.
    public var copyTranscript: String
    /// The spoken name of the chevron beside Copy that opens the other formats.
    public var moreCopyFormats: String
    public var copyAsMarkdown: String
    public var copyAsSlack: String
    public var sendRecap: String
    public var edit: String
    public var regenerate: String
    public var yourNotes: String
    public var summary: String
    public var decisions: String
    public var openQuestions: String
    public var actionItems: String
    /// The one line at the top of the margin.
    public var marginIntro: String
    /// Under the margin line on a note that has no sources.
    public var regenerateForSources: String
    /// "Open in transcript at 18:25".
    public var openInTranscriptAt: @Sendable (String) -> String
    /// "Due Thu, Nov 19".
    public var dueOn: @Sendable (String) -> String
    /// The spoken name of the group of speaker lanes.
    public var speakerLanes: String
    /// Spoken once when a transcript line is highlighted: "Showing the line at 18:25".
    public var showingLine: @Sendable (String) -> String
    /// The spoken hint of the lanes: how to move between spans.
    public var lanesHint: String

    public init(
        note: String, viewSwitch: String, copy: String, copyTranscript: String, moreCopyFormats: String, copyAsMarkdown: String, copyAsSlack: String, sendRecap: String, edit: String,
        regenerate: String, yourNotes: String, summary: String, decisions: String, openQuestions: String, actionItems: String,
        marginIntro: String, regenerateForSources: String,
        openInTranscriptAt: @escaping @Sendable (String) -> String, dueOn: @escaping @Sendable (String) -> String,
        speakerLanes: String, showingLine: @escaping @Sendable (String) -> String, lanesHint: String
    ) {
        self.note = note
        self.viewSwitch = viewSwitch
        self.copy = copy
        self.copyTranscript = copyTranscript
        self.moreCopyFormats = moreCopyFormats
        self.copyAsMarkdown = copyAsMarkdown
        self.copyAsSlack = copyAsSlack
        self.sendRecap = sendRecap
        self.edit = edit
        self.regenerate = regenerate
        self.yourNotes = yourNotes
        self.summary = summary
        self.decisions = decisions
        self.openQuestions = openQuestions
        self.actionItems = actionItems
        self.marginIntro = marginIntro
        self.regenerateForSources = regenerateForSources
        self.openInTranscriptAt = openInTranscriptAt
        self.dueOn = dueOn
        self.speakerLanes = speakerLanes
        self.showingLine = showingLine
        self.lanesHint = lanesHint
    }

    public static let english = WNoteStrings(
        note: "Note", viewSwitch: "View", copy: "Copy", copyTranscript: "Copy transcript", moreCopyFormats: "More copy formats", copyAsMarkdown: "Copy as Markdown", copyAsSlack: "Copy as Slack",
        sendRecap: "Send recap", edit: "Edit", regenerate: "Regenerate", yourNotes: "Your notes", summary: "Summary",
        decisions: "Decisions", openQuestions: "Open questions", actionItems: "Action items",
        marginIntro: "Whimbrel wrote the marked lines. Each number points to the moment in the transcript it came from.",
        regenerateForSources: "Regenerate the summary to add sources.",
        openInTranscriptAt: { "Open in transcript at \($0)" }, dueOn: { "Due \($0)" },
        speakerLanes: "Speaker lanes", showingLine: { "Showing the line at \($0)" },
        lanesHint: "Use the arrow keys to move between spans, and Return to open the line."
    )
}

/// Every word the components show or speak, in one value. The package owns no translations: it ships
/// English (`WStrings.english`) and the app hands in a `WStrings` built from its own string catalog
/// (`apps/macos/Whimbrel/UI/WhimbrelUIStrings.swift`) through the environment.
///
/// Text that carries a number, a name or a time is a closure, so the app can use the catalog's plural
/// and format rules. The initializer has no default values on purpose: a new string cannot be added
/// here without the app file failing to compile until it supplies a translation source for it.
///
/// This is the only file in the package, with `WGallery.swift`, that may hold user-facing text.
/// `CopyTests.noUserFacingLiteralsOutsideWStrings` enforces that.
public struct WStrings: Sendable {
    // Recording indicator
    public var recording: String
    public var starting: String
    public var resuming: String
    /// What a screen reader says when a recording is paused. Also the live meeting's "Recording paused".
    public var recordingPaused: String
    public var paused: String
    public var processing: String
    public var off: String
    public var problem: String
    public var pause: String
    public var resume: String
    public var stop: String
    /// "1 hour", "2 hours".
    public var hours: @Sendable (Int) -> String
    public var minutes: @Sendable (Int) -> String
    public var seconds: @Sendable (Int) -> String
    /// Joins a list for speech: "Recording, 14 minutes 8 seconds", "Problem, Microphone not found".
    public var joinDetails: @Sendable ([String]) -> String
    /// Joins the parts of a spoken duration: "14 minutes 8 seconds".
    public var joinDuration: @Sendable ([String]) -> String

    // Capture state
    public var memoryOn: String
    public var memoryOff: String
    /// Memory is on, but screen text is not being saved (a permission is off, or capture is not running).
    public var memoryNotCapturing: String
    /// "Memory paused until 10:52". The argument is a clock time the caller formatted. It says "Memory" because a recording can be
    /// paused on the same screen, and "Paused" alone would be read as that.
    public var pausedUntil: @Sendable (String) -> String
    public var whatIsCaptured: String
    public var opensPauseChoices: String

    // Consent banner. None of these says anyone was told or agreed.
    public var consentMessage: String
    public var copyNotice: String
    public var copied: String
    public var done: String
    public var notDone: String

    // Search and filters
    public var searchPlaceholder: String
    public var clearSearch: String
    /// "Remove filter People".
    public var removeFilter: @Sendable (String) -> String

    // Time line
    public var timeline: String
    public var now: String

    // Notes, sources and transcript
    /// "Whimbrel wrote: <line>", the spoken name of an AI line.
    public var whimbrelWrote: @Sendable (String) -> String
    /// "Source 2".
    public var sourceNumber: @Sendable (Int) -> String
    public var selectsThisSource: String
    public var transcriptMoment: String
    public var seenOnScreen: String
    /// Kind of a source card: an email thread.
    public var sourceEmail: String
    /// Kind of a source card: a routine's result.
    public var sourceRoutineRun: String
    /// The default app name on a source card.
    public var transcript: String
    public var open: String
    /// "Open source at 10:52".
    public var sourceAt: @Sendable (String) -> String
    /// A quotation, with the quote marks the language uses: “text”.
    public var quoted: @Sendable (String) -> String
    public var cited: String
    public var listening: String
    /// The two spoken extras on a transcript line, which follow the line's text.
    public var spokenListening: String
    public var spokenCited: String
    public var liveTranscript: String

    // Memory block, loading, error
    public var memory: String
    public var loading: String
    public var tryAgain: String

    // First run, launch and setup screens
    public var onboarding: WOnboardingStrings

    // Home and the app shell
    public var home: WHomeStrings

    // The live meeting screen and the in-meeting window
    public var live: WLiveStrings

    // The finished note page and its transcript view
    public var note: WNoteStrings

    // Library: Find, Recent, Episodes and Digest
    public var find: WFindStrings

    // The menu bar menu, the small alert windows and the hotkey panel
    public var menu: WMenuStrings

    // The settings parts: nav, rows, secrets, lists, sheets. Defaults to English; the app can leave it out.
    public var settings: WSettingsStrings

    public init(
        recording: String, starting: String, resuming: String, recordingPaused: String, paused: String, processing: String, off: String, problem: String,
        pause: String, resume: String, stop: String,
        hours: @escaping @Sendable (Int) -> String,
        minutes: @escaping @Sendable (Int) -> String,
        seconds: @escaping @Sendable (Int) -> String,
        joinDetails: @escaping @Sendable ([String]) -> String,
        joinDuration: @escaping @Sendable ([String]) -> String,
        memoryOn: String, memoryOff: String, memoryNotCapturing: String,
        pausedUntil: @escaping @Sendable (String) -> String,
        whatIsCaptured: String, opensPauseChoices: String,
        consentMessage: String, copyNotice: String, copied: String, done: String, notDone: String,
        searchPlaceholder: String, clearSearch: String,
        removeFilter: @escaping @Sendable (String) -> String,
        timeline: String, now: String,
        whimbrelWrote: @escaping @Sendable (String) -> String,
        sourceNumber: @escaping @Sendable (Int) -> String,
        selectsThisSource: String, transcriptMoment: String, seenOnScreen: String,
        sourceEmail: String = "Email", sourceRoutineRun: String = "Routine run", transcript: String, open: String,
        sourceAt: @escaping @Sendable (String) -> String,
        quoted: @escaping @Sendable (String) -> String,
        cited: String, listening: String, spokenListening: String, spokenCited: String, liveTranscript: String,
        memory: String, loading: String, tryAgain: String,
        onboarding: WOnboardingStrings,
        home: WHomeStrings,
        live: WLiveStrings,
        note: WNoteStrings,
        find: WFindStrings,
        menu: WMenuStrings,
        settings: WSettingsStrings = .english
    ) {
        self.settings = settings
        self.menu = menu
        self.find = find
        self.live = live
        self.note = note
        self.starting = starting
        self.resuming = resuming
        self.recordingPaused = recordingPaused
        self.recording = recording
        self.paused = paused
        self.processing = processing
        self.off = off
        self.problem = problem
        self.pause = pause
        self.resume = resume
        self.stop = stop
        self.hours = hours
        self.minutes = minutes
        self.seconds = seconds
        self.joinDetails = joinDetails
        self.joinDuration = joinDuration
        self.memoryOn = memoryOn
        self.memoryNotCapturing = memoryNotCapturing
        self.memoryOff = memoryOff
        self.pausedUntil = pausedUntil
        self.whatIsCaptured = whatIsCaptured
        self.opensPauseChoices = opensPauseChoices
        self.consentMessage = consentMessage
        self.copyNotice = copyNotice
        self.copied = copied
        self.done = done
        self.notDone = notDone
        self.searchPlaceholder = searchPlaceholder
        self.clearSearch = clearSearch
        self.removeFilter = removeFilter
        self.timeline = timeline
        self.now = now
        self.whimbrelWrote = whimbrelWrote
        self.sourceNumber = sourceNumber
        self.selectsThisSource = selectsThisSource
        self.transcriptMoment = transcriptMoment
        self.seenOnScreen = seenOnScreen
        self.sourceEmail = sourceEmail
        self.sourceRoutineRun = sourceRoutineRun
        self.transcript = transcript
        self.open = open
        self.sourceAt = sourceAt
        self.quoted = quoted
        self.cited = cited
        self.listening = listening
        self.spokenListening = spokenListening
        self.spokenCited = spokenCited
        self.liveTranscript = liveTranscript
        self.memory = memory
        self.loading = loading
        self.tryAgain = tryAgain
        self.onboarding = onboarding
        self.home = home
    }

    /// US English. What the package shows when the app supplies nothing, and what the gallery and the
    /// tests use.
    public static let english = WStrings(
        recording: "Recording", starting: "Starting", resuming: "Resuming", recordingPaused: "Recording paused", paused: "Paused", processing: "Processing", off: "Not recording", problem: "Problem",
        pause: "Pause", resume: "Resume", stop: "Stop",
        hours: { "\($0) \($0 == 1 ? "hour" : "hours")" },
        minutes: { "\($0) \($0 == 1 ? "minute" : "minutes")" },
        seconds: { "\($0) \($0 == 1 ? "second" : "seconds")" },
        joinDetails: { $0.joined(separator: ", ") },
        joinDuration: { $0.joined(separator: " ") },
        memoryOn: "Memory on", memoryOff: "Memory off", memoryNotCapturing: "Memory on, but not capturing",
        pausedUntil: { "Memory paused until \($0)" },
        whatIsCaptured: "What is captured", opensPauseChoices: "Opens pause choices",
        consentMessage: "You are recording. Tell the others before you go on.",
        copyNotice: "Copy notice", copied: "Copied", done: "Done", notDone: "Not done",
        searchPlaceholder: "Search your work memory", clearSearch: "Clear search",
        removeFilter: { "Remove filter \($0)" },
        timeline: "Timeline", now: "Now",
        whimbrelWrote: { "Whimbrel wrote: \($0)" },
        sourceNumber: { "Source \($0)" },
        selectsThisSource: "Selects this source", transcriptMoment: "Transcript moment", seenOnScreen: "Seen on screen",
        sourceEmail: "Email", sourceRoutineRun: "Routine run",
        transcript: "Transcript", open: "Open",
        sourceAt: { "Open source at \($0)" },
        quoted: { "\u{201C}\($0)\u{201D}" },
        cited: "Cited", listening: "Listening", spokenListening: "listening", spokenCited: "cited",
        liveTranscript: "Live transcript",
        memory: "Memory", loading: "Loading", tryAgain: "Try again",
        onboarding: .english,
        home: .english,
        live: .english,
        note: .english,
        find: .english,
        menu: .english,
        settings: .english
    )
}

/// The words the menu bar menu, the small alert windows and the hotkey panel draw themselves. Text that
/// belongs to one situation (a meeting's title, a permission's reason, a time) arrives in the model instead.
public struct WMenuStrings: Sendable {
    public var appName: String
    /// Spoken name of the menu.
    public var menuName: String
    public var needsAttention: String
    public var nextMeeting: String
    public var workingOn: String
    /// Spoken names of the recording buttons. "Pause" alone would be ambiguous next to the memory pause.
    public var pauseRecording: String
    public var resumeRecording: String
    public var stopRecording: String
    /// "Recording: Pricing review". The first value is the state word, the second the meeting's title. Spoken only.
    public var stateWithTitle: @Sendable (String, String) -> String
    /// The state words of the recording section, which say a recording is open ("Recording paused", not "Paused").
    public var recordingProblem: String
    public var startingRecording: String
    public var resumingRecording: String
    /// Under a start whose audio is already on: the recording is running and Stop works once the meeting row is saved.
    public var stopWaitsForMeeting: String
    /// The line for a problem when the app gave no reason. It claims only what is known.
    public var problemUnknown: String
    /// Which streams are live, under the recording state.
    public var streamsBoth: String
    public var streamsMicrophone: String
    public var streamsSystemAudio: String
    public var streamsNone: String
    /// Spoken name of the memory control while paused: "Memory paused until 3:10 PM".
    public var memoryPausedUntil: @Sendable (String) -> String
    public var turnOnMemory: String
    public var turnOffMemory: String
    public var startMeeting: String
    public var openBrief: String
    /// "Open brief for Pricing review".
    public var openBriefFor: @Sendable (String) -> String
    /// "Open topic Pricing", the spoken name of the topic chip.
    public var openTopic: @Sendable (String) -> String
    public var askAboutWindow: String
    public var openWhimbrel: String
    public var settings: String
    public var quit: String
    // Alert windows
    public var cancel: String
    public var alertWindowName: String
    // Hotkey panel
    public var hotkeyWindowName: String
    public var close: String
    public var copy: String
    public var answering: String
    /// The word on the disabled button that stands where Stop was once a Stop has been asked for and the meeting is still being
    /// saved. The old menu says the same.
    public var saving: String
    /// The quiet row above Open Whimbrel: it copies a short message with a link, so a person can tell a friend. No account needed.
    public var tellAFriend: String
    /// What the row says for a few seconds after the copy worked.
    public var tellAFriendCopied: String
    /// Spoken after the name of the row: what it does. It opens nothing and sends nothing.
    public var tellAFriendHint: String

    public init(
        appName: String, menuName: String, needsAttention: String, nextMeeting: String, workingOn: String,
        pauseRecording: String, resumeRecording: String, stopRecording: String,
        stateWithTitle: @escaping @Sendable (String, String) -> String,
        recordingProblem: String, startingRecording: String, resumingRecording: String, stopWaitsForMeeting: String, problemUnknown: String,
        streamsBoth: String, streamsMicrophone: String, streamsSystemAudio: String, streamsNone: String,
        memoryPausedUntil: @escaping @Sendable (String) -> String,
        turnOnMemory: String, turnOffMemory: String, startMeeting: String, openBrief: String,
        openBriefFor: @escaping @Sendable (String) -> String,
        openTopic: @escaping @Sendable (String) -> String,
        askAboutWindow: String, openWhimbrel: String, settings: String, quit: String,
        cancel: String, alertWindowName: String, hotkeyWindowName: String, close: String, copy: String, answering: String,
        saving: String = "Saving\u{2026}",
        tellAFriend: String = "Tell a friend", tellAFriendCopied: String = "Copied",
        tellAFriendHint: String = "Copies a short message and a link to Whimbrel. Nothing is sent."
    ) {
        self.saving = saving
        self.tellAFriend = tellAFriend
        self.tellAFriendCopied = tellAFriendCopied
        self.tellAFriendHint = tellAFriendHint
        self.appName = appName
        self.menuName = menuName
        self.needsAttention = needsAttention
        self.nextMeeting = nextMeeting
        self.workingOn = workingOn
        self.pauseRecording = pauseRecording
        self.resumeRecording = resumeRecording
        self.stopRecording = stopRecording
        self.stateWithTitle = stateWithTitle
        self.recordingProblem = recordingProblem
        self.startingRecording = startingRecording
        self.resumingRecording = resumingRecording
        self.stopWaitsForMeeting = stopWaitsForMeeting
        self.problemUnknown = problemUnknown
        self.streamsBoth = streamsBoth
        self.streamsMicrophone = streamsMicrophone
        self.streamsSystemAudio = streamsSystemAudio
        self.streamsNone = streamsNone
        self.memoryPausedUntil = memoryPausedUntil
        self.turnOnMemory = turnOnMemory
        self.turnOffMemory = turnOffMemory
        self.startMeeting = startMeeting
        self.openBrief = openBrief
        self.openBriefFor = openBriefFor
        self.openTopic = openTopic
        self.askAboutWindow = askAboutWindow
        self.openWhimbrel = openWhimbrel
        self.settings = settings
        self.quit = quit
        self.cancel = cancel
        self.alertWindowName = alertWindowName
        self.hotkeyWindowName = hotkeyWindowName
        self.close = close
        self.copy = copy
        self.answering = answering
    }

    public static let english = WMenuStrings(
        appName: "Whimbrel", menuName: "Whimbrel menu", needsAttention: "Needs attention", nextMeeting: "Next meeting", workingOn: "Working on",
        pauseRecording: "Pause recording", resumeRecording: "Resume recording", stopRecording: "Stop recording",
        stateWithTitle: { "\($0): \($1)" },
        recordingProblem: "Recording problem", startingRecording: "Starting recording", resumingRecording: "Resuming recording",
        stopWaitsForMeeting: "Recording has started. Stop is not available until the meeting is saved.",
        problemUnknown: "Some audio may not be recording.",
        streamsBoth: "Microphone and system audio", streamsMicrophone: "Microphone only", streamsSystemAudio: "System audio only",
        streamsNone: "No audio is being captured.",
        memoryPausedUntil: { "Memory paused until \($0)" },
        turnOnMemory: "Turn on memory", turnOffMemory: "Turn off memory", startMeeting: "Start Meeting", openBrief: "Open brief",
        openBriefFor: { "Open brief for \($0)" },
        openTopic: { "Open topic \($0)" },
        askAboutWindow: "Ask about this window", openWhimbrel: "Open Whimbrel", settings: "Settings", quit: "Quit",
        cancel: "Cancel", alertWindowName: "Whimbrel alert", hotkeyWindowName: "Whimbrel, ask about this window",
        close: "Close", copy: "Copy", answering: "Answering"
    )
}

extension EnvironmentValues {
    /// The words every component reads. The app sets it from its string catalog; the default is English.
    @Entry public var wStrings: WStrings = .english
}

extension View {
    /// Supplies the words the components show. Call it once, high in the view tree, next to `wThemed()`.
    public func wStrings(_ strings: WStrings) -> some View {
        environment(\.wStrings, strings)
    }
}
