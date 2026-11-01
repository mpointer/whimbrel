import XCTest
@testable import Whimbrel

final class ImportTests: XCTestCase {
    private func outcome(meetings: Int = 0, notes: Int = 0, skipped: Int = 0, overlapping: Int = 0, failed: Int = 0) -> CoreImportOutcome {
        CoreImportOutcome(meetingIds: (0..<meetings).map(Int64.init), notes: notes, skipped: skipped, overlapping: overlapping, failed: Array(repeating: "x", count: failed))
    }

    func testTheSummaryLineSaysWhatHappened() {
        let text = ImportSummary.text(outcome(meetings: 2, skipped: 1, failed: 1), limit: 25)
        XCTAssertTrue(text.contains("2"))
        XCTAssertTrue(text.contains("1"))
        XCTAssertFalse(text.contains("25"))
    }

    func testItSaysWhenOnlyTheNewestAreSummarized() {
        XCTAssertTrue(ImportSummary.text(outcome(meetings: 30), limit: 25).contains("25"))
    }

    func testItSaysWhenRecordedMeetingsWereSkipped() {
        XCTAssertTrue(ImportSummary.text(outcome(overlapping: 7), limit: 25).contains("7"))
    }

    func testAnEmptyImportSaysSo() {
        XCTAssertEqual(ImportSummary.text(outcome(), limit: 25), "Nothing to import.")
    }

    func testAFileImportsThroughTheBridge() throws {
        let store = try XCTUnwrap(CoreMemoryStore(path: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".sqlite"), modelsDir: nil))
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".txt")
        try "Ana: Hello".write(to: file, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertEqual(store.importFiles([file], deviceId: "d")?.meetingIds.count, 1)
        XCTAssertEqual(store.importFiles([file], deviceId: "d")?.skipped, 1)
    }
}
