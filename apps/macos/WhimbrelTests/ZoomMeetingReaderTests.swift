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
