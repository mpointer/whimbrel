import XCTest
@testable import Whimbrel

final class CoreSelfTestLineScanTests: XCTestCase {
    func testFindsCompleteLineWhenNewlinePresent() {
        let data = "hello\nworld".data(using: .utf8)!
        XCTAssertEqual(CoreSelfTest.scanForCompleteLine(in: data, reachedEndOfFile: false), .foundLine(endIndex: 6))
    }

    func testWaitsForMoreDataWhenNoNewlineButAtEndOfFile() {
        let data = "partial line, no newline yet".data(using: .utf8)!
        XCTAssertEqual(CoreSelfTest.scanForCompleteLine(in: data, reachedEndOfFile: true), .waitForMoreData)
    }

    func testTreatsNoNewlineNotAtEndOfFileAsOversizedRatherThanWaitingForever() {
        // Regression test for the 2026-09-10 stalled-ingestion bug: a single captured
        // line (~526KB) exceeded the per-pass byte cap. The old code treated "no
        // newline in this read" as always meaning "wait for more data," which never
        // advanced the offset when the read genuinely hadn't reached the end of the
        // file — stalling ingestion on that exact byte forever.
        let data = Data(repeating: 0x41, count: 10)
        XCTAssertEqual(CoreSelfTest.scanForCompleteLine(in: data, reachedEndOfFile: false), .oversizedLine(bytesToSkip: 10))
    }
}
