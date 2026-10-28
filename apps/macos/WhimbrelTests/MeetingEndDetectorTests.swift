import AVFoundation
import CoreAudio
import XCTest
@testable import Whimbrel

final class MeetingEndDetectorTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000_000)

    private func evaluate(
        now: Date,
        appRunning: Bool = true,
        isOnMeetTab: Bool? = nil,
        scheduledEndUnixMs: Int64? = nil,
        audioActive: Bool = true,
        captureSessionActive: Bool? = nil,
        state: inout MeetingEndDetector.PollState
    ) -> MeetingEndDetector.EndReason? {
        MeetingEndDetector.evaluate(
            now: now,
            recordingStartedAt: start,
            appRunning: appRunning,
            isOnMeetTab: isOnMeetTab,
            scheduledEndUnixMs: scheduledEndUnixMs,
            audioActive: audioActive,
            captureSessionActive: captureSessionActive,
            state: &state
        )
    }

    func testCaptureReleasedFiresOnceItsGraceOutlastsTheMinimumAge() {
        // Its clock must survive the round trip through core between polls: released
        // from 125 s, it hasn't been 20 s at 135 s, and has at 150 s.
        var state = MeetingEndDetector.PollState()
        XCTAssertNil(evaluate(now: start.addingTimeInterval(125), captureSessionActive: false, state: &state))
        XCTAssertNotNil(state.captureReleasedStartedAt)
        XCTAssertNil(evaluate(now: start.addingTimeInterval(135), captureSessionActive: false, state: &state))
        XCTAssertEqual(evaluate(now: start.addingTimeInterval(150), captureSessionActive: false, state: &state), .captureReleased)
    }

    func testCaptureActiveAgainResetsTheReleasedClock() {
        var state = MeetingEndDetector.PollState()
        _ = evaluate(now: start.addingTimeInterval(125), captureSessionActive: false, state: &state)
        XCTAssertNil(evaluate(now: start.addingTimeInterval(140), captureSessionActive: true, state: &state))
        XCTAssertNil(state.captureReleasedStartedAt)
        XCTAssertNil(evaluate(now: start.addingTimeInterval(150), captureSessionActive: false, state: &state))
    }

    func testAppQuitFiresInstantlyEvenWithinGracePeriod() {
        var state = MeetingEndDetector.PollState()
        let reason = evaluate(now: start.addingTimeInterval(5), appRunning: false, state: &state)
        XCTAssertEqual(reason, .appQuit)
    }

    func testHardCeilingDoesNotFireWithinGracePeriodEvenIfAlreadyPastCeiling() {
        // Regression: a calendar meeting whose scheduled end was already 11 minutes ago
        // when recording actually started (a late join/accept) must not be killed on the
        // very first poll — it needs at least minimumRecordingAgeBeforeAutoEndSeconds to run.
        let scheduledEnd = Int64(start.addingTimeInterval(-11 * 60).timeIntervalSince1970 * 1000)
        var state = MeetingEndDetector.PollState()
        let reason = evaluate(now: start.addingTimeInterval(5), scheduledEndUnixMs: scheduledEnd, state: &state)
        XCTAssertNil(reason, "must not fire the hard ceiling before the grace period elapses")
    }

    func testHardCeilingFiresOnceGracePeriodElapses() {
        let scheduledEnd = Int64(start.addingTimeInterval(-11 * 60).timeIntervalSince1970 * 1000)
        var state = MeetingEndDetector.PollState()
        let reason = evaluate(now: start.addingTimeInterval(125), scheduledEndUnixMs: scheduledEnd, state: &state)
        XCTAssertEqual(reason, .scheduledEndHardCeiling)
    }

    func testSilenceDoesNotFireWithinGracePeriod() {
        // Regression: a quiet first minute (people joining, waiting for others) must not
        // kill the recording before it has a chance to capture anything.
        var state = MeetingEndDetector.PollState()
        var reason: MeetingEndDetector.EndReason?
        for offset in stride(from: 5.0, through: 115.0, by: 5.0) {
            reason = evaluate(now: start.addingTimeInterval(offset), audioActive: false, state: &state)
        }
        XCTAssertNil(reason, "must not fire on silence before the grace period elapses")
    }

    func testSilenceFiresAfterGracePeriodAndRequiredWindow() {
        var state = MeetingEndDetector.PollState()
        var reason: MeetingEndDetector.EndReason?
        for offset in stride(from: 5.0, through: 185.0, by: 5.0) {
            reason = evaluate(now: start.addingTimeInterval(offset), audioActive: false, state: &state)
            if reason != nil { break }
        }
        XCTAssertEqual(reason, .silence)
    }

    func testAudioActivityResetsSilenceClock() {
        var state = MeetingEndDetector.PollState()
        _ = evaluate(now: start.addingTimeInterval(150), audioActive: false, state: &state)
        XCTAssertNotNil(state.silenceStartedAt)
        _ = evaluate(now: start.addingTimeInterval(155), audioActive: true, state: &state)
        XCTAssertNil(state.silenceStartedAt)
    }

    func testBrowserTabGoneDoesNotFireWithinGracePeriod() {
        var state = MeetingEndDetector.PollState()
        let reason = evaluate(now: start.addingTimeInterval(10), isOnMeetTab: false, state: &state)
        XCTAssertNil(reason)
    }

    func testBrowserTabGoneFiresAfterGracePeriodAndOwnGraceWindow() {
        var state = MeetingEndDetector.PollState()
        _ = evaluate(now: start.addingTimeInterval(130), isOnMeetTab: false, state: &state)
        let reason = evaluate(now: start.addingTimeInterval(150), isOnMeetTab: false, state: &state)
        XCTAssertEqual(reason, .browserTabGone)
    }

    func testReturningToMeetTabResetsBrowserTabGoneClock() {
        var state = MeetingEndDetector.PollState()
        _ = evaluate(now: start.addingTimeInterval(130), isOnMeetTab: false, state: &state)
        XCTAssertNotNil(state.browserTabGoneStartedAt)
        _ = evaluate(now: start.addingTimeInterval(135), isOnMeetTab: true, state: &state)
        XCTAssertNil(state.browserTabGoneStartedAt)
    }

    func testShorterSilenceWindowAppliesOncePastScheduledEnd() {
        let scheduledEnd = Int64(start.addingTimeInterval(150).timeIntervalSince1970 * 1000)
        var state = MeetingEndDetector.PollState()
        var reason: MeetingEndDetector.EndReason?
        for offset in stride(from: 155.0, through: 175.0, by: 5.0) {
            reason = evaluate(now: start.addingTimeInterval(offset), scheduledEndUnixMs: scheduledEnd, audioActive: false, state: &state)
            if reason != nil { break }
        }
        XCTAssertEqual(reason, .silence, "the shorter post-scheduled-end silence window should apply once grace has also passed")
    }

    func testManualMeetingSignalsOnlyEndAtMaxDuration() {
        // A manual meeting: no app, no tab, no schedule, audio always "active".
        var state = MeetingEndDetector.PollState()
        XCTAssertNil(evaluate(now: start.addingTimeInterval(3 * 3600 + 59 * 60), state: &state))
        XCTAssertEqual(evaluate(now: start.addingTimeInterval(4 * 3600), state: &state), .maxDuration)
    }

    func testNewSwiftOnlyReasonsDoNotCountAsCallGone() {
        let reasons: [MeetingEndDetector.EndReason] = [.checkInTimeout, .checkInStop, .durationCap, .lowDisk]
        XCTAssertEqual(reasons.map(\.rawValue), ["check_in_timeout", "check_in_stop", "duration_cap", "low_disk"])
        XCTAssertTrue(reasons.allSatisfy { !$0.callWentAway })
    }
}

