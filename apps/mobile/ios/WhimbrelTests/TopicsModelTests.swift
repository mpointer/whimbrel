import XCTest
@testable import Whimbrel

@MainActor
final class TopicsModelTests: XCTestCase {
    private func sync(_ store: FakeStore?) -> SyncModel { SyncModel(store: store, credentials: FakeCredentials()) }

    /// As core's `whimbrel_topic_snapshot_json` writes it.
    private let fixture = #"""
    {"topic_key":"acme","name":"Acme renewal","kind":"customer","deleted":false,
     "lead":{"kind":"step","text":"Send the SOW","due_date":"2026-11-14"},
     "role":{"text":"Account lead","edited":true},
     "next_meeting":{"title":"Acme weekly","start_unix_ms":1763460000000,"attendees_by_org":[["acme.com",3],["partner.io",1]],"me":1},
     "you_owe":[
       {"text":"Send the SOW","source_kind":"meeting","source_title":"Acme kickoff","asked_at_unix_ms":1762000000000,"due_date":"2026-11-14","meeting":{"device_id":"mac-1","source_meeting_id":42}},
       {"text":"Share pricing","source_kind":"email","source_title":"Re: pricing","asked_at_unix_ms":null,"due_date":null,"meeting":null}
     ],
     "owed_to_you":[],"done":[],
     "recent":[{"kind":"email","title":"Re: pricing","sender":"Priya Nair","ts_unix_ms":1762100000000,"meeting":null}],
     "recent_total":1,"truncated":false,"built_at_unix_ms":1762200000000,"built_by":"MacBook"}
    """#

    func testDecodesSnapshotJSON() throws {
        let snap = try JSONDecoder().decode(PhoneTopicSnapshot.self, from: Data(fixture.utf8))
        XCTAssertEqual(snap.id, "acme")
        XCTAssertEqual(snap.name, "Acme renewal")
        XCTAssertEqual(snap.lead, .step(text: "Send the SOW", dueDate: "2026-11-14"))
        XCTAssertEqual(snap.youOwe.map(\.text), ["Send the SOW", "Share pricing"])
        XCTAssertEqual(snap.youOwe[0].meeting, PhoneMeetingRef(deviceId: "mac-1", sourceMeetingId: 42))
        XCTAssertNil(snap.youOwe[1].meeting)
        XCTAssertEqual(snap.recent.count, 1)
        XCTAssertEqual(snap.recent[0].kind, "email")
        XCTAssertEqual(snap.recent[0].sender, "Priya Nair")
        XCTAssertEqual(snap.nextMeeting?.attendeesByOrg, [.init(domain: "acme.com", count: 3), .init(domain: "partner.io", count: 1)])
        XCTAssertEqual(snap.nextMeeting.map(TopicText.attendees), "1 you · 3 acme.com · 1 partner.io")
        XCTAssertEqual(snap.role?.text, "Account lead")
        XCTAssertEqual(snap.builtBy, "MacBook")
        XCTAssertFalse(snap.truncated)

        let dated = try JSONDecoder().decode(PhoneTopicSnapshot.Lead.self, from: Data(#"{"kind":"date","label":"SOW due","date":"2026-11-14"}"#.utf8))
        XCTAssertEqual(dated, .date(label: "SOW due", date: "2026-11-14"))
    }

    /// Upcoming next meetings soonest first, then the rest by name. A next meeting that has
    /// already started counts as none, so Delta sorts by name after Beta, not first.
    func testSortsUpcomingMeetingsSoonestFirstThenByName() async {
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let hourMs: Int64 = 3_600_000
        func meeting(_ offsetMs: Int64) -> PhoneTopicSnapshot.NextMeeting {
            .init(title: "Weekly", startUnixMs: nowMs + offsetMs, attendeesByOrg: [], me: 1)
        }
        let store = FakeStore()
        store.topics = [
            PhoneTopicSnapshot(topicKey: "d", name: "Delta", nextMeeting: meeting(-hourMs)),
            PhoneTopicSnapshot(topicKey: "b", name: "Beta"),
            PhoneTopicSnapshot(topicKey: "c", name: "Cobalt", nextMeeting: meeting(2 * hourMs)),
            PhoneTopicSnapshot(topicKey: "a", name: "Acme", nextMeeting: meeting(hourMs)),
        ]
        let model = TopicsModel()
        await model.reload(sync: sync(store))
        XCTAssertEqual(model.topics.map(\.id), ["a", "c", "b", "d"])
        XCTAssertTrue(model.loaded)
    }

    func testEmptyWhenNoSnapshots() async {
        let model = TopicsModel()
        await model.reload(sync: sync(FakeStore()))
        XCTAssertTrue(model.topics.isEmpty)
        XCTAssertTrue(model.loaded)

        let unopened = TopicsModel()
        await unopened.reload(sync: sync(nil))
        XCTAssertTrue(unopened.topics.isEmpty)
    }

    func testPageLoadsAndResolvesItsMeetings() async {
        let store = FakeStore()
        let ref = PhoneMeetingRef(deviceId: "mac-1", sourceMeetingId: 42)
        store.meetingIds[ref] = 7
        store.topics = [PhoneTopicSnapshot(topicKey: "acme", name: "Acme",
                                           youOwe: [.init(text: "Send the SOW", sourceKind: "meeting", meeting: ref)])]
        let model = TopicPageModel(key: "acme")
        await model.reload(sync: sync(store))
        XCTAssertEqual(model.snapshot?.name, "Acme")
        XCTAssertEqual(model.meetingIds, [ref: 7])
        XCTAssertTrue(model.loaded)
    }

    func testMissingTopicIsLoadedNotLoading() async {
        let missing = TopicPageModel(key: "gone")
        await missing.reload(sync: sync(FakeStore()))
        XCTAssertNil(missing.snapshot)
        XCTAssertTrue(missing.loaded, "an absent page shows the missing state")

        let unopened = TopicPageModel(key: "acme")
        await unopened.reload(sync: sync(nil))
        XCTAssertNil(unopened.snapshot)
        XCTAssertTrue(unopened.loaded, "no store open shows the missing state, not Loading forever")
    }

    func testChipsResolveRecentMeetingsThroughTheSyncMap() {
        let store = FakeStore()
        let ref = PhoneMeetingRef(deviceId: "mac-1", sourceMeetingId: 42)
        store.meetingIds[ref] = 7
        let topics = [
            PhoneTopicSnapshot(topicKey: "acme", name: "Acme", recent: [.init(kind: "meeting", title: "Kickoff", tsUnixMs: 1, meeting: ref)]),
            PhoneTopicSnapshot(topicKey: "beta", name: "Beta", recent: [
                .init(kind: "meeting", title: "Kickoff", tsUnixMs: 1, meeting: ref),
                .init(kind: "meeting", title: "Not here", tsUnixMs: 1, meeting: PhoneMeetingRef(deviceId: "mac-2", sourceMeetingId: 42)),
            ]),
        ]
        let chips = TopicsModel.chipsByMeeting(topics, in: store)
        XCTAssertEqual(chips, [7: PhoneTopicChip(key: "acme", name: "Acme")], "first topic wins; an unknown ref names nothing")

        let copy = PhoneMeeting(id: 9, deviceId: "phone", startUnixMs: 0, title: "Kickoff",
                                alsoRecorded: [PhoneMeetingCopy(id: 7, deviceId: "mac-1", title: "Kickoff")])
        XCTAssertEqual(TopicsModel.chip(for: copy, in: chips)?.key, "acme", "a grouped call matches through its copies")
        XCTAssertNil(TopicsModel.chip(for: PhoneMeeting(id: 3, deviceId: "mac-1", startUnixMs: 0, title: "Other"), in: chips))
    }
}
