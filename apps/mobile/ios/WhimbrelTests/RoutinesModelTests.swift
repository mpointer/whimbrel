import XCTest
@testable import Whimbrel

@MainActor
final class RoutinesModelTests: XCTestCase {
    func testRunsLoadNewestFirstAndAreEmptyWithNoStore() async {
        let store = FakeStore()
        store.runs = [
            PhoneRoutineRun(id: 1, routineName: "Old", ranAtUnixMs: 100, status: "success", outputSummary: "a"),
            PhoneRoutineRun(id: 2, routineName: "New", ranAtUnixMs: 300, status: "failed", outputSummary: "b"),
            PhoneRoutineRun(id: 3, routineName: "Mid", ranAtUnixMs: 200, status: "success", outputSummary: "c"),
        ]
        let model = RoutinesModel()
        await model.reload(sync: SyncModel(store: store, credentials: FakeCredentials()))
        XCTAssertEqual(model.runs.map(\.id), [2, 3, 1])
        XCTAssertTrue(model.loaded)

        let empty = RoutinesModel()
        await empty.reload(sync: SyncModel(store: nil, credentials: FakeCredentials()))
        XCTAssertTrue(empty.runs.isEmpty)
        XCTAssertTrue(empty.loaded)
    }

    func testRunStatusFlags() {
        XCTAssertTrue(PhoneRoutineRun(id: 1, routineName: "R", ranAtUnixMs: 1, status: "success", outputSummary: "").succeeded)
        XCTAssertFalse(PhoneRoutineRun(id: 1, routineName: "R", ranAtUnixMs: 1, status: "failed", outputSummary: "").succeeded)
    }
}
