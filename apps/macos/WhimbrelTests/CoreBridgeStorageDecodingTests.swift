import XCTest
@testable import Whimbrel

final class CoreBridgeStorageDecodingTests: XCTestCase {
    private func decode(_ json: String) throws -> CoreCompactResult {
        try JSONDecoder().decode(CoreCompactResult.self, from: Data(json.utf8))
    }

    func testDecodesARun() throws {
        let r = try decode(#"{"ran":true,"reason":"ran","need_bytes":null,"file_bytes_before":3000,"file_bytes_after":2000}"#)
        XCTAssertTrue(r.ran)
        XCTAssertNil(r.needBytes)
        XCTAssertEqual(r.fileBytesAfter, 2000)
    }

    func testDecodesNotEnoughDisk() throws {
        let r = try decode(#"{"ran":false,"reason":"not_enough_disk","need_bytes":3000,"file_bytes_before":3000,"file_bytes_after":3000}"#)
        XCTAssertEqual(r.needBytes, 3000)
        XCTAssertTrue(CompactCopy.message(r).hasPrefix("Not enough free disk"))
    }

    func testNotWorthItSaysNothingToCompact() throws {
        let r = try decode(#"{"ran":false,"reason":"not_worth_it","need_bytes":null,"file_bytes_before":3000,"file_bytes_after":3000}"#)
        XCTAssertEqual(CompactCopy.message(r), "Nothing to compact yet.")
        XCTAssertEqual(CompactCopy.message(nil), "Couldn't compact the database.")
    }

    func testARunWhoseSpaceIsStillInTheWALSaysSo() throws {
        let r = try decode(#"{"ran":true,"reason":"ran","need_bytes":null,"file_bytes_before":3000,"file_bytes_after":2000,"space_pending":true}"#)
        XCTAssertEqual(r.spacePending, true)
        XCTAssertEqual(CompactCopy.message(r), "Compacted. The freed space returns to the disk shortly.")
    }

    func testDecodesSpace() throws {
        let s = try JSONDecoder().decode(CoreDbSpace.self, from: Data(#"{"file_bytes":10,"free_bytes":4}"#.utf8))
        XCTAssertEqual(s, CoreDbSpace(fileBytes: 10, freeBytes: 4))
    }
}
