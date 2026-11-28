import CoreGraphics
import XCTest
@testable import Whimbrel

/// The reader against real Zoom windows recorded on 2026-11-06 (names replaced with Person A/B/C).
final class ZoomMeetingReaderTests: XCTestCase {
    private func fixture(_ name: String) throws -> [AXNodeSnapshot] {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("Fixtures/ZoomAX/\(name).json")
        return try JSONDecoder().decode([AXNodeSnapshot].self, from: Data(contentsOf: url))
    }

    func test_waiting_room() throws {
        XCTAssertEqual(ZoomMeetingReader.state(from: try fixture("waiting-room")), .waitingRoom)
    }

    func test_passcode_prompt() throws {
        XCTAssertEqual(ZoomMeetingReader.state(from: try fixture("passcode-prompt")), .passcodePrompt)
    }

    func test_in_meeting_muted_video_off() throws {
        XCTAssertEqual(ZoomMeetingReader.state(from: try fixture("in-meeting-muted-video-off")), .inMeeting(micMuted: true, videoOff: true))
    }

    func test_in_meeting_unmuted() throws {
        XCTAssertEqual(ZoomMeetingReader.state(from: try fixture("in-meeting-unmuted")), .inMeeting(micMuted: false, videoOff: true))
    }

    func test_chat_panel_open_is_still_in_meeting() throws {
        guard case .inMeeting = ZoomMeetingReader.state(from: try fixture("chat-panel-open")) else { return XCTFail("chat panel open is in a meeting") }
    }

    func test_removed() throws {
        XCTAssertEqual(ZoomMeetingReader.state(from: try fixture("removed-by-host")), .removed)
    }

    func test_ended() throws {
        XCTAssertEqual(ZoomMeetingReader.state(from: try fixture("meeting-ended")), .ended)
    }

    func test_no_zoom_windows_is_not_open() {
        XCTAssertEqual(ZoomMeetingReader.state(from: []), .notOpen)
    }

    /// An unreadable mic must never read as muted: Away Mode then leaves.
    func test_unknown_mic_reads_as_not_muted() {
        let window = AXNodeSnapshot(role: "AXWindow", subrole: nil, title: "Zoom Meeting", value: nil, description: nil, identifier: nil, children: [])
        XCTAssertEqual(ZoomMeetingReader.state(from: [window]), .inMeeting(micMuted: false, videoOff: false))
    }

}

/// Screen text capture reads other apps' windows. Each accessibility request is time-limited, so an app
/// that stops answering can't hold a read (Nov 19 2026), the read runs off the main thread so a slow one
/// can't stall the UI (2026-11-27), and another copy of Whimbrel is never read.
@MainActor
final class ScreenTextCaptureServiceTests: XCTestCase {
    private func source(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent(path)
        return try String(contentsOf: url, encoding: .utf8)
    }

    func testAnotherCopyOfWhimbrelIsNeverRead() {
        XCTAssertFalse(ScreenTextCaptureService.capturesText(of: "com.example.app", ownBundleId: "com.example.app"))
        XCTAssertTrue(ScreenTextCaptureService.capturesText(of: "com.apple.Safari", ownBundleId: "com.example.app"))
    }

    func testEveryRequestIsTimeLimited() throws {
        XCTAssertEqual(ScreenTextCaptureService.messagingTimeout, 1.0)
        let text = try source("Whimbrel/Capture/ScreenTextCaptureService.swift")
        for element in ["AXUIElementCreateSystemWide()", "appElement", "windowElement"] {
            XCTAssertTrue(text.contains("AXUIElementSetMessagingTimeout(\(element), timeout)"), element)
        }
    }

    /// The limit is the process-wide default from launch, so the meeting detectors' requests have it
    /// before screen capture has ever run.
    func testTheLimitIsSetAtLaunch() throws {
        let text = try source("Whimbrel/AppState.swift")
        XCTAssertTrue(text.contains("AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), ScreenTextCaptureService.messagingTimeout)"))
    }

    /// The window is read on the reader's own queue, never on the main thread.
    func testTheReadRunsOffTheMainThread() {
        let reader = ScreenTextReader()
        let done = expectation(description: "read finished")
        // pid 0 has no windows: the read ends early, which is enough to see where it ran.
        let target = ScreenTextReader.Target(pid: 0, bundleId: "com.example.none", appName: nil, skipWebAreas: false, excludedDomains: [])
        reader.read(target) { reading in
            XCTAssertFalse(Thread.isMainThread)
            XCTAssertNil(reading)
            done.fulfill()
        }
        wait(for: [done], timeout: 5)
    }

    func testAnExcludedSiteIsMatchedAgainstASnapshotOfTheList() {
        XCTAssertTrue(CaptureExclusions.isExcluded(url: "https://mail.example.com/inbox", domains: ["example.com"]))
        XCTAssertFalse(CaptureExclusions.isExcluded(url: "https://example.org/", domains: ["example.com"]))
        XCTAssertFalse(CaptureExclusions.isExcluded(url: "file:///tmp/a", domains: ["example.com"]))
    }
}

/// OCR is skipped for a window whose image has not changed since its last OCR.
final class OCRFingerprintTests: XCTestCase {
    private func image(gray: UInt8, width: Int = 200, height: Int = 100, stripeAt: Int? = nil) -> CGImage {
        var pixels = [UInt8](repeating: gray, count: width * height)
        if let stripeAt {
            for y in 0..<height { for x in stripeAt..<min(width, stripeAt + 40) { pixels[y * width + x] = 255 - gray } }
        }
        let provider = CGDataProvider(data: Data(pixels) as CFData)!
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: width,
                       space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: 0),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    }

    func testTheSameImageHasTheSameFingerprint() {
        XCTAssertEqual(OCRFallback.fingerprint(of: image(gray: 30)), OCRFallback.fingerprint(of: image(gray: 30)))
    }

    func testAVisibleChangeOrANewSizeChangesIt() {
        let base = OCRFallback.fingerprint(of: image(gray: 30))
        XCTAssertNotEqual(base, OCRFallback.fingerprint(of: image(gray: 30, stripeAt: 60)))
        XCTAssertNotEqual(base, OCRFallback.fingerprint(of: image(gray: 30, width: 220)))
    }
}
