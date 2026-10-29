import XCTest
@testable import Whimbrel

@MainActor
final class UpdateCheckerStatusTests: XCTestCase {
    private func manifest(_ version: String) -> Data {
        #"{"version": "\#(version)", "url": "https://example.com/Whimbrel-\#(version).zip"}"#.data(using: .utf8)!
    }

    private func staged(_ version: String) -> UpdateChecker.DownloadedUpdate {
        .init(version: version, appPath: URL(fileURLWithPath: "/tmp/Whimbrel.app"))
    }

    func test_up_to_date_when_remote_not_newer() async {
        var downloads = 0
        let c = UpdateChecker(currentVersion: "1.0.0",
                              fetchManifest: { _ in self.manifest("1.0.0") },
                              downloader: { v, _, _ in downloads += 1; return self.staged(v) })
        await c.checkNow()
        XCTAssertEqual(c.status, .upToDate(version: "1.0.0"))
        XCTAssertEqual(downloads, 0)
    }

    func test_failed_when_fetch_fails() async {
        let c = UpdateChecker(currentVersion: "1.0.0", fetchManifest: { _ in nil })
        await c.checkNow()
        XCTAssertEqual(c.status, .failed)
    }

    func test_ready_when_already_staged() async {
        var downloads = 0
        let c = UpdateChecker(currentVersion: "1.0.0",
                              fetchManifest: { _ in self.manifest("1.1.0") },
                              downloader: { v, _, _ in downloads += 1; return self.staged(v) })
        await c.checkNow()
        await c.checkNow()
        XCTAssertEqual(c.status, .ready(version: "1.1.0"))
        XCTAssertEqual(downloads, 1, "second check must report ready without re-downloading")
    }

    func test_manual_check_while_checking_does_not_start_second_download() async {
        var downloads = 0
        let c = UpdateChecker(currentVersion: "1.0.0",
                              fetchManifest: { _ in self.manifest("1.1.0") },
                              downloader: { v, _, _ in
                                  downloads += 1
                                  try? await Task.sleep(nanoseconds: 200_000_000)
                                  return self.staged(v)
                              })
        async let first: Void = c.checkNow()
        async let second: Void = c.checkNow()
        _ = await (first, second)
        XCTAssertEqual(downloads, 1)
        XCTAssertEqual(c.status, .ready(version: "1.1.0"))
    }
}
