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
        let old = [segment(endCs: 30_000, text: "hi")]
        XCTAssertFalse(StillInMeeting.hasNewSpeech(old: old, new: old))
        XCTAssertFalse(StillInMeeting.hasNewSpeech(old: old, new: old + [segment(endCs: 40_000, text: " ")]))
        XCTAssertTrue(StillInMeeting.hasNewSpeech(old: old, new: old + [segment(endCs: 20_000, text: "late")]))
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
