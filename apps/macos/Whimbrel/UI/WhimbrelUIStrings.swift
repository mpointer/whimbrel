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
            recordingPaused: String(localized: "Recording paused", comment: "State of a paused recording, as written in the menu bar menu and as spoken once when a recording is paused."),
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
            memoryNotCapturing: String(localized: "Memory on, but not capturing", comment: "Work memory state: memory is switched on but Whimbrel is not saving text from the screen, for example because a permission is off."),
            pausedUntil: { time in
                String(localized: "Memory paused until \(time)", comment: "Work memory paused, as written on the memory control and the time line and as spoken. It says Memory because a recording can be paused too. The placeholder is a clock time, for example 3:10 PM.")
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
            sourceEmail: String(localized: "Email"),
            sourceRoutineRun: String(localized: "Routine run", comment: "Kind of a source card: the result of a routine."),
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
                recordingPaused: String(localized: "Recording paused", comment: "State of a paused recording, as written in the menu bar menu and as spoken once when a recording is paused."),
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
            ),
            find: findStrings(),
            menu: menuStrings()
        )
    }

    /// The words of the menu bar menu, the small alert windows and the hotkey panel. Existing keys keep their wording and comment.
    static func menuStrings() -> WMenuStrings {
        WMenuStrings(
            appName: String(localized: "Whimbrel"),
            menuName: String(localized: "Whimbrel menu", comment: "Spoken name of the menu that opens from Whimbrel's icon in the menu bar."),
            needsAttention: String(localized: "Needs attention"),
            nextMeeting: String(localized: "Next meeting"),
            workingOn: String(localized: "Working on", comment: "Heading in the menu bar menu above what Whimbrel is doing after a recording, for example Transcribing."),
            pauseRecording: String(localized: "Pause recording", comment: "Spoken name of the Pause button for a recording in the menu bar menu. Not the pause of work memory."),
            resumeRecording: String(localized: "Resume recording", comment: "Spoken name of the Resume button for a paused recording in the menu bar menu."),
            stopRecording: String(localized: "Stop recording", comment: "Spoken name of the Stop button for a recording in the menu bar menu."),
            stateWithTitle: { word, title in
                String(localized: "\(word): \(title)", comment: "A label and its value joined by a colon. The first placeholder is the label, the second is its value. In the menu bar menu the label is the recording's state, for example Recording paused, and the value is the meeting's title.")
            },
            recordingProblem: String(localized: "Recording problem", comment: "State of a recording that has a problem, in the menu bar menu. The reason and the time follow it."),
            startingRecording: String(localized: "Starting recording", comment: "State in the menu bar menu while a recording is starting."),
            resumingRecording: String(localized: "Resuming recording", comment: "State in the menu bar menu after Resume, until audio is arriving again."),
            stopWaitsForMeeting: String(localized: "Recording has started. Stop is not available until the meeting is saved.", comment: "Line in the menu bar menu under Starting recording, when audio is already being recorded but the meeting is not saved yet, so Stop does nothing yet."),
            problemUnknown: String(localized: "Some audio may not be recording.", comment: "Line under Recording problem in the menu bar menu when no reason is known."),
            streamsBoth: String(localized: "Microphone and system audio", comment: "Line under Recording in the menu bar menu: the microphone and the other people on the call are both being recorded."),
            streamsMicrophone: String(localized: "Microphone only", comment: "Line under Recording in the menu bar menu: only the person using this Mac is being recorded."),
            streamsSystemAudio: String(localized: "System audio only", comment: "Line under Recording in the menu bar menu: only the sound from the Mac, such as the other people on a call, is being recorded."),
            streamsNone: String(localized: "No audio is being captured.", comment: "Line under Recording problem in the menu bar menu: neither the microphone nor system audio is being recorded."),
            memoryPausedUntil: { time in
                String(localized: "Memory paused until \(time)", comment: "Work memory paused, as written on the memory control and the time line and as spoken. It says Memory because a recording can be paused too. The placeholder is a clock time, for example 3:10 PM.")
            },
            turnOnMemory: String(localized: "Turn on memory", comment: "Menu item in the capture state menu: starts saving context again."),
            turnOffMemory: String(localized: "Turn off memory", comment: "Menu item in the capture state menu: stops saving context."),
            startMeeting: String(localized: "Start Meeting"),
            openBrief: String(localized: "Open brief"),
            openBriefFor: { title in
                String(localized: "Open brief for \(title)", comment: "Spoken name of the Open brief button in the menu bar menu. The placeholder is the meeting's title.")
            },
            openTopic: { name in
                String(localized: "Open topic \(name)", comment: "Spoken name of the topic chip in the menu bar menu. The placeholder is the topic's name.")
            },
            askAboutWindow: String(localized: "Ask about this window"),
            openWhimbrel: String(localized: "Open Whimbrel"),
            settings: String(localized: "Settings"),
            quit: String(localized: "Quit"),
            cancel: String(localized: "Cancel"),
            alertWindowName: String(localized: "Whimbrel alert", comment: "Spoken name of the small window that asks about a meeting or a long recording."),
            hotkeyWindowName: String(localized: "Whimbrel, ask about this window", comment: "Spoken name of the small window opened by the Ask about this window shortcut."),
            close: String(localized: "Close"),
            copy: String(localized: "Copy"),
            answering: String(localized: "Answering", comment: "Spoken name of the progress indicator in the Ask about this window panel while the answer is being written."),
            // The old menu's disabled button after a Stop during a start; the key is the one it already uses.
            saving: String(localized: "Saving…")
        )
    }

    /// The words of Library's Find page and its other views, looked up in the catalog. Every field of `WFindStrings` is set here.
    static func findStrings() -> WFindStrings {
        var f = WFindStrings()
        f.viewSwitchName = String(localized: "Library view", comment: "Spoken name of the switch between Find, Recent, Episodes and Digest in Library.")
        f.viewFind = String(localized: "Find", comment: "Name of Library's search view, the first of four views: Find, Recent, Episodes, Digest.")
        f.kindSlide = String(localized: "Slide", comment: "Kind of a search result: a slide saved from a meeting.")
        f.kindAutomationRun = String(localized: "Automation run", comment: "Kind of a search result: one run of an automation.")
        f.meetingAudio = String(localized: "Meeting audio", comment: "Where a transcript moment comes from, shown on its search result card in the place an app name takes on other cards.")
        f.resultCount = { v in String(localized: "Results: \(v)", comment: "Count at the right of the filter chips on Library's Find page. The placeholder is a number.") }
        f.sourceCount = { v in String(localized: "\(v) sources found", comment: "Count at the right of the filter chips once an answer is shown, in singular and plural. The placeholder is the number of sources the answer used.") }
        f.moreMayExist = String(localized: "More may exist", comment: "Under the results when the search reached its limit of candidates.")
        f.orderName = String(localized: "Order", comment: "Spoken name of the switch that orders search results by day or by best match.")
        f.orderByDay = String(localized: "By day", comment: "Choice that groups search results by day, newest first.")
        f.orderBestFirst = String(localized: "Best matches first", comment: "Choice that lists search results with the best matches first, not grouped by day.")
        f.askOnThisMac = String(localized: "Ask, on this Mac", comment: "Ask button when the answer is made on this Mac and nothing leaves it.")
        f.askProvider = { v in String(localized: "Ask \(v)", comment: "Ask button when the answer is made by a hosted provider. The placeholder is the provider's name, for example Anthropic.") }
        f.askHint = String(localized: "Enter searches. Ask writes an answer from what it finds.", comment: "Line under the Ask button on Library's Find page. Pressing Enter only searches this device; the Ask button sends the question to the model it names.")
        f.emptyTitle = String(localized: "Find anything you worked on", comment: "Title of Library's Find page before anything is typed.")
        f.emptyHelp = String(localized: "Type a few words or a question. Results come from this device: meetings, what was on screen, slides, email threads and episodes.", comment: "Help under the title of Library's Find page before anything is typed.")
        f.startersName = String(localized: "Ideas to start with", comment: "Spoken name of the row of example questions on Library's Find page.")
        f.searching = String(localized: "Searching", comment: "Shown while Library's Find page looks for results.")
        f.noResultsTitle = String(localized: "Nothing found", comment: "Title when a search on Library's Find page has no results.")
        f.noResultsHelp = String(localized: "Nothing on this device matches those words. Try fewer words, or a different spelling.", comment: "Help when a search with no filters has no results.")
        f.noResultsWithFilters = { v in String(localized: "Nothing matches with these filters on: \(v).", comment: "Help when a search with filters on has no results. The placeholder lists the filters, for example People: Sam Okafor, Dates: Last week.") }
        f.clearFilters = String(localized: "Clear filters", comment: "Button that turns every search filter off.")
        f.searchFailedTitle = String(localized: "The search did not finish", comment: "Title when Library's search failed.")
        f.noMeaningSearch = String(localized: "The search model is not installed, so results match your words only.", comment: "Notice on Library's Find page while the search model is missing: results match the typed words and not their meaning.")
        f.answeredOnThisMac = String(localized: "Answered on this Mac.", comment: "Note under an answer that was made on this Mac.")
        f.answeredByLocalModel = String(localized: "Answered by a local model.", comment: "Note under an answer opened from a saved conversation that was made by a model on this computer. Only the saved flag is known, so it does not say which computer.")
        f.stoppedNote = String(localized: "Stopped. Your question is back in the field.", comment: "Note after the person stops an answer that was being written. The question returns to the search field.")
        f.routeChanged = String(localized: "Nothing was sent. The answer would no longer come from where the Ask button said. The button now shows where it would come from. Check it, then ask again.", comment: "Shown on Library's Find page when a question was held back: the Ask button had named where the answer would come from, and a fresh look at the route showed otherwise.")
        f.newConversationNote = String(localized: "You changed the filters, so this is a new conversation.", comment: "Note when the person changes a search filter after an answer: an answer made under one set of filters is never used for a question under another.")
        f.answerTitle = String(localized: "Answer", comment: "Heading of the answer card on Library's Find page.")
        f.fromMemory = String(localized: "From your work memory", comment: "Line beside the answer heading: what the answer drew on.")
        f.fromMemoryRange = { v in String(localized: "From your work memory, \(v)", comment: "Line beside the answer heading when a date filter is on. The placeholder is a range of days, for example Nov 9 to Nov 15.") }
        f.copyAnswer = String(localized: "Copy answer", comment: "Button on the answer card that copies the answer's text.")
        f.copiedAnswer = String(localized: "Answer copied", comment: "Shown on the answer card for a moment after Copy answer.")
        f.answersCanBeWrong = String(localized: "Answers can be wrong. Open the sources to check.", comment: "Line under every answer.")
        f.answerIn = { v in String(localized: "Answer: \(v)", comment: "Spoken once when an answer is finished. The placeholder is the answer's text.") }
        f.yourQuestion = { v in String(localized: "Your question: \(v)", comment: "Spoken name of a question above its answer. The placeholder is the question.") }
        f.sourcesHeading = String(localized: "Sources for this answer", comment: "Heading above the numbered sources of an answer on the time line.")
        f.otherResultsHeading = String(localized: "Other results", comment: "Heading above the search results that are not sources of the answer.")
        f.mentionsHeading = { v in String(localized: "Mentions of \(v)", comment: "Heading above screen captures that mention a person. The placeholder is the person's name.") }
        f.mentionsHelp = String(localized: "Screen captures whose text holds the name. They are listed apart from meetings.", comment: "Line under the Mentions heading.")
        f.loadMore = String(localized: "Load more", comment: "Button under the search results that loads the next results.")
        f.loadingMore = String(localized: "Loading more", comment: "Shown on the Load more button while it works.")
        f.opensSavedCapture = String(localized: "Opens the saved capture", comment: "Line beside the Open button of a screen capture result.")
        f.opensNoteAt = { v in String(localized: "Opens the note at \(v)", comment: "Line beside the Open button of a transcript moment. The placeholder is a time in the meeting, for example 14:32.") }
        f.opensMeeting = String(localized: "Opens the meeting", comment: "Line beside the Open button of a slide or a meeting result.")
        f.opensEpisode = String(localized: "Opens the episode", comment: "Line beside the Open button of an episode result.")
        f.opensThread = String(localized: "Opens the thread summary", comment: "Line beside the Open button of an email thread result.")
        f.noLongerOnDevice = String(localized: "No longer on this device", comment: "Shown on a source or a result whose saved item is gone from this device. No text is shown with it.")
        f.couldNotRead = String(localized: "Could not read this just now.", comment: "Shown on a source of an answer whose saved item could not be read just now. It is not deleted, only unreadable for the moment.")
        f.couldNotReadHeading = String(localized: "Could not read just now", comment: "Heading of the group of sources that could not be read just now, on Library's Find page.")
        f.showMore = String(localized: "Show more", comment: "Button that shows the whole stored text on a result card.")
        f.spokenBy = { v in String(localized: "Spoken by \(v)", comment: "Line on a transcript result found because the person named in the filter spoke it. The placeholder is the person's name.") }
        f.inMeetingTheyAttended = String(localized: "In a meeting they attended", comment: "Line on a transcript result found because it is from a meeting the person named in the filter attended.")
        f.followUpPlaceholder = String(localized: "Ask a follow-up question", comment: "Placeholder of the field under an answer that continues the conversation.")
        f.deleteBusyHint = String(localized: "An answer is being written in this conversation. Delete it when the answer is done.", comment: "Spoken hint on the Delete control of a conversation in Library's Find page while an answer is being written in it. Delete is off until the answer is done.")
        f.askBusyReason = String(localized: "An answer is being written. Ask again when it is done.", comment: "Spoken hint on the header's Ask button while an answer is being written on Library's Find page. The button is off until the answer is done.")
        f.noConversations = String(localized: "No conversations yet.", comment: "Shown in the conversations panel when there are none.")
        f.showScreenCaptures = String(localized: "Show screen captures", comment: "Switch on Library's Recent view: list screen captures as well as meetings. The same setting as Show screen captures in Library in Settings.")
        f.hiddenEpisode = String(localized: "Hidden", comment: "Label on an episode the person hid, shown when hidden episodes are listed.")
        f.regenerateDigest = String(localized: "Regenerate today's digest", comment: "Button on Library's Digest view once a digest is shown.")
        f.viewRecent = String(localized: "Recent")
        f.viewEpisodes = String(localized: "Episodes")
        f.viewDigest = String(localized: "Digest")
        f.kindMeeting = String(localized: "Meeting")
        f.kindEpisode = String(localized: "Episode")
        f.kindEmailThread = String(localized: "Email thread", comment: "Client controls (X6, Task 10).")
        f.open = String(localized: "Open")
        f.showLess = String(localized: "Show less")
        f.askButton = String(localized: "Ask")
        f.followUpButton = String(localized: "Ask")
        f.conversationsHeading = String(localized: "Conversations")
        f.showConversations = String(localized: "Show Conversations")
        f.hideConversations = String(localized: "Hide Conversations")
        f.newConversation = String(localized: "New Conversation")
        f.renameConversation = String(localized: "Rename")
        f.deleteConversation = String(localized: "Delete")
        f.starConversation = String(localized: "Star conversation")
        f.unstarConversation = String(localized: "Unstar conversation")
        f.showHiddenEpisodes = String(localized: "Show hidden episodes")
        f.openInNewWindow = String(localized: "Open in new window")
        f.forgetMeeting = String(localized: "Forget meeting")
        f.generateDigest = String(localized: "Generate Today's Digest")
        f.openLoops = String(localized: "Open loops")
        f.legacySourcesHeading = String(localized: "Sources")
        return f
    }
}
