import XCTest
@testable import Whimbrel

/// The MCP server child gets the database key on its stdin pipe, never in argv or the
/// environment (X4, design 3.2). Exercised on a harmless stand-in (`/usr/bin/wc -c` counts
/// what arrives on stdin); never `McpServerProcessManager.start()`, which reaps any running
/// `whimbrel-mcp-server`.
final class McpServerStdinKeyTests: XCTestCase {
    private func run(key: Data?) throws -> (stdout: String, task: Process) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/wc")
        task.arguments = ["-c"]
        let out = Pipe()
        task.standardOutput = out
        try McpServerProcessManager.launch(task, sendingKey: key)
        task.waitUntilExit()
        let text = String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        return (text.trimmingCharacters(in: .whitespacesAndNewlines), task)
    }

    func testTheKeyArrivesOnStdinAndStdinIsClosed() throws {
        let key = Data((0..<32).map { UInt8($0 + 100) })
        let (count, task) = try run(key: key)
        XCTAssertEqual(count, "32", "exactly the 32 key bytes, then end of input")
        XCTAssertEqual(task.terminationStatus, 0)
        let hex = key.map { String(format: "%02x", $0) }.joined()
        XCTAssertFalse((task.arguments ?? []).joined(separator: " ").contains(hex))
        XCTAssertFalse((task.environment ?? [:]).values.contains { $0.lowercased().contains(hex) })
    }

    func testWithoutAKeyStdinIsEmptyAndClosed() throws {
        let (count, _) = try run(key: nil)
        XCTAssertEqual(count, "0", "the child sees end of input at once, never the app's stdin")
    }

    func testAKeyThatIsNot32BytesIsRefused() {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/true")
        XCTAssertThrowsError(try McpServerProcessManager.launch(task, sendingKey: Data(count: 16)))
        XCTAssertFalse(task.isRunning)
    }
}
