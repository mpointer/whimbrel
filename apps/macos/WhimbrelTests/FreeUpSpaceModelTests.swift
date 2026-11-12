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
}
