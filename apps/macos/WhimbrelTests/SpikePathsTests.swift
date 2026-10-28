import XCTest
@testable import Whimbrel

final class SpikePathsTests: XCTestCase {
    private var appSupport: URL!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        appSupport = fm.temporaryDirectory.appendingPathComponent("SpikePathsTests-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: appSupport, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? fm.removeItem(at: appSupport)
    }

    private var legacy: URL { appSupport.appendingPathComponent("littlebird/spike") }
    private var previous: URL { appSupport.appendingPathComponent(SpikePaths.previousFolderName) }
    private var whimbrel: URL { appSupport.appendingPathComponent("Whimbrel") }

    private func makeFolder(_ dir: URL, files: [String]) throws {
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        for name in files { try Data(name.utf8).write(to: dir.appendingPathComponent(name)) }
    }

    func testNewInstallCreatesWhimbrelFolder() {
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.root.lastPathComponent, "Whimbrel")
        XCTAssertNil(result.movedFrom)
        XCTAssertTrue(fm.fileExists(atPath: whimbrel.path))
    }

    func testMovesLegacyFolderAndRemovesItsEmptyParent() throws {
        try makeFolder(legacy, files: ["core-memory.sqlite", "mic-audio-1.caf"])
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.root.lastPathComponent, "Whimbrel")
        XCTAssertEqual(result.movedFrom?.path, legacy.path)
        XCTAssertTrue(fm.fileExists(atPath: whimbrel.appendingPathComponent("mic-audio-1.caf").path))
        XCTAssertFalse(fm.fileExists(atPath: legacy.deletingLastPathComponent().path), "an emptied littlebird/ parent goes")
    }

    func testLeavesANonEmptyLegacyParentAlone() throws {
        try makeFolder(legacy, files: ["core-memory.sqlite"])
        try makeFolder(legacy.deletingLastPathComponent(), files: ["little-bird-prod-auth.json"])
        _ = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertTrue(fm.fileExists(atPath: legacy.deletingLastPathComponent().appendingPathComponent("little-bird-prod-auth.json").path))
    }

    func testExistingWhimbrelDatabaseWinsAndLegacyIsUntouched() throws {
        try makeFolder(whimbrel, files: ["core-memory.sqlite"])
        try makeFolder(legacy, files: ["core-memory.sqlite"])
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.root.lastPathComponent, "Whimbrel")
        XCTAssertNil(result.movedFrom)
        XCTAssertTrue(fm.fileExists(atPath: legacy.appendingPathComponent("core-memory.sqlite").path))
    }

    func testStrayWhimbrelFolderWithoutDatabaseKeepsLegacyInUse() throws {
        try makeFolder(whimbrel, files: [])
        try makeFolder(legacy, files: ["core-memory.sqlite"])
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.root.path, legacy.path)
        XCTAssertNil(result.movedFrom)
    }

    func testMovesKestrelFolderToWhimbrel() throws {
        try makeFolder(previous, files: ["core-memory.sqlite", "mic-audio-1.caf"])
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.root.path, whimbrel.path)
        XCTAssertEqual(result.movedFrom?.path, previous.path)
        XCTAssertTrue(fm.fileExists(atPath: whimbrel.appendingPathComponent("mic-audio-1.caf").path))
        XCTAssertFalse(fm.fileExists(atPath: previous.path))
    }

    func testKestrelPreferredOverLittlebird() throws {
        try makeFolder(previous, files: ["core-memory.sqlite"])
        try makeFolder(legacy, files: ["core-memory.sqlite"])
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.movedFrom?.path, previous.path)
        XCTAssertTrue(fm.fileExists(atPath: legacy.appendingPathComponent("core-memory.sqlite").path), "littlebird is left alone")
    }

    func testExistingWhimbrelDatabaseWins() throws {
        try makeFolder(whimbrel, files: ["core-memory.sqlite"])
        try makeFolder(previous, files: ["core-memory.sqlite"])
        let result = SpikePaths.resolveRoot(appSupport: appSupport)
        XCTAssertEqual(result.root.path, whimbrel.path)
        XCTAssertNil(result.movedFrom)
        XCTAssertTrue(fm.fileExists(atPath: previous.appendingPathComponent("core-memory.sqlite").path))
    }

    func testSpikePathsNeverResolvesAgainstTheRealApplicationSupportUnderTheTestHost() {
        let real = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].standardizedFileURL.path
        XCTAssertFalse(SpikePaths.root.standardizedFileURL.path.hasPrefix(real + "/"))
        XCTAssertFalse(SpikePaths.appSupportDirectory.standardizedFileURL.path.hasPrefix(real))
    }

    func testLostMoveRaceUsesNewRoot() throws {
        // A FileManager whose move fails after another launch already produced Whimbrel.
        final class RacingFM: FileManager {
            var winner: URL!
            override func moveItem(at: URL, to: URL) throws {
                try createDirectory(at: winner, withIntermediateDirectories: true)
                try Data("db".utf8).write(to: winner.appendingPathComponent("core-memory.sqlite"))
                throw CocoaError(.fileWriteFileExists)
            }
        }
        try makeFolder(previous, files: ["core-memory.sqlite"])
        let racing = RacingFM()
        racing.winner = whimbrel
        let result = SpikePaths.resolveRoot(appSupport: appSupport, fileManager: racing)
        XCTAssertEqual(result.root.path, whimbrel.path)
        XCTAssertNil(result.movedFrom)
    }

    func testLegacyPrefixesAreReRootedEvenWhenNothingMovedThisLaunch() {
        let prefixes = SpikePaths.legacyPrefixes(appSupport: appSupport, root: whimbrel)
        XCTAssertEqual(prefixes.count, 2)
        let previousPath = previous.path + "/models/m.bin"
        XCTAssertEqual(SpikePaths.reroot(previousPath, prefixes: prefixes, root: whimbrel), whimbrel.path + "/models/m.bin")
        XCTAssertEqual(SpikePaths.reroot(legacy.path + "/x.caf", prefixes: prefixes, root: whimbrel), whimbrel.path + "/x.caf")
        XCTAssertNil(SpikePaths.reroot("/elsewhere/x.caf", prefixes: prefixes, root: whimbrel))
        XCTAssertTrue(SpikePaths.legacyPrefixes(appSupport: appSupport, root: legacy).isEmpty, "no re-root while a legacy folder is still in use")
    }
}
