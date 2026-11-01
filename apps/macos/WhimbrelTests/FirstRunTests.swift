import XCTest
@testable import Whimbrel

@MainActor
final class FirstRunTests: XCTestCase {
    func test_shows_once_and_only_on_a_new_mac() {
        XCTAssertTrue(FirstRun.shouldShow(done: false, microphoneGranted: false, screenGranted: false))
        XCTAssertFalse(FirstRun.shouldShow(done: true, microphoneGranted: false, screenGranted: false))
        XCTAssertFalse(FirstRun.shouldShow(done: false, microphoneGranted: true, screenGranted: false), "an existing install")
        XCTAssertFalse(FirstRun.shouldShow(done: false, microphoneGranted: false, screenGranted: true), "an existing install")
    }

    func test_first_run_asks_for_two_permissions_only() {
        XCTAssertEqual(PermissionKind.firstRun, [.microphone, .screenRecording])
    }

    func test_nothing_to_download_is_ready_at_once() async {
        let b = ModelBootstrap(present: { true }, download: { _ in XCTFail("no download needed") })
        await b.start()
        XCTAssertEqual(b.phase, .ready)
    }

    func test_download_reports_progress_then_ready() async {
        var seen: [Double] = []
        let b = ModelBootstrap(present: { false }, download: { progress in progress(0.5, "Downloading…"); seen.append(0.5) })
        await b.start()
        XCTAssertEqual(seen, [0.5])
        XCTAssertEqual(b.phase, .ready)
    }

    func test_failure_can_be_retried() async {
        var attempts = 0
        let b = ModelBootstrap(present: { false }, download: { _ in
            attempts += 1
            if attempts == 1 { throw CocoaError(.fileReadUnknown) }
        })
        await b.start()
        XCTAssertEqual(b.phase, .failed)
        await b.start()
        XCTAssertEqual(b.phase, .ready)
        XCTAssertEqual(attempts, 2)
    }

    func test_a_second_start_while_downloading_does_nothing() async {
        var attempts = 0
        let b = ModelBootstrap(present: { false }, download: { _ in
            attempts += 1
            try? await Task.sleep(for: .milliseconds(50))
        })
        async let first: Void = b.start()
        await Task.yield()
        await b.start()
        await first
        XCTAssertEqual(attempts, 1)
    }

    func testAMeetingWaitsForTheModelWhileItDownloads() {
        var polls = 0
        let ready = MeetingTranscriptionQueue.waitForModel(ready: { polls >= 3 }, downloading: { true }, pause: { polls += 1 })
        XCTAssertTrue(ready)
        XCTAssertEqual(polls, 3)
    }

    func testAFailedDownloadStopsTheWait() {
        var polls = 0
        let ready = MeetingTranscriptionQueue.waitForModel(ready: { false }, downloading: { polls < 2 }, pause: { polls += 1 })
        XCTAssertFalse(ready)
        XCTAssertEqual(polls, 2)
    }

    func testTheWaitHasALimit() {
        var polls = 0
        MeetingTranscriptionQueue.waitForModel(ready: { false }, downloading: { true }, pause: { polls += 1 }, limit: 5)
        XCTAssertEqual(polls, 5)
    }
}
