import XCTest
@testable import Whimbrel

final class LiveAlertsSectionTests: XCTestCase {
    func testPausedLabelOnlyWhenPausedAndOn() {
        XCTAssertEqual(LiveAlertsSection.pausedLabel(status: .paused, alertsOn: true), "Question alerts paused")
        XCTAssertNil(LiveAlertsSection.pausedLabel(status: .paused, alertsOn: false))
        XCTAssertNil(LiveAlertsSection.pausedLabel(status: .listening, alertsOn: true))
        XCTAssertNil(LiveAlertsSection.pausedLabel(status: .idle, alertsOn: true))
        XCTAssertNil(LiveAlertsSection.pausedLabel(status: .unavailable("x"), alertsOn: true))
    }

    func testEmailStatusText() {
        XCTAssertEqual(LiveAlertsSection.emailStatusText("failed:no token"), "Email not sent: no token")
        XCTAssertNil(LiveAlertsSection.emailStatusText("sent"))
        XCTAssertNil(LiveAlertsSection.emailStatusText("off"))
        XCTAssertNil(LiveAlertsSection.emailStatusText(nil))
    }

    func testNearestStartPicksClosestLine() {
        XCTAssertNil(LiveAlertsSection.nearest([], to: 500))
        XCTAssertEqual(LiveAlertsSection.nearest([(1, 0), (2, 1_000), (3, 2_000)], to: 1_400), 2)
        XCTAssertEqual(LiveAlertsSection.nearest([(1, 0), (2, 1_000), (3, 2_000)], to: 9_000), 3)
    }

    // MARK: - Final review fixes

    private func alert(origin: String?, draftJson: String?, createdAtUnixMs: Int64 = 0) -> CoreLiveAlert {
        CoreLiveAlert(
            id: 1, meetingId: 3, originDeviceId: origin, sourceId: 1, sourceMeetingId: 3, meetingTitle: "Standup",
            quote: "Mike?", restatement: "Reply", asker: nil, atCs: 0, createdAtUnixMs: createdAtUnixMs, draftJson: draftJson,
            emailStatus: nil, notifiedAtUnixMs: nil
        )
    }

    func testAnotherDevicesAlertShowsNoChipsOnlyASourcesNote() {
        let cited = #"{"text":"We shipped v2 [1]","citations":[{"kind":"capture","id":9}]}"#
        var resolved = 0
        let resolve: ([CoreContextItemRef]) -> [LiveAlertDelivery.Evidence] = { refs in
            resolved += refs.count
            return refs.map { LiveAlertDelivery.Evidence(text: "Item \($0.id)", link: nil) }
        }
        let peer = LiveAlertsSection.row(alert(origin: "mac-2", draftJson: cited), resolve: resolve)
        XCTAssertEqual(peer.evidence, [])
        XCTAssertTrue(peer.sourcesElsewhere)
        XCTAssertEqual(peer.draft, "We shipped v2 [1]")
        XCTAssertEqual(resolved, 0, "the origin's local ids are never resolved here")

        let own = LiveAlertsSection.row(alert(origin: nil, draftJson: cited), resolve: resolve)
        XCTAssertEqual(own.evidence, [LiveAlertDelivery.Evidence(text: "Item 9", link: nil)])
        XCTAssertFalse(own.sourcesElsewhere)

        let uncited = LiveAlertsSection.row(alert(origin: "mac-2", draftJson: #"{"text":"Yes","citations":[]}"#), resolve: resolve)
        XCTAssertFalse(uncited.sourcesElsewhere)
        XCTAssertFalse(LiveAlertsSection.row(alert(origin: "mac-2", draftJson: nil), resolve: resolve).sourcesElsewhere)
    }

    func testPollsOnlyWhileRecordingWithAlertsOn() {
        XCTAssertTrue(LiveAlertsSection.shouldPoll(recording: true, alertsOn: true))
        XCTAssertFalse(LiveAlertsSection.shouldPoll(recording: true, alertsOn: false))
        XCTAssertFalse(LiveAlertsSection.shouldPoll(recording: false, alertsOn: true))
        XCTAssertFalse(LiveAlertsSection.shouldPoll(recording: false, alertsOn: false))
    }

    func testKeepsPollingAfterTheRecordingForADraftOrHalfAnHour() {
        let now: Int64 = 100 * 60_000
        let drafted = LiveAlertsSection.row(alert(origin: nil, draftJson: #"{"text":"Yes","citations":[]}"#), resolve: { _ in [] })
        let drafting = LiveAlertsSection.row(alert(origin: nil, draftJson: nil, createdAtUnixMs: now - 5 * 60_000), resolve: { _ in [] })
        let stale = LiveAlertsSection.row(alert(origin: nil, draftJson: nil), resolve: { _ in [] })
        func poll(rows: [LiveAlertsSection.Row], endedMinutesAgo: Int64?) -> Bool {
            LiveAlertsSection.shouldPoll(recording: false, alertsOn: true, rows: rows,
                                         endedUnixMs: endedMinutesAgo.map { now - $0 * 60_000 }, nowUnixMs: now)
        }
        XCTAssertTrue(poll(rows: [], endedMinutesAgo: 29))
        XCTAssertFalse(poll(rows: [], endedMinutesAgo: 31))
        XCTAssertFalse(poll(rows: [drafted], endedMinutesAgo: 31))
        XCTAssertTrue(poll(rows: [drafting], endedMinutesAgo: 31), "a draft still on its way")
        XCTAssertFalse(poll(rows: [stale], endedMinutesAgo: 31), "a draft that never came stops being waited for")
        XCTAssertFalse(LiveAlertsSection.shouldPoll(recording: false, alertsOn: false, rows: [drafting], endedUnixMs: now, nowUnixMs: now))
    }
}
