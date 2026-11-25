import WhimbrelUI
import XCTest
@testable import Whimbrel

/// The live meeting adapter and the recording notice default, flag on and off. Nothing here records or reads real audio.
final class LiveMeetingRedesignTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let d = UserDefaults(suiteName: "LiveMeetingRedesignTests")!
        d.removePersistentDomain(forName: "LiveMeetingRedesignTests")
        return d
    }

    // MARK: Notice default

    func testWithTheFlagOnAnAbsentSettingMeansOn() {
        XCTAssertTrue(RecordingNoticeSettings(defaults: defaults(), redesign: true).enabled)
    }

    func testTheAppsOwnValueIsOnWithNothingStoredAndAStoredOffStays() {
        let d = defaults()
        XCTAssertTrue(RecordingNoticeSettings(defaults: d, redesign: RedesignFlag.isOn()).enabled)
        RecordingNoticeSettings(defaults: d, redesign: RedesignFlag.isOn()).enabled = false
        XCTAssertFalse(RecordingNoticeSettings(defaults: d, redesign: RedesignFlag.isOn()).enabled)
    }

    func testWithTheFlagOffAnAbsentSettingStaysOff() {
        XCTAssertFalse(RecordingNoticeSettings(defaults: defaults(), redesign: false).enabled)
    }

    func testAStoredOffWinsWithTheFlagOn() {
        let d = defaults()
        let settings = RecordingNoticeSettings(defaults: d, redesign: true)
        settings.enabled = false
        XCTAssertFalse(RecordingNoticeSettings(defaults: d, redesign: true).enabled)
        settings.enabled = true
        XCTAssertTrue(RecordingNoticeSettings(defaults: d, redesign: false).enabled)
    }

    // MARK: State, time and text

    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    func testTheStateFollowsThePauseModel() {
        var pause = RecordingPauseModel(startedAt: start)
        func state(resuming: Bool = false, problem: String? = nil) -> WLiveState {
            LiveMeetingAdapter.state(isProcessing: false, stage: nil, isStarting: false, pause: pause, resuming: resuming, problem: problem)
        }
        XCTAssertEqual(state(), .recording)
        pause.apply(.pause, now: start.addingTimeInterval(60))
        XCTAssertEqual(state(), .paused)
        XCTAssertEqual(state(resuming: true), .resuming)
        pause.apply(.resumeFailed, now: start.addingTimeInterval(90))
        XCTAssertEqual(state(problem: "Microphone not found"), .problem("Microphone not found"))
        XCTAssertEqual(LiveMeetingAdapter.state(isProcessing: true, stage: "Transcribing", isStarting: false, pause: nil, resuming: false, problem: nil), .processing(stage: "Transcribing"))
        XCTAssertEqual(LiveMeetingAdapter.state(isProcessing: false, stage: nil, isStarting: true, pause: nil, resuming: false, problem: nil), .starting)
    }

    func testRecordedTimeLeavesOutThePause() {
        var pause = RecordingPauseModel(startedAt: start)
        pause.apply(.pause, now: start.addingTimeInterval(100))
        let now = start.addingTimeInterval(400)
        XCTAssertEqual(LiveMeetingAdapter.recordedSeconds(now: now, startedAt: start, pause: pause), 100)
        pause.apply(.resume, now: now)
        XCTAssertEqual(LiveMeetingAdapter.recordedSeconds(now: now.addingTimeInterval(50), startedAt: start, pause: pause), 150)
        XCTAssertEqual(LiveMeetingAdapter.recordedSeconds(now: now, startedAt: start, pause: nil), 400)
    }

    func testTheLabelIsTheLastNotesLine() {
        XCTAssertEqual(LiveMeetingAdapter.lastLine(of: "one\ntwo\n  three  \n\n"), "three")
        XCTAssertEqual(LiveMeetingAdapter.lastLine(of: ""), "")
    }

    func testTheTimeLineHasNextNowAndStarted() {
        let stops = LiveMeetingAdapter.stops(next: (start.addingTimeInterval(7200), "Design sync"), now: start, startedAt: start.addingTimeInterval(-900), started: "Started", nowWord: "Now")
        XCTAssertEqual(stops.map(\.kind), [.next, .now, .started])
        XCTAssertEqual(LiveMeetingAdapter.stops(next: nil, now: start, startedAt: nil, started: "Started", nowWord: "Now").map(\.kind), [.now])
    }

    func testMarksShowTheClockTimeOfDay() {
        let marker = CoreMeetingMarker(id: 7, meetingId: 1, atCs: 6000, label: "Priya: check churn", createdUnixMs: 1_800_000_000_000)
        let marks = LiveMeetingAdapter.marks([marker], locale: Locale(identifier: "en_US_POSIX"))
        XCTAssertEqual(marks.first?.id, "7")
        XCTAssertEqual(marks.first?.label, "Priya: check churn")
        XCTAssertFalse(marks.first?.timeOfDay.isEmpty ?? true)
    }
}
