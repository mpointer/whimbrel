import XCTest
@testable import Whimbrel

final class ContentSyncBackoffTests: XCTestCase {
    func testHealthySyncKeepsTheNormalInterval() {
        XCTAssertEqual(SyncTiming.nextDelaySeconds(consecutiveFailures: 0), 60)
    }

    func testRepeatedFailuresBackOffAndCapAtFifteenMinutes() {
        let delays = (1...6).map { SyncTiming.nextDelaySeconds(consecutiveFailures: $0) }
        XCTAssertEqual(delays, [120, 240, 480, 900, 900, 900])
    }

    func testADeviceFarBehindComesBackSooner() {
        XCTAssertEqual(SyncTiming.nextDelaySeconds(consecutiveFailures: 0, moreToPull: true), 10)
    }

    func testFailuresBeatCatchingUp() {
        XCTAssertEqual(SyncTiming.nextDelaySeconds(consecutiveFailures: 2, moreToPull: true), 240)
    }

    private func stale(hours: Double, failures: Int = 0, roundMinutes: Double? = nil) -> Bool {
        let now = Date()
        return SyncTiming.isStale(now: now, lastSuccess: now.addingTimeInterval(-hours * 3600), consecutiveFailures: failures,
                                  roundStartedAt: roundMinutes.map { now.addingTimeInterval(-$0 * 60) })
    }

    func testFailingForTwoHoursIsStale() {
        XCTAssertTrue(stale(hours: 2, failures: 1))
        XCTAssertFalse(stale(hours: 1.9, failures: 1))
    }

    func testAMacThatJustWokeIsNotStale() {
        XCTAssertFalse(stale(hours: 12))
    }

    func testAStuckRoundIsStale() {
        XCTAssertTrue(stale(hours: 3, roundMinutes: 6))
        XCTAssertFalse(stale(hours: 3, roundMinutes: 1))
    }

    func testNeverSyncedIsNotStale() {
        XCTAssertFalse(SyncTiming.isStale(now: Date(), lastSuccess: nil, consecutiveFailures: 3, roundStartedAt: nil))
    }
}