final class AccessQueueWaitProbeTests: XCTestCase {
    func testSummarizeReportsNearestRankPercentilesAndMax() {
        let waits = (1...100).map(Double.init)
        let s = AccessQueueWaitProbe.summarize(waits)
        XCTAssertEqual(s?.jobs, 100)
        XCTAssertEqual(s?.p50, 50)
        XCTAssertEqual(s?.p95, 95)
        XCTAssertEqual(s?.max, 100)
    }

    func testOneLongHoldShowsAsTheMax() {
        let s = AccessQueueWaitProbe.summarize([0.2, 0.3, 0.1, 30_000])
        XCTAssertEqual(s?.p50, 0)
        XCTAssertEqual(s?.max, 30_000)
        XCTAssertNil(AccessQueueWaitProbe.summarize([]))
    }
}

// MARK: - Mic capture health (AudioCaptureService)

final class StreamHealthTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000_000)

    private func health(stalledFor seconds: TimeInterval, restarts: Int) -> AudioCaptureService.StreamHealthAction {
        AudioCaptureService.streamHealth(lastBufferAt: start, restarts: restarts, now: start.addingTimeInterval(seconds))
    }

    func testAFlowingStreamIsOk() {
        XCTAssertEqual(health(stalledFor: 4.9, restarts: 0), .ok)
    }

    func testAFirstStallRestartsAndASecondIsDegraded() {
        XCTAssertEqual(health(stalledFor: 5, restarts: 0), .restart)
        XCTAssertEqual(health(stalledFor: 5, restarts: 1), .degraded)
    }
}

