import XCTest
@testable import Whimbrel

final class FreeUpSpaceModelTests: XCTestCase {
    private let usage = CoreMediaUsage(audioFullBytes: 0, audioCompressedBytes: 500, imageBytes: 0)

    func testOptionsHideWhatFreesNothing() {
        let options = FreeUpSpaceModel.options(
            space: CoreDbSpace(fileBytes: 1000, freeBytes: 0), usage: usage,
            deletableAudio: { _ in 0 }, oldCaptures: { _ in (0, 0) }, days: 90)
        XCTAssertTrue(options.isEmpty)
    }

    func testOptionsShowEachCleanupWithWhatItFrees() {
        let options = FreeUpSpaceModel.options(
            space: CoreDbSpace(fileBytes: 1000, freeBytes: 400),
            usage: CoreMediaUsage(audioFullBytes: 2000, audioCompressedBytes: 0, imageBytes: 0),
            deletableAudio: { _ in 1500 }, oldCaptures: { _ in (12, 600) }, days: 90)
        XCTAssertEqual(options.map(\.kind), [.compactDatabase, .compressAudio, .deleteOldAudio(days: 90), .deleteOldCaptures(days: 90)])
        XCTAssertEqual(options.map(\.frees), [400, nil, 1500, 600])
    }

    func testOptionsUseTheChosenDays() {
        var asked: [Int] = []
        _ = FreeUpSpaceModel.options(space: nil, usage: nil,
                                     deletableAudio: { asked.append($0); return 0 },
                                     oldCaptures: { asked.append($0); return (0, 0) }, days: FreeUpSpaceModel.defaultDays)
        _ = FreeUpSpaceModel.options(space: nil, usage: nil,
                                     deletableAudio: { asked.append($0); return 0 },
                                     oldCaptures: { asked.append($0); return (0, 0) }, days: 30)
        XCTAssertEqual(asked, [90, 90, 30, 30])
    }

    // MARK: Finishing a cleanup

    private func compact(_ json: String) -> CoreCompactResult {
        try! JSONDecoder().decode(CoreCompactResult.self, from: Data(json.utf8))
    }

    func testAFailedForcedCompactSaysSoInsteadOfFreed() {
        let message = FreeUpSpaceModel.finishMessage(retentionFailed: false, forced: true, compacted: nil, volumeFree: nil,
                                                     before: 5000, after: 5000)
        XCTAssertEqual(message, CompactCopy.message(nil, volumeFree: nil))
        XCTAssertEqual(message, "Couldn't compact the database.")
    }

    func testAFailedCaptureDeleteSaysSoInsteadOfFreed() {
        let message = FreeUpSpaceModel.finishMessage(retentionFailed: true, forced: true, compacted: nil, volumeFree: nil,
                                                     before: 5000, after: 5000)
        XCTAssertEqual(message, "Couldn't delete old screen captures.")
    }

    func testARefusedForcedCompactShowsWhy() {
        let refused = compact(#"{"ran":false,"reason":"not_enough_disk","need_bytes":3000,"file_bytes_before":1500,"file_bytes_after":1500}"#)
        let message = FreeUpSpaceModel.finishMessage(retentionFailed: false, forced: true, compacted: refused, volumeFree: 1000,
                                                     before: 5000, after: 5000)
        XCTAssertEqual(message, CompactCopy.message(refused, volumeFree: 1000))
    }

    func testASuccessSaysWhatWasFreed() {
        let ran = compact(#"{"ran":true,"reason":"ran","need_bytes":null,"file_bytes_before":3000,"file_bytes_after":2000}"#)
        let freed = "Freed \(ByteCountFormatter.string(fromByteCount: 2_000_000, countStyle: .file))."
        XCTAssertEqual(FreeUpSpaceModel.finishMessage(retentionFailed: false, forced: true, compacted: ran, volumeFree: nil,
                                                      before: 5_000_000, after: 3_000_000), freed)
        XCTAssertEqual(FreeUpSpaceModel.finishMessage(retentionFailed: false, forced: false, compacted: nil, volumeFree: nil,
                                                      before: 5_000_000, after: 3_000_000), freed,
                       "an unforced compact that didn't run isn't a failure")
    }

    func testAnOlderRefreshLandingLastIsIgnored() {
        var generation = RefreshGeneration()
        let first = generation.begin()   // 90 days
        let second = generation.begin()  // the picker moved to 30 days
        XCTAssertTrue(generation.isCurrent(second))
        XCTAssertFalse(generation.isCurrent(first), "the 90-day refresh finishing last must not overwrite the 30-day options")
    }
}
