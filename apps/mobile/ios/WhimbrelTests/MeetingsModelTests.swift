import XCTest
@testable import Whimbrel

@MainActor
final class MeetingsModelTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }()
    private func ms(_ day: Int, _ hour: Int, _ minute: Int = 0) -> Int64 {
        Int64(calendar.date(from: DateComponents(year: 2026, month: 10, day: day, hour: hour, minute: minute))!.timeIntervalSince1970 * 1000)
    }
    private func meeting(_ id: Int64, _ title: String, _ start: Int64, device: String = "a") -> PhoneMeeting {
        PhoneMeeting(id: id, deviceId: device, startUnixMs: start, title: title)
    }
    private func sync(_ store: FakeStore?) -> SyncModel { SyncModel(store: store, credentials: FakeCredentials()) }

    func testSectionsGroupByLocalDayNewestFirst() {
        let meetings = [meeting(3, "Late", ms(19, 0, 1)), meeting(2, "Night", ms(18, 23, 59)), meeting(1, "Morning", ms(18, 9))]
        let sections = MeetingsModel.sections(meetings, deviceFilter: nil, query: "", calendar: calendar)
        XCTAssertEqual(sections.map { $0.meetings.map(\.id) }, [[3], [2, 1]], "23:59 and 00:01 are different days")
        XCTAssertGreaterThan(sections[0].day, sections[1].day)
    }

    func testDeviceFilterAndSearch() {
        let meetings = [meeting(1, "Acme pricing", ms(19, 9), device: "a"), meeting(2, "Standup", ms(19, 8), device: "b"), meeting(3, "ACME renewal", ms(18, 9), device: "b")]
        XCTAssertEqual(MeetingsModel.sections(meetings, deviceFilter: "b", query: "", calendar: calendar).flatMap(\.meetings).map(\.id), [2, 3])
        XCTAssertEqual(MeetingsModel.sections(meetings, deviceFilter: nil, query: "acme", calendar: calendar).flatMap(\.meetings).map(\.id), [1, 3])
        XCTAssertEqual(MeetingsModel.sections(meetings, deviceFilter: "a", query: "acme", calendar: calendar).flatMap(\.meetings).map(\.id), [1])
        XCTAssertTrue(MeetingsModel.sections(meetings, deviceFilter: nil, query: "   ", calendar: calendar).flatMap(\.meetings).count == 3, "blank search shows everything")
    }

    func testListSaysWhenItIsCappedAtTwoHundred() async {
        let store = FakeStore()
        store.meetings = (1...250).map { meeting(Int64($0), "M\($0)", ms(19, 9)) }
        let model = MeetingsModel()
        await model.reload(sync: sync(store))
        XCTAssertEqual(model.meetings.count, 200)
        XCTAssertTrue(model.isCapped)
        store.meetings = (1...5).map { meeting(Int64($0), "M\($0)", ms(19, 9)) }
        await model.reload(sync: sync(store))
        XCTAssertFalse(model.isCapped)
        XCTAssertEqual(model.devices, ["a"])
    }

    func testDetailLoadsTranscriptInPagesAndHandlesNone() async {
        let store = FakeStore()
        store.meetings = [meeting(1, "Call", ms(19, 9)), meeting(2, "No transcript", ms(19, 8))]
        store.transcripts[1] = (0..<150).map { PhoneTranscriptSegment(id: Int64($0), startCs: Int64($0), speakerLabel: "S", text: "t\($0)") }
        store.items[1] = [PhoneActionItem(id: 1, text: "Send notes", completed: false, orderIndex: 0)]
        let sync = sync(store)
        let model = MeetingDetailModel(meetingId: 1)
        await model.reload(sync: sync)
        XCTAssertEqual(model.meeting?.title, "Call")
        XCTAssertEqual(model.actionItems.map(\.text), ["Send notes"])
        XCTAssertEqual(model.transcript.count, 100)
        XCTAssertEqual(model.transcriptTotal, 150)
        XCTAssertTrue(model.hasMoreTranscript)
        await model.loadMoreTranscript(sync: sync)
        XCTAssertEqual(model.transcript.count, 150)
        XCTAssertFalse(model.hasMoreTranscript)

        // A refresh (every pull) keeps the pages already loaded instead of jumping back to page 1.
        await model.reload(sync: sync)
        XCTAssertEqual(model.transcript.count, 150)
        XCTAssertEqual(Set(model.transcript.map(\.id)).count, 150, "no segment twice")

        let none = MeetingDetailModel(meetingId: 2)
        await none.reload(sync: sync)
        XCTAssertEqual(none.transcriptTotal, 0)
        XCTAssertTrue(none.loaded)
        XCTAssertFalse(none.hasMoreTranscript)

        let unopened = MeetingDetailModel(meetingId: 1)
        await unopened.reload(sync: SyncModel(store: nil, credentials: FakeCredentials()))
        XCTAssertFalse(unopened.loaded, "no store yet: still loading, not 'missing'")
    }
}
