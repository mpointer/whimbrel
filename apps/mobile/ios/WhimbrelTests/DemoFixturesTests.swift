#if DEBUG
import XCTest
@testable import Whimbrel

@MainActor
final class DemoFixturesTests: XCTestCase {
    /// Every screen can be shown from the demo store: Today, Meetings, detail and Routines.
    func testDemoStoreServesEveryScreen() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("DemoFixturesTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let real = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("memory.sqlite")))
        let sync = SyncModel(store: DemoSyncStore(store: real), credentials: FakeCredentials())

        let today = TodayModel()
        await today.reload(sync: sync, now: Date(), calendar: .current)
        XCTAssertNotNil(today.brief, "a Daily Brief dated today")
        XCTAssertNotNil(today.upNext)
        XCTAssertNotNil(today.prep?.brief)
        XCTAssertEqual(today.recent.count, 10)

        let meetings = MeetingsModel()
        await meetings.reload(sync: sync)
        XCTAssertEqual(meetings.meetings.count, 12)
        XCTAssertEqual(meetings.devices.count, 2)

        let detail = MeetingDetailModel(meetingId: 1)
        await detail.reload(sync: sync)
        XCTAssertNotNil(detail.meeting?.summaryTldr)
        XCTAssertEqual(detail.transcriptTotal, 150)
        let noSummary = MeetingDetailModel(meetingId: 3)
        await noSummary.reload(sync: sync)
        XCTAssertNil(noSummary.meeting?.summaryTldr)

        let routines = RoutinesModel()
        await routines.reload(sync: sync)
        XCTAssertEqual(routines.runs.count, 4)
        XCTAssertTrue(routines.runs.contains { !$0.succeeded })
    }
}
#endif
