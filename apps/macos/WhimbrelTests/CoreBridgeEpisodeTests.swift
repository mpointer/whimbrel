import XCTest
@testable import Whimbrel

/// Exercises Task 8's four episode-lifecycle wrappers
/// (`setEpisodeHidden`/`forgetEpisodeActivity`/`regenerateEpisodeSummary`/
/// `listHiddenEpisodeIds`) against a real `CoreMemoryStore` backed by a temp sqlite
/// file, mirroring `SettingsViewDuplicateCleanupTests.makeStore()`.
///
/// There's no FFI to directly insert an episode row, so `makeFinalizedEpisode`
/// mirrors `core/ffi/src/lib.rs`'s own `make_finalized_episode` test helper: ingest a
/// small JSONL capture-item fixture, then run the pending jobs batch, which clusters
/// and finalizes it into one real episode -- all through FFI Swift already wraps
/// (`ingestJSONL`/`runJobsBatch`/`recentEpisodes`).
final class CoreBridgeEpisodeTests: XCTestCase {
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

    /// Ingests a small capture-item fixture and runs the pending jobs batch to
    /// finalize it into one real episode. Returns the finalized episode's id and the
    /// device id it was ingested under.
    private func makeFinalizedEpisode(store: CoreMemoryStore) -> (episodeId: Int64, deviceId: String) {
        let jsonlURL = tempDir.appendingPathComponent("episode-lifecycle.jsonl")
        let deviceId = "episode-lifecycle-device"
        let lines = [
            #"{"timestamp":"1970-01-01T00:00:00Z","bundle_id":"com.app.a","window_title":"A","url":null,"text":"alpha one","source":"ax"}"#,
            #"{"timestamp":"1970-01-01T00:01:00Z","bundle_id":"com.app.a","window_title":"A","url":null,"text":"alpha two","source":"ax"}"#,
            #"{"timestamp":"1970-01-01T00:02:00Z","bundle_id":"com.app.b","window_title":"B","url":null,"text":"bravo one","source":"ax"}"#,
        ]
        try! Data(lines.joined(separator: "\n").utf8).write(to: jsonlURL)

        XCTAssertNotNil(store.ingestJSONL(path: jsonlURL, deviceId: deviceId), "precondition: ingest must succeed")
        XCTAssertGreaterThanOrEqual(store.runJobsBatch(maxJobs: 10), 0, "precondition: jobs batch must run")

        guard let episodeId = store.recentEpisodes(limit: 10)?.first?.id else {
            XCTFail("precondition: episode_segment job must have finalized one episode")
            return (-1, deviceId)
        }
        return (episodeId, deviceId)
    }

    func testSetEpisodeHiddenRoundTrips() {
        let store = makeStore()
        let (episodeId, deviceId) = makeFinalizedEpisode(store: store)

        XCTAssertTrue(store.setEpisodeHidden(id: episodeId, hidden: true))
        XCTAssertEqual(store.listHiddenEpisodeIds(deviceId: deviceId), [episodeId])

        XCTAssertTrue(store.setEpisodeHidden(id: episodeId, hidden: false))
        XCTAssertEqual(store.listHiddenEpisodeIds(deviceId: deviceId), [])
    }

    func testSetEpisodeHiddenReturnsFalseForUnknownId() {
        let store = makeStore()
        XCTAssertFalse(store.setEpisodeHidden(id: 999_999, hidden: true))
    }

    func testRegenerateEpisodeSummaryRoundTrips() {
        let store = makeStore()
        let (episodeId, _) = makeFinalizedEpisode(store: store)

        XCTAssertTrue(store.regenerateEpisodeSummary(id: episodeId))
    }

    func testRegenerateEpisodeSummaryReturnsFalseForUnknownId() {
        let store = makeStore()
        XCTAssertFalse(store.regenerateEpisodeSummary(id: 999_999))
    }

    func testForgetEpisodeActivityRoundTrips() {
        let store = makeStore()
        let (episodeId, _) = makeFinalizedEpisode(store: store)

        XCTAssertTrue(store.forgetEpisodeActivity(id: episodeId))
        // Second call: the episode is already gone.
        XCTAssertFalse(store.forgetEpisodeActivity(id: episodeId))
    }

    func testForgetEpisodeActivityReturnsFalseForUnknownId() {
        let store = makeStore()
        XCTAssertFalse(store.forgetEpisodeActivity(id: 999_999))
    }

    func testListHiddenEpisodeIdsReturnsEmptyOnFreshStore() {
        let store = makeStore()
        XCTAssertEqual(store.listHiddenEpisodeIds(deviceId: "no-such-device"), [])
    }
}
