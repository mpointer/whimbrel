import Foundation
import SwiftUI
import WhimbrelUI

/// The words of the WhimbrelUI design system components, looked up in this app's string catalog
/// (`Localizable.xcstrings`). The package ships English only and owns no translations; this file is
/// the one place the catalog keys for it live.
///
/// Every `String(localized:)` below is a literal, so `xcodebuild -exportLocalizations` finds it.
/// `WStrings.init` has no default values, so a string added to the package fails to compile here until
/// it is given a key. Numbers and times use the catalog's plural and format rules; the plural entries
/// carry explicit keys (`ui.duration.hours` and so on) because the existing `%lld hours` entry is not a
/// plural entry and may not change.
///
/// Used by the redesign screens (`UI/Redesign`) and the debug gallery. To use it: `view.wStrings(WhimbrelUIStrings.make())`.
enum WhimbrelUIStrings {
    static func make() -> WStrings {
        // Used twice: as a label and, in lower case, inside a spoken line. One lookup each.
        let cited = String(localized: "Cited", comment: "Label on a transcript line that a note quotes.")
        let listening = String(localized: "Listening", comment: "Label under the newest line of the live transcript while Whimbrel is still listening.")
        return WStrings(
            recording: String(localized: "Recording"),
            starting: String(localized: "Starting", comment: "State word on the recording bar while a recording is starting."),
            resuming: String(localized: "Resuming", comment: "State word on the recording bar after Resume, until audio is arriving again."),
            recordingPaused: String(localized: "Recording paused", comment: "Spoken once when a recording is paused."),
            paused: String(localized: "Paused", comment: "State word on the recording indicator while recording is paused."),
            processing: String(localized: "Processing", comment: "State word on the recording indicator while a recording is being processed."),
            off: String(localized: "Not recording", comment: "State word on the recording indicator when nothing is recording. Also its spoken name."),
            problem: String(localized: "Problem", comment: "State word on the recording indicator when something is wrong. A short reason follows it."),
            pause: String(localized: "Pause", comment: "Button on the recording bar that pauses a recording."),
            resume: String(localized: "Resume", comment: "Button on the recording bar that resumes a paused recording."),
            stop: String(localized: "Stop"),
            hours: { count in
                String(localized: "ui.duration.hours", defaultValue: "\(count) hours", comment: "Spoken length of a recording, hours part. Plural.")
            },
            minutes: { count in
                String(localized: "ui.duration.minutes", defaultValue: "\(count) minutes", comment: "Spoken length of a recording, minutes part. Plural.")
            },
            seconds: { count in
                String(localized: "ui.duration.seconds", defaultValue: "\(count) seconds", comment: "Spoken length of a recording, seconds part. Plural.")
            },
            joinDetails: { parts in parts.joined(separator: ", ") },
            joinDuration: { parts in parts.joined(separator: " ") },
            memoryOn: String(localized: "Memory on", comment: "Work memory state in the sidebar footer: Whimbrel is saving context."),
            memoryOff: String(localized: "Memory off", comment: "Work memory state in the sidebar footer: Whimbrel is not saving context."),
            pausedUntil: { time in
                String(localized: "Paused until \(time)", comment: "Work memory state in the sidebar footer. The placeholder is a clock time, for example 10:52 AM.")
            },
            whatIsCaptured: String(localized: "What is captured", comment: "Menu item that explains what work memory saves."),
            opensPauseChoices: String(localized: "Opens pause choices", comment: "Accessibility hint on the work memory state button."),
            consentMessage: String(localized: "You are recording. Tell the others before you go on.", comment: "Recording notice. An instruction to the user. It must not say or suggest that anyone was told, was notified, agreed or consented."),
            copyNotice: String(localized: "Copy notice", comment: "Button that copies the recording notice, so the user can paste it into the meeting chat."),
            copied: String(localized: "Copied", comment: "Shown after the recording notice was copied."),
            done: String(localized: "Done"),
            notDone: String(localized: "Not done", comment: "Spoken state of an action item that is not checked off."),
            searchPlaceholder: String(localized: "Search your work memory", comment: "Placeholder in the search field."),
            clearSearch: String(localized: "Clear search", comment: "Button that empties the search field."),
            removeFilter: { name in
                String(localized: "Remove filter \(name)", comment: "Accessibility label of the button that removes a search filter. The placeholder is the filter name, for example People.")
            },
            timeline: String(localized: "Timeline", comment: "Accessibility label of the time line of the day."),
            now: String(localized: "Now"),
            whimbrelWrote: { text in
                String(localized: "Whimbrel wrote: \(text)", comment: "Accessibility label of a note line that Whimbrel wrote. The placeholder is the line.")
            },
            sourceNumber: { number in
                String(localized: "Source \(number)", comment: "Accessibility label of the numbered mark on a line Whimbrel wrote. The placeholder is the number.")
            },
            selectsThisSource: String(localized: "Selects this source", comment: "Accessibility hint of the numbered source mark."),
            transcriptMoment: String(localized: "Transcript moment", comment: "Kind of a source card: a moment in the meeting transcript."),
            seenOnScreen: String(localized: "Seen on screen", comment: "Kind of a source card: something Whimbrel saw on the screen."),
            transcript: String(localized: "Transcript"),
            open: String(localized: "Open"),
            sourceAt: { time in
                String(localized: "Open source at \(time)", comment: "Accessibility label of a button that jumps to the moment in the transcript. The placeholder is a time in the meeting.")
            },
            quoted: { text in
                // The quote marks follow the language: “text” in English, « text » in French.
                let locale = Locale.current
                let open = locale.quotationBeginDelimiter ?? "\u{201C}"
                let close = locale.quotationEndDelimiter ?? "\u{201D}"
                return open + text + close
            },
            cited: cited,
            listening: listening,
            spokenListening: listening.lowercased(with: .current),
            spokenCited: cited.lowercased(with: .current),
            liveTranscript: String(localized: "Live transcript"),
            memory: String(localized: "Memory", comment: "Label next to a time range on a block that lists what Whimbrel saw in that time."),
            loading: String(localized: "Loading", comment: "Title of a loading placeholder."),
            tryAgain: String(localized: "Try again"),
            onboarding: WOnboardingStrings(
                allowed: String(localized: "Allowed", comment: "State of a permission row on the first run and permissions screens: macOS has allowed it."),
                notYet: String(localized: "Not yet", comment: "State of a permission row: Whimbrel has not asked for it yet."),
                denied: String(localized: "Denied", comment: "State of a permission row: it was asked for and not allowed."),
                allow: String(localized: "Allow", comment: "Button on a permission row that has not been asked for yet. Asks macOS."),
                openSystemSettings: String(localized: "Open System Settings"),
                allowNamed: { name in
                    String(localized: "Allow \(name)", comment: "Spoken name of the Allow button on a permission row. The placeholder is the permission, for example Microphone.")
                },
                openSettingsNamed: { name in
                    String(localized: "Open System Settings for \(name)", comment: "Spoken name of the Open System Settings button on a permission row. The placeholder is the permission, for example Microphone.")
                },
                recordingCaption: String(localized: "This is what you see while Whimbrel is recording.", comment: "Caption under a picture of the recording indicator on the first run screen."),
                moreOptions: String(localized: "More options", comment: "Menu that holds the extra buttons of a screen that has more than three."),
                stepOf: { step, total in
                    String(localized: "Step \(step) of \(total)", comment: "Step line at the top of a setup screen. First placeholder is the current step, second the number of steps.")
                },
                wordItem: { number, total, word in
                    String(localized: "Word \(number) of \(total), \(word)", comment: "Spoken form of one recovery word in the numbered list. Placeholders: its number, how many words, the word.")
                }
            ),
            home: WHomeStrings(
                peopleFilter: String(localized: "People", comment: "Search filter chip: narrows a search to one person from recent meetings."),
                anyPerson: String(localized: "Any person", comment: "Choice on the People filter: no person filter."),
                appsFilter: String(localized: "Apps", comment: "Search filter chip: narrows a search to captures from one app."),
                datesFilter: String(localized: "Dates", comment: "Search filter chip: narrows a search to a date range."),
                anyApp: String(localized: "Any app", comment: "Choice on the Apps filter: no app filter."),
                anyDate: String(localized: "Any date", comment: "Choice on the Dates filter: no date filter."),
                today: String(localized: "Today"),
                yesterday: String(localized: "Yesterday"),
                lastSevenDays: String(localized: "Last 7 days", comment: "Choice on the Dates filter."),
                customRange: String(localized: "Custom range", comment: "Choice on the Dates filter: pick a first and a last day."),
                rangeFrom: String(localized: "From", comment: "Label of the first day of a custom date range."),
                rangeTo: String(localized: "To", comment: "Label of the last day of a custom date range."),
                applyRange: String(localized: "Apply", comment: "Button that applies a custom date range."),
                searchFilters: String(localized: "Search filters", comment: "Accessibility label of the group of search filter chips."),
                sections: String(localized: "Sections", comment: "Accessibility label of the sidebar list of sections."),
                pauseFifteenMinutes: String(localized: "Pause for 15 minutes"),
                pauseOneHour: String(localized: "Pause for 1 hour"),
                pauseUntilTomorrow: String(localized: "Pause until tomorrow"),
                resumeNow: String(localized: "Resume now"),
                thread: String(localized: "Thread", comment: "Heading of the sidebar list of parts of the day: Today, Yesterday, Last week."),
                lastWeek: String(localized: "Earlier", comment: "Collapsed row on Today for meetings from the seven days before yesterday, and its entry in the sidebar list."),
                expanded: String(localized: "Expanded", comment: "Spoken state of a row on Today that is open."),
                collapsed: String(localized: "Collapsed", comment: "Spoken state of a row on Today that is closed.")
            ),
            live: WLiveStrings(
                recordingPaused: String(localized: "Recording paused", comment: "Spoken once when a recording is paused."),
                markThisMoment: String(localized: "Mark this moment", comment: "Button on the live meeting screen that marks the current moment of the meeting."),
                momentsYouMarked: String(localized: "Moments you marked", comment: "Heading of the list of moments the person marked during a meeting."),
                audioStaysOnMac: String(localized: "Audio stays on this Mac.", comment: "Line at the foot of the live meeting screen: the audio is not sent anywhere."),
                tidiesNotes: String(localized: "Whimbrel tidies your notes when you stop.", comment: "Line at the foot of the live meeting screen: what happens after the recording stops."),
                stillPaused: String(localized: "This recording is still paused.", comment: "Reminder shown when a recording has stayed paused for a while."),
                showInMeetingWindow: String(localized: "Show in-meeting window", comment: "Menu item that shows the small always-on-top window with the recording controls."),
                audioArriving: String(localized: "Audio is arriving", comment: "Spoken name of the level meter while sound is coming in."),
                noAudioNow: String(localized: "No audio right now", comment: "Spoken name of the level meter while no sound is coming in."),
                meetingWindowName: String(localized: "Whimbrel, in-meeting window", comment: "Spoken name of the small always-on-top window shown during a recording."),
                askAboutMeeting: String(localized: "Ask about this meeting", comment: "Placeholder of the question field on the live meeting screen and the in-meeting window."),
                started: String(localized: "Started", comment: "Caption under the time a recording started, on the short time line of the live meeting screen."),
                notes: String(localized: "Notes", comment: "Spoken name of the notes field on the live meeting screen."),
                closeWindow: String(localized: "Close in-meeting window", comment: "Spoken name of the button that closes the small always-on-top recording window. It does not stop the recording."),
                momentAt: { time in
                    String(localized: "Moment at \(time)", comment: "Spoken name of a marked moment. The placeholder is a clock time, for example 10:12.")
                },
                removeMomentAt: { time in
                    String(localized: "Remove moment at \(time)", comment: "Spoken name of the button that removes a marked moment. The placeholder is a clock time.")
                },
                momentMarked: { time in
                    String(localized: "Moment marked at \(time)", comment: "Spoken once after Mark this moment worked. The placeholder is a clock time.")
                },
                count: { number in "\(number)" }
            ),
            note: WNoteStrings(
                note: String(localized: "Note"),
                viewSwitch: String(localized: "View", comment: "Spoken name of the switch between the Note and the Transcript on a finished meeting page."),
                copy: String(localized: "Copy"),
                copyTranscript: String(localized: "Copy transcript", comment: "Button in the header of a finished meeting's transcript view. It copies the whole transcript. In the note view the Copy button copies the summary instead."),
                moreCopyFormats: String(localized: "More copy formats", comment: "Spoken name of the small arrow beside the Copy button on a finished meeting note. It opens a menu with Copy as Markdown and Copy as Slack."),
                copyAsMarkdown: String(localized: "Copy as Markdown"),
                copyAsSlack: String(localized: "Copy as Slack"),
                sendRecap: String(localized: "Send recap"),
                edit: String(localized: "Edit"),
                regenerate: String(localized: "Regenerate"),
                yourNotes: String(localized: "Your notes"),
                summary: String(localized: "Summary"),
                decisions: String(localized: "Decisions"),
                openQuestions: String(localized: "Open questions"),
                actionItems: String(localized: "Action items"),
                marginIntro: String(localized: "Whimbrel wrote the marked lines. Each number points to the moment in the transcript it came from.", comment: "Line at the top of the margin of a finished meeting note. The lines Whimbrel wrote are marked with a colored rule and a number; each number points to a moment in the transcript."),
                regenerateForSources: String(localized: "Regenerate the summary to add sources.", comment: "Line in the margin of an older finished meeting note whose lines have no sources yet."),
                openInTranscriptAt: { time in
                    String(localized: "Open in transcript at \(time)", comment: "Link on a source card, and the spoken name of a marked moment: opens the transcript at a time. The placeholder is a time in the meeting, for example 18:25.")
                },
                dueOn: { due in
                    String(localized: "Due \(due)", comment: "Due date under an action item. The placeholder is a date, for example Thu, Nov 19.")
                },
                speakerLanes: String(localized: "Speaker lanes", comment: "Spoken name of the picture above a transcript that shows when each person spoke, one lane per person."),
                showingLine: { time in
                    String(localized: "Showing the line at \(time)", comment: "Spoken once when a transcript line is highlighted. The placeholder is a time in the meeting, for example 18:25.")
                },
                lanesHint: String(localized: "Use the arrow keys to move between spans, and Return to open the line.", comment: "Spoken hint of the speaker lanes above a transcript.")
            )
        )
    }
}
