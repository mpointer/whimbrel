import XCTest
@testable import Whimbrel

/// The Mac's real ingest path (`CoreMemoryStore.ingestJSONL` → `whimbrel_memory_ingest_jsonl`
/// → `ingest::ingest_record` → `Store::insert_capture_record`) for captures longer than
/// one 2,000-char chunk. Before the chunk fix, every chunk after the first was deduped
/// away: all chunks of a record carry the record's hash, and the store compared each
/// chunk with the one just inserted. Store setup mirrors `CoreBridgeEpisodeTests`.
final class CoreBridgeIngestTests: XCTestCase {
    private var tempDir: URL!

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: tempDir)
        tempDir = nil
        super.tearDown()
    }

    private func makeStore() -> CoreMemoryStore {
        CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    /// One JSONL capture record, the shape `ScreenTextCaptureService` writes.
    private func writeRecord(_ name: String, text: String) -> URL {
        let url = tempDir.appendingPathComponent(name)
        let record: [String: Any] = [
            "timestamp": "2026-09-10T12:00:00Z",
            "bundle_id": "com.example.long",
            "window_title": "Long document",
            "url": NSNull(),
            "text": text,
            "source": "ax",
        ]
        try! JSONSerialization.data(withJSONObject: record).write(to: url)
        return url
    }

    /// "word0 word1 … word999": about 7,900 characters, which `ingest::chunk_text` splits
    /// into exactly 4 chunks. `core/ingest`'s `long_text_stores_every_chunk` asserts the
    /// same count for the same text, so the two suites agree on the fixture.
    private let longText = (0..<1000).map { "word\($0)" }.joined(separator: " ")

    func testLongCaptureStoresEveryChunk() {
        let store = makeStore()
        let stats = store.ingestJSONL(path: writeRecord("long.jsonl", text: longText), deviceId: "ingest-test-device")
        XCTAssertEqual(stats?.recordsRead, 1)
        XCTAssertEqual(stats?.itemsInserted, 4, "every chunk of a long capture is stored, not just the first")
        XCTAssertEqual(stats?.itemsDeduped, 0)
        XCTAssertEqual(stats?.parseErrors, 0)
    }

    func testRepeatingALongCaptureIsDedupedAsAWhole() {
        let store = makeStore()
        let url = writeRecord("long.jsonl", text: longText)
        XCTAssertEqual(store.ingestJSONL(path: url, deviceId: "ingest-test-device")?.itemsInserted, 4)
        let again = store.ingestJSONL(path: url, deviceId: "ingest-test-device")
        XCTAssertEqual(again?.itemsInserted, 0, "the same capture again adds nothing")
        XCTAssertEqual(again?.itemsDeduped, 4, "all of its chunks count as deduped")
    }

    func testShortCaptureIsStillOneItem() {
        let store = makeStore()
        let stats = store.ingestJSONL(path: writeRecord("short.jsonl", text: "a short capture"), deviceId: "ingest-test-device")
        XCTAssertEqual(stats?.itemsInserted, 1)
        XCTAssertEqual(stats?.itemsDeduped, 0)
    }
}
