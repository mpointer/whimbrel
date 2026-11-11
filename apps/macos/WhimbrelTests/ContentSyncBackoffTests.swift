import XCTest
@testable import Whimbrel

final class ContentSyncBackoffTests: XCTestCase {
    func testABacklogIsStalledOnlyOnceItsOldestChangeIsAnHourOld() {
        let now = Date()
        XCTAssertFalse(SyncTiming.backlogIsStalled(pending: 0, oldest: nil, now: now))
        XCTAssertFalse(SyncTiming.backlogIsStalled(pending: 5, oldest: now.addingTimeInterval(-59 * 60), now: now))
        XCTAssertTrue(SyncTiming.backlogIsStalled(pending: 5, oldest: now.addingTimeInterval(-61 * 60), now: now))
    }

    func testOutboxBacklogDecodesTheCoreJSON() throws {
        let b = try JSONDecoder().decode(CoreOutboxBacklog.self, from: Data(#"{"pending":3,"oldest_unix_ms":1000}"#.utf8))
        XCTAssertEqual(b.pending, 3)
        XCTAssertEqual(b.oldest, Date(timeIntervalSince1970: 1))
    }

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
        // A failure is a round that finished after the last success.
        let finished = failures > 0 ? now.addingTimeInterval(-60) : now.addingTimeInterval(-hours * 3600)
        return SyncTiming.isStale(now: now, lastSuccess: now.addingTimeInterval(-hours * 3600), lastRoundFinishedAt: finished,
                                  lastActivityAt: now.addingTimeInterval(-60), roundStartedAt: roundMinutes.map { now.addingTimeInterval(-$0 * 60) })
    }

    func testFailingForTwoHoursIsStale() {
        XCTAssertTrue(stale(hours: 2, failures: 1))
        XCTAssertFalse(stale(hours: 1.9, failures: 1))
    }

    func testAMacThatJustWokeIsNotStale() {
        let now = Date()
        XCTAssertFalse(SyncTiming.isStale(now: now, lastSuccess: now.addingTimeInterval(-12 * 3600), lastRoundFinishedAt: now.addingTimeInterval(-12 * 3600),
                                          lastActivityAt: now.addingTimeInterval(-30), roundStartedAt: nil))
    }

    func testAStuckRoundIsStale() {
        XCTAssertTrue(stale(hours: 3, roundMinutes: 6))
        XCTAssertFalse(stale(hours: 3, roundMinutes: 1))
    }

    func testNeverSyncedIsNotStale() {
        XCTAssertFalse(SyncTiming.isStale(now: Date(), lastSuccess: nil, lastRoundFinishedAt: nil, lastActivityAt: nil, roundStartedAt: nil))
    }
}
