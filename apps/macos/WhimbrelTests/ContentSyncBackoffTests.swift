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
}
