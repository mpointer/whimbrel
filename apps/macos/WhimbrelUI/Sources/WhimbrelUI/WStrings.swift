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
    /// "Paused until 10:52". The argument is a clock time the caller formatted.
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

    public init(
        recording: String, starting: String, resuming: String, recordingPaused: String, paused: String, processing: String, off: String, problem: String,
        pause: String, resume: String, stop: String,
        hours: @escaping @Sendable (Int) -> String,
        minutes: @escaping @Sendable (Int) -> String,
        seconds: @escaping @Sendable (Int) -> String,
        joinDetails: @escaping @Sendable ([String]) -> String,
        joinDuration: @escaping @Sendable ([String]) -> String,
        memoryOn: String, memoryOff: String,
        pausedUntil: @escaping @Sendable (String) -> String,
        whatIsCaptured: String, opensPauseChoices: String,
        consentMessage: String, copyNotice: String, copied: String, done: String, notDone: String,
        searchPlaceholder: String, clearSearch: String,
        removeFilter: @escaping @Sendable (String) -> String,
        timeline: String, now: String,
        whimbrelWrote: @escaping @Sendable (String) -> String,
        sourceNumber: @escaping @Sendable (Int) -> String,
        selectsThisSource: String, transcriptMoment: String, seenOnScreen: String, transcript: String, open: String,
        sourceAt: @escaping @Sendable (String) -> String,
        quoted: @escaping @Sendable (String) -> String,
        cited: String, listening: String, spokenListening: String, spokenCited: String, liveTranscript: String,
        memory: String, loading: String, tryAgain: String,
        onboarding: WOnboardingStrings,
        home: WHomeStrings,
        live: WLiveStrings
    ) {
        self.live = live
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
        memoryOn: "Memory on", memoryOff: "Memory off",
        pausedUntil: { "Paused until \($0)" },
        whatIsCaptured: "What is captured", opensPauseChoices: "Opens pause choices",
        consentMessage: "You are recording. Tell the others before you go on.",
        copyNotice: "Copy notice", copied: "Copied", done: "Done", notDone: "Not done",
        searchPlaceholder: "Search your work memory", clearSearch: "Clear search",
        removeFilter: { "Remove filter \($0)" },
        timeline: "Timeline", now: "Now",
        whimbrelWrote: { "Whimbrel wrote: \($0)" },
        sourceNumber: { "Source \($0)" },
        selectsThisSource: "Selects this source", transcriptMoment: "Transcript moment", seenOnScreen: "Seen on screen",
        transcript: "Transcript", open: "Open",
        sourceAt: { "Open source at \($0)" },
        quoted: { "\u{201C}\($0)\u{201D}" },
        cited: "Cited", listening: "Listening", spokenListening: "listening", spokenCited: "cited",
        liveTranscript: "Live transcript",
        memory: "Memory", loading: "Loading", tryAgain: "Try again",
        onboarding: .english,
        home: .english,
        live: .english
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
