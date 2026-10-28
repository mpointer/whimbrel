import XCTest
@testable import Whimbrel

/// Smoke test for the CoreBridge wrappers over core/api::models' job registry. The
/// download behaviour itself (size floors, unpack, truncation cleanup, cancel) is
/// tested in Rust (`core/api/src/models.rs`) against local HTTP fixtures; this proves
/// the Swift side can start, poll to completion and release a job.
final class ModelDownloadBridgeTests: XCTestCase {
    func testJobWithAllFilesPresentCompletesWithoutARequest() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let paths = ["w.bin", "s.onnx", "e.onnx"].map { dir.appendingPathComponent($0) }
        for p in paths { try Data("present".utf8).write(to: p) }

        guard let jobId = CoreMemoryStore.startModelDownload(whisperModelPath: paths[0], segmentationModelPath: paths[1], embeddingModelPath: paths[2]) else {
            return XCTFail("start returned nil")
        }
        defer { CoreMemoryStore.releaseModelDownload(jobId: jobId) }
        let deadline = Date().addingTimeInterval(10)
        while true {
            guard let p = CoreMemoryStore.pollModelDownload(jobId: jobId) else { return XCTFail("poll returned nil") }
            if p.done {
                XCTAssertEqual(p.fraction, 1.0)
                XCTAssertEqual(p.status, "All models ready.")
                XCTAssertFalse(p.cancelled)
                XCTAssertNil(p.error)
                break
            }
            XCTAssertLessThan(Date(), deadline, "job never finished")
            Thread.sleep(forTimeInterval: 0.01)
        }
        XCTAssertEqual(try Data(contentsOf: paths[0]), Data("present".utf8), "present files are untouched")
    }

    func testUnknownJobIdPollsNil() {
        XCTAssertNil(CoreMemoryStore.pollModelDownload(jobId: "nope"))
        XCTAssertFalse(CoreMemoryStore.cancelModelDownload(jobId: "nope"))
        CoreMemoryStore.releaseModelDownload(jobId: "nope")
    }
}
