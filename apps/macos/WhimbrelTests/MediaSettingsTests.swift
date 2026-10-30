import XCTest
@testable import Whimbrel

@MainActor
final class MediaSettingsTests: XCTestCase {
    private var suite: String!
    private var defaults: UserDefaults!
    private var tempDir: URL!

    override func setUp() {
        super.setUp()
        suite = "MediaSettingsTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)!
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent("MediaSettingsTests-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        // Restore write permission on anything a test locked, so cleanup can remove it.
        if let items = FileManager.default.enumerator(atPath: tempDir.path) {
            for case let rel as String in items {
                try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tempDir.appendingPathComponent(rel).path)
            }
        }
        try? FileManager.default.removeItem(at: tempDir)
        super.tearDown()
    }

    func testDefaults() {
        let s = MediaSettings(defaults: defaults)
        XCTAssertNil(s.folderPath)
        XCTAssertEqual(s.compressAfterDays, 30)
        XCTAssertEqual(s.deleteAudioAfterDays, 90)
        XCTAssertEqual(s.shrinkImagesAfterDays, 30)
        XCTAssertFalse(s.keepFullAudioForever)
        XCTAssertTrue(s.neverDeleteAudio, "deleting audio is off until the user turns it on")
        XCTAssertFalse(s.keepFullImagesForever)
        XCTAssertTrue(s.folderURL.path.hasSuffix("Pictures/Whimbrel"), s.folderURL.path)
        XCTAssertEqual(s.audioURL(deviceId: "mac-1"), s.folderURL.appendingPathComponent("Audio/mac-1", isDirectory: true))
        XCTAssertEqual(s.audioURL(deviceId: "a/b:c").lastPathComponent, "a-b-c", "one folder level, whatever the id")
        XCTAssertEqual(s.snapshotsURL, s.folderURL.appendingPathComponent("Snapshots", isDirectory: true))
        XCTAssertEqual(s.policy.compressAfterDays, 30)
        XCTAssertNil(s.policy.deleteAudioAfterDays)
        XCTAssertEqual(s.policy.shrinkImagesAfterDays, 30)

        s.folderPath = tempDir.path
        s.compressAfterDays = 7
        let reloaded = MediaSettings(defaults: defaults)
        XCTAssertEqual(reloaded.folderURL.path, tempDir.path)
        XCTAssertEqual(reloaded.compressAfterDays, 7)
    }

    func testDeleteAudioIsOffOnAFreshInstall() {
        let p = MediaSettings(defaults: defaults).policy
        XCTAssertNil(p.deleteAudioAfterDays)
        XCTAssertEqual(p.compressAfterDays, 30)
        XCTAssertEqual(p.shrinkImagesAfterDays, 30)
        XCTAssertEqual(p.deleteAudioPreviewAfterDays, 90, "Settings still measures what 90 days would free")
    }

    func testDeleteAudioIsOffAfterAnUpgradeWithNoStoredValue() {
        // An earlier build stored the ages but never the "Never delete" choice.
        defaults.set(45, forKey: "com.mpointer.kestrel.media.compressAfterDays")
        defaults.set(120, forKey: "com.mpointer.kestrel.media.deleteAudioAfterDays")
        let p = MediaSettings(defaults: defaults).policy
        XCTAssertNil(p.deleteAudioAfterDays)
        XCTAssertEqual(p.compressAfterDays, 45)
        XCTAssertEqual(p.deleteAudioPreviewAfterDays, 120)
    }

    func testTurningDeleteOnInSettingsPlansItAndIsRemembered() {
        let s = MediaSettings(defaults: defaults)
        s.neverDeleteAudio = false
        XCTAssertEqual(s.policy.deleteAudioAfterDays, 90)
        XCTAssertEqual(s.currentPolicy.deleteAudioAfterDays, 90)
        XCTAssertEqual(MediaSettings(defaults: defaults).policy.deleteAudioAfterDays, 90)
    }