/// Drives the real mic capture through default-input switches. Needs real devices and
/// the mic permission, so it only runs with WHIMBREL_LIVE_AUDIO=1 (pass
/// TEST_RUNNER_WHIMBREL_LIVE_AUDIO=1 to xcodebuild). Speaks with `say` so the file has
/// speech to check, and restores the original default input. The system-audio test
/// also needs Screen & System Audio Recording, which a development-signed build doesn't
/// share with the installed app. The test host is the full app, so quit the installed
/// Whimbrel first: both would use the same database.
@MainActor
final class AudioCaptureLiveTests: XCTestCase {
    func testMicSurvivesDefaultInputSwitches() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["WHIMBREL_LIVE_AUDIO"] == "1")
        let original = try XCTUnwrap(AudioDeviceActivity.defaultInputDeviceID())
        let other = try XCTUnwrap(Self.inputDevices().first { $0 != original }, "needs a second input device")
        defer { Self.setDefaultInput(original) }

        let capture = AudioCaptureService()
        let began = Date()
        await capture.start()
        let say = Process()
        say.executableURL = URL(fileURLWithPath: "/usr/bin/say")
        say.arguments = ["Counting for the capture test. One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twenty one twenty two twenty three twenty four twenty five"]
        try say.run()
        try await Task.sleep(nanoseconds: 4_000_000_000)
        Self.setDefaultInput(other)
        try await Task.sleep(nanoseconds: 4_000_000_000)
        Self.setDefaultInput(original)
        try await Task.sleep(nanoseconds: 4_000_000_000)
        let elapsed = Date().timeIntervalSince(began)
        capture.stop()
        say.terminate()

        let url = try XCTUnwrap(capture.micAudioFileURL)
        let file = try AVAudioFile(forReading: url)
        let seconds = Double(file.length) / file.processingFormat.sampleRate
        print("[live] elapsed=\(elapsed) fileSeconds=\(seconds) format=\(file.processingFormat)")
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)))
        try file.read(into: buffer)
        let samples = UnsafeBufferPointer(start: buffer.floatChannelData![0], count: Int(buffer.frameLength))
        let rate = Int(file.processingFormat.sampleRate)
        for second in stride(from: 0, to: samples.count / rate, by: 1) {
            let window = samples[(second * rate)..<((second + 1) * rate)]
            let mean = window.reduce(0, +) / Float(rate)
            let rms = (window.reduce(0) { $0 + ($1 - mean) * ($1 - mean) } / Float(rate)).squareRoot()
            print(String(format: "[live] %2ds mean=%+.4f rms=%.4f", second, mean, rms))
        }
        XCTAssertEqual(seconds, elapsed, accuracy: 1.5)
    }

    func testSystemAudioReopensAfterAStallAndStaysAligned() async throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["WHIMBREL_LIVE_AUDIO"] == "1")
        let capture = AudioCaptureService()
        let began = Date()
        await capture.start()
        try XCTSkipUnless(capture.systemAudioActive, "needs Screen & System Audio Recording for this build")
        let say = Process()
        say.executableURL = URL(fileURLWithPath: "/usr/bin/say")
        say.arguments = ["System audio reopen test. One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty"]
        try say.run()
        try await Task.sleep(nanoseconds: 4_000_000_000)
        await capture.simulateSystemAudioStall()
        try await Task.sleep(nanoseconds: 10_000_000_000)
        XCTAssertTrue(capture.systemAudioActive, "reopened: \(capture.degradedReason ?? "no reason")")
        try await Task.sleep(nanoseconds: 3_000_000_000)
        let elapsed = Date().timeIntervalSince(began)
        let url = try XCTUnwrap(capture.systemAudioFileURL)
        capture.stop()
        say.terminate()
        try await Task.sleep(nanoseconds: 500_000_000)
        let file = try AVAudioFile(forReading: url)
        let seconds = Double(file.length) / file.processingFormat.sampleRate
        print("[live-system] elapsed=\(elapsed) fileSeconds=\(seconds)")
        XCTAssertEqual(seconds, elapsed, accuracy: 1.5)
    }

    private static func inputDevices() -> [AudioDeviceID] {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size)
        var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &ids)
        return ids.filter { id in
            var streams = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreams, mScope: kAudioDevicePropertyScopeInput, mElement: kAudioObjectPropertyElementMain)
            var bytes: UInt32 = 0
            return AudioObjectGetPropertyDataSize(id, &streams, 0, nil, &bytes) == noErr && bytes > 0
        }
    }

    private static func setDefaultInput(_ id: AudioDeviceID) {
        var device = id
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        AudioObjectSetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, UInt32(MemoryLayout<AudioDeviceID>.size), &device)
    }
}
