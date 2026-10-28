import XCTest
@testable import Whimbrel

final class ZoomLeaverTests: XCTestCase {
    func test_leave_matches_leave() {
        XCTAssertTrue(ZoomLeaveMatcher.isLeaveButton(title: "Leave", description: nil))
        XCTAssertTrue(ZoomLeaveMatcher.isLeaveButton(title: " leave ", description: nil))
        XCTAssertTrue(ZoomLeaveMatcher.isLeaveButton(title: nil, description: "Leave"))
    }

    func test_leave_never_matches_end_for_all() {
        for text in ["End meeting for all", "End Meeting for All", "End"] {
            XCTAssertFalse(ZoomLeaveMatcher.isLeaveButton(title: text, description: nil), text)
            XCTAssertFalse(ZoomLeaveMatcher.isLeaveButton(title: nil, description: text), text)
            XCTAssertFalse(ZoomLeaveMatcher.isLeaveMeetingConfirm(title: text, description: nil), text)
            XCTAssertFalse(ZoomLeaveMatcher.isLeaveMeetingConfirm(title: nil, description: text), text)
        }
    }

    func test_confirm_matches_leave_meeting() {
        XCTAssertTrue(ZoomLeaveMatcher.isLeaveMeetingConfirm(title: "Leave meeting", description: nil))
        XCTAssertTrue(ZoomLeaveMatcher.isLeaveMeetingConfirm(title: "Leave Meeting", description: nil))
        XCTAssertFalse(ZoomLeaveMatcher.isLeaveMeetingConfirm(title: "Leave", description: nil))
        XCTAssertFalse(ZoomLeaveMatcher.isLeaveButton(title: "Leave meeting", description: nil))
    }

    func test_nil_and_empty_never_match() {
        for (t, d) in [(nil, nil), ("", nil), (nil, ""), ("  ", "  ")] as [(String?, String?)] {
            XCTAssertFalse(ZoomLeaveMatcher.isLeaveButton(title: t, description: d))
            XCTAssertFalse(ZoomLeaveMatcher.isLeaveMeetingConfirm(title: t, description: d))
        }
    }

    func test_meeting_window_titles() {
        for t in ["Zoom Meeting", "Zoom Meeting 40-Minutes", "Zoom Webinar"] {
            XCTAssertTrue(ZoomLeaver.isMeetingWindowTitle(t), t)
        }
        for t in ["Zoom", "Zoom Workplace", "Settings"] {
            XCTAssertFalse(ZoomLeaver.isMeetingWindowTitle(t), t)
        }
        XCTAssertFalse(ZoomLeaver.isMeetingWindowTitle(nil))
    }
}
