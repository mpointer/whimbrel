import XCTest
@testable import Whimbrel

final class JSONLLoggerTrimTests: XCTestCase {
    private struct Record: Codable { let n: Int; let text: String }

    private func makeLogger() throws -> (JSONLLogger, URL) {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent("log.jsonl")
        return (JSONLLogger(url: url), url)
    }

    private func size(_ url: URL) -> UInt64 {
        (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? UInt64) ?? 0
    }

    private func lines(_ url: URL) -> [String] {
        let text = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        return text.split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
    }

    func testTrimsWhenEverythingWasRead() throws {
        let (logger, url) = try makeLogger()
        for n in 0..<3 { logger.append(Record(n: n, text: "line \(n)")) }
        logger.waitUntilWritten()
        XCTAssertTrue(logger.trimIfFullyRead(through: size(url)))
        XCTAssertEqual(size(url), 0)

        logger.append(Record(n: 3, text: "after"))
        logger.waitUntilWritten()
        let data = try Data(contentsOf: url)
        XCTAssertFalse(data.contains(0), "no zero bytes from writing past a truncated end")
        XCTAssertEqual(lines(url).count, 1)
        XCTAssertTrue(lines(url)[0].contains("after"))
    }

    func testBeforeTruncateRunsFirstAndOnlyWhenTrimming() throws {
        let (logger, url) = try makeLogger()
        logger.append(Record(n: 0, text: "one"))
        logger.waitUntilWritten()
        var calls: [UInt64] = []
        XCTAssertFalse(logger.trimIfFullyRead(through: 1) { calls.append(self.size(url)) })
        XCTAssertTrue(calls.isEmpty, "not called when unread bytes remain")
        let full = size(url)
        XCTAssertTrue(logger.trimIfFullyRead(through: full) { calls.append(self.size(url)) })
        XCTAssertEqual(calls, [full], "called before the file is emptied")
    }

    func testKeepsUnreadBytes() throws {
        let (logger, url) = try makeLogger()
        logger.append(Record(n: 0, text: "first"))
        logger.waitUntilWritten()
        let afterFirst = size(url)
        logger.append(Record(n: 1, text: "second"))
        logger.append(Record(n: 2, text: "third"))
        logger.waitUntilWritten()
        XCTAssertFalse(logger.trimIfFullyRead(through: afterFirst))
        XCTAssertEqual(lines(url).count, 3)
    }

    func testConcurrentAppendsSurviveATrim() throws {
        let (logger, url) = try makeLogger()
        let writer = DispatchQueue(label: "writer")
        let done = expectation(description: "writes queued")
        writer.async {
            for n in 0..<500 { logger.append(Record(n: n, text: String(repeating: "x", count: 200))) }
            done.fulfill()
        }
        for _ in 0..<200 { _ = logger.trimIfFullyRead(through: size(url)) }
        wait(for: [done], timeout: 10)
        logger.waitUntilWritten()
        let data = try Data(contentsOf: url)
        XCTAssertFalse(data.contains(0))
        for line in lines(url) {
            XCTAssertNoThrow(try JSONDecoder().decode(Record.self, from: Data(line.utf8)), line)
        }
    }
}
