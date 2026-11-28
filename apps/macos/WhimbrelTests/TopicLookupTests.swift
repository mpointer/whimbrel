import XCTest
@testable import Whimbrel

/// A meeting or calendar event with no topic: the core answers JSON `null`, which reads as no topic (#1073).
final class TopicLookupTests: XCTestCase {
    private var tempDir: URL!
    private var store: CoreMemoryStore!

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    override func tearDown() {
        store = nil
        try? FileManager.default.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    func testAMeetingWithNoTopicHasNone() {
        XCTAssertNil(store.topicForMeeting(meetingId: 424242))
    }

    func testAnEventWithNoTopicHasNone() {
        XCTAssertNil(store.topicForEvent(eventId: "no-such-event", title: "Coffee"))
    }
}
