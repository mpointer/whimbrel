import XCTest
@testable import Whimbrel

final class CallAutoStartTests: XCTestCase {
    func testCalendarMicOnlyWaits() {
        let d = CallAutoStart.decide(autoStartEnabled: true, source: .calendar, inCall: false)
        XCTAssertEqual(d.autoStart, false)
        XCTAssertEqual(d.waitForCall, true)
    }

    func testCalendarInCallAutoStarts() {
        let d = CallAutoStart.decide(autoStartEnabled: true, source: .calendar, inCall: true)
        XCTAssertEqual(d.autoStart, true)
        XCTAssertEqual(d.waitForCall, false)
    }

    func testGoogleCalendarWaitsToo() {
        let d = CallAutoStart.decide(autoStartEnabled: true, source: .googleCalendar, inCall: false)
        XCTAssertEqual(d.autoStart, false)
        XCTAssertEqual(d.waitForCall, true)
    }

    func testAutoStartOffNeverWaits() {
        let d = CallAutoStart.decide(autoStartEnabled: false, source: .calendar, inCall: false)
        XCTAssertEqual(d.autoStart, false)
        XCTAssertEqual(d.waitForCall, false)
    }

    func testDetectedUnchanged() {
        let d = CallAutoStart.decide(autoStartEnabled: true, source: .detected, inCall: false)
        XCTAssertEqual(d.autoStart, true)
        XCTAssertEqual(d.waitForCall, false)
    }

    func testInCallNeedsMicAndOutput() {
        XCTAssertFalse(CallAutoStart.inCall(mic: true, output: false))
        XCTAssertFalse(CallAutoStart.inCall(mic: false, output: true))
        XCTAssertTrue(CallAutoStart.inCall(mic: true, output: true))
    }
}
