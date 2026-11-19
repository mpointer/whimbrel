import XCTest
@testable import Whimbrel

final class LiveAskHandoffTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_800_000_000)

    func testAHeldQuestionIsTakenOnce() {
        var h = LiveAskHandoff()
        h.hold("  What did Sam say about tiers? ", now: t0)
        XCTAssertTrue(h.isHolding)
        XCTAssertEqual(h.take(), "What did Sam say about tiers?")
        XCTAssertNil(h.take())
        XCTAssertFalse(h.isHolding)
    }

    func testAQuestionWaitsForALateAskBarAndExpiresAfterTheDeadline() {
        var h = LiveAskHandoff()
        h.hold("q", now: t0)
        XCTAssertFalse(h.isExpired(now: t0.addingTimeInterval(LiveAskHandoff.deadline - 0.1)))
        XCTAssertTrue(h.isExpired(now: t0.addingTimeInterval(LiveAskHandoff.deadline)))
        XCTAssertTrue(h.giveUp())
        XCTAssertNil(h.take())
        XCTAssertFalse(h.giveUp())
    }

    func testBlankTextIsNotHeldAndANewQuestionReplacesTheOldOne() {
        var h = LiveAskHandoff()
        h.hold("   ", now: t0)
        XCTAssertFalse(h.isHolding)
        h.hold("one", now: t0)
        h.hold("two", now: t0.addingTimeInterval(1))
        XCTAssertEqual(h.take(), "two")
    }

    func testNothingHeldIsNeverExpired() {
        XCTAssertFalse(LiveAskHandoff().isExpired(now: t0.addingTimeInterval(100)))
    }
}
