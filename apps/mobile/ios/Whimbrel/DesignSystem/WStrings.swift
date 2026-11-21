import Foundation

/// The words the design-system components draw themselves, the same words as the desktop Thread
/// system. Components never read the string catalog: a screen fills this from the catalog and
/// passes it down (`.environment(\.wStrings, …)`), so the debug gallery and tests can use `.english`.
struct WStrings: Sendable {
    var recording = "Recording"
    var paused = "Paused"
    var processing = "Processing"
    var notRecording = "Not recording"
    var problem = "Problem"
    var pause = "Pause"
    var resume = "Resume"
    var stop = "Stop"
    var stopAndSave = "Stop and save"
    var consentMessage = "You are recording. Tell the others before you go on."
    var showNotice = "Show notice"
    var done = "Done"
    var notDone = "Not done"
    var searchPlaceholder = "Search your work memory"
    var clearSearch = "Clear search"
    var now = "Now"
    var memory = "Memory"
    var transcriptMoment = "Transcript moment"
    var seenOnScreen = "Seen on screen"
    var open = "Open"
    var listening = "Listening"
    var cited = "Cited"
    var loading = "Loading"
    var tryAgain = "Try again"
    var offline = "No connection. Showing what is already on this phone."
    var whimbrelWrote: @Sendable (String) -> String = { "Whimbrel wrote: \($0)" }
    var sourceNumber: @Sendable (Int) -> String = { "Source \($0)" }
    var removeFilter: @Sendable (String) -> String = { "Remove filter \($0)" }
    /// "Recording, 14 minutes 8 seconds": a state and a detail joined for speech.
    var spoken: @Sendable (String, String) -> String = { "\($0), \($1)" }

    static let english = WStrings()

    /// The app's words: the entries the string catalog already has, English for the rest until a
    /// screen that shows them is rebuilt and adds them to the catalog.
    static var app: WStrings {
        var s = WStrings()
        s.now = String(localized: "Now")
        s.done = String(localized: "Done")
        s.stop = String(localized: "Stop")
        s.tryAgain = String(localized: "Try again")
        return s
    }
}

enum WElapsed {
    /// "12:47", or "1:02:47" past an hour.
    static func clock(_ seconds: Int) -> String {
        let s = max(0, seconds)
        return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, (s % 3600) / 60, s % 60) : String(format: "%d:%02d", s / 60, s % 60)
    }

    /// "12 minutes, 47 seconds", for VoiceOver.
    static func spoken(_ seconds: Int) -> String {
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .full
        formatter.allowedUnits = seconds >= 3600 ? [.hour, .minute, .second] : [.minute, .second]
        return formatter.string(from: TimeInterval(max(0, seconds))) ?? clock(seconds)
    }
}
