import UserNotifications
import XCTest
@testable import Whimbrel

/// "Still in a meeting?" (docs/superpowers/specs/2026-11-05-trim-and-still-in-meeting-design.md).
final class StillInMeetingTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000_000)
    private func at(_ seconds: Double) -> Date { start.addingTimeInterval(seconds) }

    private func state(_ now: Double, speech: Double? = nil, shown: Double? = nil, kept: Double? = nil) -> StillInMeeting.Action {
        StillInMeeting.state(now: at(now), recordingStartedAt: start, lastSpeechAt: speech.map(at),
                             promptShownAt: shown.map(at), keptAt: kept.map(at))
    }

    private func segment(endCs: Int64, text: String) -> CoreLiveSegment {
        CoreLiveSegment(id: endCs, meetingId: 1, channel: "system", chunkIndex: 0, startCs: max(0, endCs - 300),
                        endCs: endCs, speakerLabel: "Them", text: text)
    }

    func testPromptAfterFifteenQuietMinutes() {
        XCTAssertEqual(state(899), .none)
        XCTAssertEqual(state(900), .prompt)
    }

    func testSpeechResetsTimer() {
        XCTAssertEqual(state(900, speech: 600), .none)
        XCTAssertEqual(state(1499, speech: 600), .none)
        XCTAssertEqual(state(1500, speech: 600), .prompt)
    }

    func testKeepRearmsFifteen() {
        // Kept at 1000 s after a prompt at 900 s: quiet again until 1900 s.
        XCTAssertEqual(state(1899, kept: 1000), .none)
        XCTAssertEqual(state(1900, kept: 1000), .prompt)
    }

    func testNoAnswerStopsUntrimmed() {
        // The stop never cuts: the queue trims once the full transcript is stored.
        XCTAssertEqual(state(1500 + 119, speech: 600, shown: 1500), .prompt)
        XCTAssertEqual(state(1500 + 120, speech: 600, shown: 1500), .stop)
        XCTAssertEqual(state(1020, shown: 900), .stop)
    }

    private func line(_ startCs: Int64, _ endCs: Int64, _ text: String = "words") -> TranscriptionService.Line {
        TranscriptionService.Line(speakerLabel: "Me", startCs: startCs, endCs: endCs, text: text)
    }

    func testAutoTrimKeepsToLastStoredSpeechPlus30() {
        // Last line with text ends at 600 s (its blank successor doesn't count): keep 630 s.
        XCTAssertEqual(StillInMeeting.autoTrimKeepMs(stored: [line(10_000, 0), line(59_000, 0), line(70_000, 0, " ")],
                                                     partLines: [line(10_000, 12_000), line(59_000, 60_000), line(70_000, 71_000, " ")],
                                                     partOffsetMs: 0, lengthMs: 1_000_000), 630_000)
        // A later part's lines are offset into the meeting clock.
        XCTAssertEqual(StillInMeeting.autoTrimKeepMs(stored: [line(10_000, 0)], partLines: [line(500, 1_000)],
                                                     partOffsetMs: 200_000, lengthMs: 1_000_000), 240_000)
        // Nothing said: no trim. Keep not shorter than the meeting: no trim.
        XCTAssertNil(StillInMeeting.autoTrimKeepMs(stored: [line(100, 0, "")], partLines: [], partOffsetMs: 0, lengthMs: 1_000_000))
        XCTAssertNil(StillInMeeting.autoTrimKeepMs(stored: [line(10_000, 0)], partLines: [line(10_000, 12_000)],
                                                   partOffsetMs: 0, lengthMs: 150_000))
    }

    func testSpeechDuringWaitCancels() {
        XCTAssertEqual(state(1000, speech: 950, shown: 900), .none)
        // Speech before the prompt doesn't.
        XCTAssertEqual(state(1000, speech: 100, shown: 900), .prompt)
    }

    func testLateArrivingSpeechCancelsStop() {
        // A backlogged line timed at 600 s arrived at 950 s, after the prompt: someone is talking.
        XCTAssertEqual(StillInMeeting.state(now: at(1020), recordingStartedAt: start, lastSpeechAt: at(600),
                                            speechArrivedAt: at(950), promptShownAt: at(900), keptAt: nil), .none)
        // And it re-arms the quiet timer, so the prompt doesn't come straight back.
        XCTAssertEqual(StillInMeeting.state(now: at(1025), recordingStartedAt: start, lastSpeechAt: at(600),
                                            speechArrivedAt: at(950), promptShownAt: nil, keptAt: nil), .none)
        XCTAssertEqual(StillInMeeting.state(now: at(1850), recordingStartedAt: start, lastSpeechAt: at(600),
                                            speechArrivedAt: at(950), promptShownAt: nil, keptAt: nil), .prompt)
        // Arrival is a new line with text, not a refresh of the same rows or a blank one.
        let old = [segment(endCs: 30_000, text: "hi there")]
        XCTAssertFalse(StillInMeeting.hasNewSpeech(old: old, new: old))
        XCTAssertFalse(StillInMeeting.hasNewSpeech(old: old, new: old + [segment(endCs: 40_000, text: " ")]))
        XCTAssertTrue(StillInMeeting.hasNewSpeech(old: old, new: old + [segment(endCs: 20_000, text: "late again")]))
    }

    // MARK: Filler is not speech

    private func line(endCs: Int64, lengthCs: Int64, text: String) -> CoreLiveSegment {
        CoreLiveSegment(id: endCs, meetingId: 1, channel: "system", chunkIndex: 0, startCs: endCs - lengthCs,
                        endCs: endCs, speakerLabel: "Them", text: text)
    }

    func testShortFillerLineIsNotSpeech() {
        // "Thank you." from near-silent audio is two words but under a second.
        let filler = line(endCs: 60_000, lengthCs: 60, text: "Thank you.")
        XCTAssertFalse(StillInMeeting.countsAsSpeech(filler))
        XCTAssertFalse(StillInMeeting.hasNewSpeech(old: [], new: [filler]))
        XCTAssertNil(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: true, liveSegments: [filler],
                                                 lastNonSilentAt: at(0)))
        // One word is not speech however long it runs.
        XCTAssertFalse(StillInMeeting.countsAsSpeech(line(endCs: 60_000, lengthCs: 400, text: "Thanks.")))
        XCTAssertFalse(StillInMeeting.countsAsSpeech(line(endCs: 60_000, lengthCs: 400, text: "... - 12")))
    }

    func testRealTwoWordLineOfASecondResetsTheClock() {
        let real = line(endCs: 60_000, lengthCs: 100, text: "Let's begin")
        XCTAssertTrue(StillInMeeting.countsAsSpeech(real))
        XCTAssertTrue(StillInMeeting.hasNewSpeech(old: [], new: [real]))
        XCTAssertEqual(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: true, liveSegments: [real],
                                                   lastNonSilentAt: at(0)), at(600))
        XCTAssertFalse(StillInMeeting.countsAsSpeech(line(endCs: 60_000, lengthCs: 99, text: "Let's begin")))
    }

    func testFillerAmongRealLinesDoesNotMoveLastSpeech() {
        let segments = [line(endCs: 30_000, lengthCs: 300, text: "We shipped it"), line(endCs: 90_000, lengthCs: 50, text: "Thank you.")]
        XCTAssertEqual(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: true, liveSegments: segments,
                                                   lastNonSilentAt: at(0)), at(300))
    }

    func testPromptIsNotCancelledByOneFillerLine() {
        // A prompt shows at 900 s. A filler line arrives: it must not count as arrival, so
        // the prompt runs to its stop.
        let filler = line(endCs: 95_000, lengthCs: 60, text: "Thank you.")
        let arrived: Date? = StillInMeeting.hasNewSpeech(old: [], new: [filler]) ? at(950) : nil
        let lastSpeech = StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: true, liveSegments: [filler],
                                                     lastNonSilentAt: at(0))
        XCTAssertEqual(StillInMeeting.state(now: at(960), recordingStartedAt: start, lastSpeechAt: lastSpeech,
                                            speechArrivedAt: arrived, promptShownAt: at(900), keptAt: nil), .prompt)
        XCTAssertEqual(StillInMeeting.state(now: at(1020), recordingStartedAt: start, lastSpeechAt: lastSpeech,
                                            speechArrivedAt: arrived, promptShownAt: at(900), keptAt: nil), .stop)
    }

    func testWordsWithLettersAreUnicodeAware() {
        XCTAssertEqual(StillInMeeting.speechWordCount("Thank you."), 2)
        XCTAssertEqual(StillInMeeting.speechWordCount("Gracias, amigo m\u{ED}o"), 3)
        XCTAssertEqual(StillInMeeting.speechWordCount("12 - ... 3.5"), 0)
        // A no-space script: a run of 2 or more letters is enough on its own.
        XCTAssertGreaterThanOrEqual(StillInMeeting.speechWordCount("\u{4F1A}\u{8B70}\u{3092}\u{59CB}\u{3081}\u{307E}\u{3059}"), 2)
        XCTAssertGreaterThanOrEqual(StillInMeeting.speechWordCount("\u{5927}\u{5BB6}\u{597D}"), 2)
        XCTAssertLessThan(StillInMeeting.speechWordCount("\u{306F}"), 2)
        XCTAssertTrue(StillInMeeting.countsAsSpeech(line(endCs: 60_000, lengthCs: 300, text: "\u{4F1A}\u{8B70}\u{3092}\u{59CB}\u{3081}\u{307E}\u{3059}")))
        XCTAssertFalse(StillInMeeting.countsAsSpeech(line(endCs: 60_000, lengthCs: 50, text: "\u{4F1A}\u{8B70}\u{3092}\u{59CB}\u{3081}\u{307E}\u{3059}")))
    }

    func testCheckInLogLineHasFieldsAndNoTranscriptText() {
        let rows = [line(endCs: 60_000, lengthCs: 300, text: "Secret budget plan"), line(endCs: 61_000, lengthCs: 40, text: "Thank you.")]
        let arrival = CheckInLog.arrival(old: [], new: rows)
        XCTAssertEqual(arrival, CheckInLog.Arrival(speechLines: 1, fillerLines: 1, words: 3, characters: 18))
        let text = CheckInLog.line(event: "speech-arrived", reason: "live-line", lastSpeechAt: at(600), quietSince: nil,
                                   liveStatus: "listening", engine: "WhisperEngine", extra: arrival.extra)
        XCTAssertTrue(text.contains("event=speech-arrived reason=live-line"))
        XCTAssertTrue(text.contains("quietSince=none live=listening engine=WhisperEngine"))
        XCTAssertTrue(text.contains("speechLines=1 fillerLines=1 words=3 characters=18"))
        XCTAssertFalse(text.contains("Secret"))
        XCTAssertFalse(text.contains("Thank"))
        // Rows already seen add nothing.
        XCTAssertEqual(CheckInLog.arrival(old: rows, new: rows), CheckInLog.Arrival())
    }

    func testQuietSinceNamesTheLatestResetter() {
        XCTAssertEqual(StillInMeeting.quietSince(recordingStartedAt: start, lastSpeechAt: at(600), speechArrivedAt: at(700), keptAt: at(650)), at(700))
        XCTAssertEqual(StillInMeeting.quietSince(recordingStartedAt: start, lastSpeechAt: nil, speechArrivedAt: nil, keptAt: nil), start)
    }

    func testPromptOnlyForManual() {
        XCTAssertTrue(StillInMeeting.applies(to: .manual))
        for trigger in [CoreMeetingTrigger.calendar, .detected, .away] {
            XCTAssertFalse(StillInMeeting.applies(to: trigger))
        }
    }

    func testFallsBackToAudioLevelWhenLiveOff() {
        let segments = [segment(endCs: 30_000, text: "Thanks, see you"), segment(endCs: 45_000, text: "  ")]
        // Listening: the latest line with text, by its end (300 s), not the blank one or the audio.
        XCTAssertEqual(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: true, liveSegments: segments,
                                                   lastNonSilentAt: at(800)), at(300))
        XCTAssertNil(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: true, liveSegments: [],
                                                 lastNonSilentAt: at(800)))
        // Off or not ready: the last audible moment.
        XCTAssertEqual(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: false, liveSegments: segments,
                                                   lastNonSilentAt: at(800)), at(800))
        // Never audible: `lastNonSilentAt` is still the audio start's reset.
        XCTAssertNil(StillInMeeting.lastSpeechAt(recordingStartedAt: start, liveListening: false, liveSegments: [],
                                                 lastNonSilentAt: at(0.4)))
    }

    func testPromptShownBeforeSleepIsReshownOnWake() {
        // Shown at 900 s, slept from 905 s to 4000 s: shown again at wake, with 2 fresh minutes.
        let reshown = StillInMeeting.promptShownAt(now: at(4000), lastPollAt: at(905), promptShownAt: at(900))
        XCTAssertEqual(reshown, at(4000))
        XCTAssertEqual(StillInMeeting.state(now: at(4000), recordingStartedAt: start, lastSpeechAt: nil,
                                            promptShownAt: reshown, keptAt: nil), .prompt)
        // Awake polls keep the original window, so no answer still stops at 2 minutes.
        XCTAssertEqual(StillInMeeting.promptShownAt(now: at(1020), lastPollAt: at(1015), promptShownAt: at(900)), at(900))
        // A short sleep inside the window keeps it too; no prompt stays none.
        XCTAssertEqual(StillInMeeting.promptShownAt(now: at(960), lastPollAt: at(905), promptShownAt: at(900)), at(900))
        XCTAssertNil(StillInMeeting.promptShownAt(now: at(4000), lastPollAt: at(905), promptShownAt: nil))
    }

    func testNotificationButtonsAnswerThePrompt() {
        let category = StillInMeeting.notificationCategory
        XCTAssertEqual(NotificationClickHandler.click(category: category, action: StillInMeeting.keepAction, hasLink: false), .stillInMeeting(keep: true))
        XCTAssertEqual(NotificationClickHandler.click(category: category, action: StillInMeeting.stopAction, hasLink: false), .stillInMeeting(keep: false))
        XCTAssertEqual(NotificationClickHandler.click(category: category, action: UNNotificationDefaultActionIdentifier, hasLink: false), .activate)
        XCTAssertEqual(NotificationClickHandler.click(category: "budget", action: StillInMeeting.stopAction, hasLink: false), .openAIProviders)
    }
}