    func testPolicyNilsTheOffChoices() throws {
        let s = MediaSettings(defaults: defaults)
        s.keepFullAudioForever = true
        s.neverDeleteAudio = true
        s.keepFullImagesForever = true
        XCTAssertNil(s.policy.compressAfterDays)
        XCTAssertNil(s.policy.deleteAudioAfterDays)
        XCTAssertNil(s.policy.shrinkImagesAfterDays)

        s.neverDeleteAudio = false
        let json = try JSONSerialization.jsonObject(with: JSONEncoder().encode(s.policy)) as? [String: Any]
        XCTAssertEqual(json?["delete_audio_after_days"] as? Int, 90)
        XCTAssertNotNil(MediaSettings(defaults: defaults).policy.deleteAudioAfterDays)
        XCTAssertNil(MediaSettings(defaults: defaults).policy.compressAfterDays)
    }

    func testRefusesFolderInsideDataFolder() throws {
        let data = tempDir.appendingPathComponent("data", isDirectory: true)
        let inside = data.appendingPathComponent("Media", isDirectory: true)
        try FileManager.default.createDirectory(at: inside, withIntermediateDirectories: true)
        let msg = "This folder is inside Whimbrel's data folder. Choose another."
        XCTAssertEqual(MediaSettings.refusal(for: inside, dataFolder: data), msg)
        XCTAssertEqual(MediaSettings.refusal(for: data, dataFolder: data), msg)
        // A sibling whose name merely starts with the data folder's name is not inside it.
        let sibling = tempDir.appendingPathComponent("data-media", isDirectory: true)
        try FileManager.default.createDirectory(at: sibling, withIntermediateDirectories: true)
        XCTAssertNil(MediaSettings.refusal(for: sibling, dataFolder: data))
    }

    func testRefusesSymlinkIntoDataFolder() throws {
        let data = tempDir.appendingPathComponent("data", isDirectory: true)
        let inside = data.appendingPathComponent("Media", isDirectory: true)
        try FileManager.default.createDirectory(at: inside, withIntermediateDirectories: true)
        let link = tempDir.appendingPathComponent("link-to-media")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: inside)
        XCTAssertEqual(MediaSettings.refusal(for: link, dataFolder: data),
                       "This folder is inside Whimbrel's data folder. Choose another.")
        XCTAssertEqual(probeFiles(in: inside), [])
    }

    func testRefusesWhenDataFolderHasTrailingSlash() throws {
        let data = tempDir.appendingPathComponent("data", isDirectory: true)
        let inside = data.appendingPathComponent("Media", isDirectory: true)
        try FileManager.default.createDirectory(at: inside, withIntermediateDirectories: true)
        let slashed = URL(fileURLWithPath: data.path + "/")
        XCTAssertEqual(MediaSettings.refusal(for: inside, dataFolder: slashed),
                       "This folder is inside Whimbrel's data folder. Choose another.")
        XCTAssertEqual(MediaSettings.refusal(for: URL(fileURLWithPath: inside.path + "/"), dataFolder: data),
                       "This folder is inside Whimbrel's data folder. Choose another.")
    }

    func testDataFolderCheckIgnoresCase() throws {
        // Paths that don't exist on disk, so symlink resolution can't canonicalize the case:
        // the comparison itself must ignore it (APFS is case-insensitive).
        let data = tempDir.appendingPathComponent("Data", isDirectory: true)
        let otherCase = URL(fileURLWithPath: tempDir.path + "/DATA/media", isDirectory: true)
        XCTAssertEqual(MediaSettings.refusal(for: otherCase, dataFolder: data),
                       "This folder is inside Whimbrel's data folder. Choose another.")
    }

    func testRefusesReadOnlyFolder() throws {
        let locked = tempDir.appendingPathComponent("locked", isDirectory: true)
        try FileManager.default.createDirectory(at: locked, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: locked.path)
        XCTAssertEqual(MediaSettings.refusal(for: locked, dataFolder: tempDir.appendingPathComponent("data")),
                       "Whimbrel can't write to this folder.")
        XCTAssertEqual(probeFiles(in: locked), [])
    }

    func testAcceptsWritableFolder() throws {
        let ok = tempDir.appendingPathComponent("media", isDirectory: true)
        try FileManager.default.createDirectory(at: ok, withIntermediateDirectories: true)
        XCTAssertNil(MediaSettings.refusal(for: ok, dataFolder: tempDir.appendingPathComponent("data")))
        // The probe file is cleaned up.
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: ok.path), [])
        XCTAssertEqual(probeFiles(in: ok), [])
    }

    private func probeFiles(in dir: URL) -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []).filter { $0.hasPrefix(".whimbrel-write-probe-") }
    }
}
