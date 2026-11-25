import XCTest
@testable import Whimbrel

final class RecordingNoticeTests: XCTestCase {
    private struct Chat: ZoomChatPosting {
        let works: Bool
        func post(_ text: String) async -> Bool { works }
    }

    private func defaults() -> UserDefaults {
        let d = UserDefaults(suiteName: "RecordingNoticeTests")!
        d.removePersistentDomain(forName: "RecordingNoticeTests")
        return d
    }

    func testItIsOnUnlessAPersonTurnedItOff() {
        XCTAssertTrue(RecordingNoticeSettings(defaults: defaults()).enabled, "a value never stored means on")
        let d = defaults()
        RecordingNoticeSettings(defaults: d).enabled = false
        XCTAssertFalse(RecordingNoticeSettings(defaults: d).enabled, "a stored off stays off")
    }

    func testCustomWordingWinsAndPersists() {
        let d = defaults()
        let settings = RecordingNoticeSettings(defaults: d)
        XCTAssertFalse(settings.text.isEmpty)
        settings.customText = "Recording, everyone."
        XCTAssertEqual(RecordingNoticeSettings(defaults: d).text, "Recording, everyone.")
    }

    func testAChatLinePostsWhereThePlatformAllowsIt() async {
        let delivery = await RecordingNotice.deliver("x", chat: Chat(works: true))
        XCTAssertEqual(delivery, .posted)
    }

    func testAFailedPostFallsBackToAReminder() async {
        let delivery = await RecordingNotice.deliver("x", chat: Chat(works: false))
        XCTAssertEqual(delivery, .remind)
    }

    func testNoChatMeansAReminder() async {
        let delivery = await RecordingNotice.deliver("x", chat: nil)
        XCTAssertEqual(delivery, .remind)
    }
}
