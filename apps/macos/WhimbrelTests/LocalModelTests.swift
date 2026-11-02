import XCTest
@testable import Whimbrel

final class LocalModelTests: XCTestCase {
    func testTheOfferNeedsTheHelperMemoryAndDisk() {
        let gb: UInt64 = 1 << 30
        XCTAssertTrue(LocalModel.canOffer(helperBundled: true, memory: 8 * gb, freeDisk: 4_000_000_000))
        XCTAssertFalse(LocalModel.canOffer(helperBundled: false, memory: 8 * gb, freeDisk: 4_000_000_000))
        XCTAssertFalse(LocalModel.canOffer(helperBundled: true, memory: 8 * gb - 1, freeDisk: 4_000_000_000))
        XCTAssertFalse(LocalModel.canOffer(helperBundled: true, memory: 8 * gb, freeDisk: 3_999_999_999))
    }

    func testTheModelLivesWithTheOtherModels() {
        XCTAssertEqual(LocalModel.modelPath.deletingLastPathComponent().lastPathComponent, "models")
        XCTAssertEqual(LocalModel.modelPath.pathExtension, "gguf")
    }

    func testConfiguringWithNoHelperIsSafe() {
        CoreMemoryStore.configureLocalModel(helper: nil, model: LocalModel.modelPath)
        CoreMemoryStore.stopLocalModel()
    }
}
