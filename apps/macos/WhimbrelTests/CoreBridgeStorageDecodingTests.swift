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
        XCTAssertTrue(CompactCopy.message(r, volumeFree: nil).hasPrefix("Not enough free disk"))
    }

    func testNotEnoughDiskAsksOnlyForWhatIsMissing() throws {
        let r = try decode(#"{"ran":false,"reason":"not_enough_disk","need_bytes":3000000000,"file_bytes_before":1500000000,"file_bytes_after":1500000000}"#)
        func size(_ b: Int64) -> String { ByteCountFormatter.string(fromByteCount: b, countStyle: .file) }
        XCTAssertEqual(CompactCopy.message(r, volumeFree: 1_000_000_000),
                       "Not enough free disk to compact. Free \(size(2_000_000_000)) and try again.", "need minus what's already free")
        XCTAssertEqual(CompactCopy.message(r, volumeFree: nil),
                       "Not enough free disk to compact. Free \(size(3_000_000_000)) and try again.", "free unknown: the whole need")
        XCTAssertEqual(CompactCopy.message(r, volumeFree: 5_000_000_000),
                       "Not enough free disk to compact. Free \(size(3_000_000_000)) and try again.", "free grew since: never asks for 0")
    }

    func testNotWorthItSaysNothingToCompact() throws {
        let r = try decode(#"{"ran":false,"reason":"not_worth_it","need_bytes":null,"file_bytes_before":3000,"file_bytes_after":3000}"#)
        XCTAssertEqual(CompactCopy.message(r, volumeFree: nil), "Nothing to compact yet.")
        XCTAssertEqual(CompactCopy.message(nil, volumeFree: nil), "Couldn't compact the database.")
    }

    func testARunWhoseSpaceIsStillInTheWALSaysSo() throws {
        let r = try decode(#"{"ran":true,"reason":"ran","need_bytes":null,"file_bytes_before":3000,"file_bytes_after":2000,"space_pending":true}"#)
        XCTAssertEqual(r.spacePending, true)
        XCTAssertEqual(CompactCopy.message(r, volumeFree: nil), "Compacted. The freed space returns to the disk shortly.")
    }

    func testDecodesSpace() throws {
        let s = try JSONDecoder().decode(CoreDbSpace.self, from: Data(#"{"file_bytes":10,"free_bytes":4}"#.utf8))
        XCTAssertEqual(s, CoreDbSpace(fileBytes: 10, freeBytes: 4))
    }
}
