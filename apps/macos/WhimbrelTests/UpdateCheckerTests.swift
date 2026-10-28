import XCTest
@testable import Whimbrel

final class UpdateCheckerTests: XCTestCase {
    // MARK: - Manifest decode + version compare (core/api::update, through the bridge).
    // The version-compare, decode and hash cases themselves are Rust tests
    // (core/api/src/update.rs); these prove the Swift/Rust JSON plumbing.

    func testNewerManifestDecodesAllSevenFieldsThroughTheBridge() throws {
        let json = """
        {"version": "0.3.12", "url": "https://example.com/Whimbrel-0.3.12.zip", "sha256": "abc123", "commit": "a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3", "tag": "v0.3.12", "published_at": "2026-08-18T14:32:00Z", "release_notes_url": "https://github.com/mpointer/kestrel/releases/tag/v0.3.12"}
        """.data(using: .utf8)!
        let check = try XCTUnwrap(CoreMemoryStore.checkUpdateManifest(json: json, currentVersion: "0.3.11"))
        XCTAssertTrue(check.newer)
        XCTAssertEqual(check.manifest.version, "0.3.12")
        XCTAssertEqual(check.manifest.sha256, "abc123")
        XCTAssertEqual(check.manifest.commit, "a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3")
        XCTAssertEqual(check.manifest.tag, "v0.3.12")
        XCTAssertEqual(check.manifest.publishedAt, "2026-08-18T14:32:00Z")
        XCTAssertEqual(check.manifest.releaseNotesURL, "https://github.com/mpointer/kestrel/releases/tag/v0.3.12")
    }

    func testLegacyTwoFieldManifestDecodesWithNilNewFieldsAndEqualVersionIsNotNewer() throws {
        let json = """
        {"version": "0.3.11", "url": "https://example.com/Whimbrel-0.3.11.zip"}
        """.data(using: .utf8)!
        let check = try XCTUnwrap(CoreMemoryStore.checkUpdateManifest(json: json, currentVersion: "0.3.11"))
        XCTAssertFalse(check.newer)
        XCTAssertNil(check.manifest.sha256)
        XCTAssertNil(check.manifest.releaseNotesURL)
    }

    func testMalformedManifestOrVersionNeverTriggersAPhantomUpdate() {
        XCTAssertNil(CoreMemoryStore.checkUpdateManifest(json: Data(#"{"url": "https://example.com/Whimbrel.zip"}"#.utf8), currentVersion: "0.2.0"))
        XCTAssertNil(CoreMemoryStore.checkUpdateManifest(json: Data("not json".utf8), currentVersion: "0.2.0"))
        let check = CoreMemoryStore.checkUpdateManifest(json: Data(#"{"version": "not-a-version", "url": "u"}"#.utf8), currentVersion: "0.2.0")
        XCTAssertEqual(check?.newer, false)
        let check2 = CoreMemoryStore.checkUpdateManifest(json: Data(#"{"version": "0.3.0", "url": "u"}"#.utf8), currentVersion: "also-not-a-version")
        XCTAssertEqual(check2?.newer, false)
    }

    // MARK: - shouldDownload

    func testShouldDownloadWhenNothingStagedYet() {
        XCTAssertTrue(UpdateChecker.shouldDownload(remoteVersion: "0.3.0", alreadyStagedVersion: nil))
    }

    func testShouldNotReDownloadTheSameStagedVersion() {
        XCTAssertFalse(UpdateChecker.shouldDownload(remoteVersion: "0.3.0", alreadyStagedVersion: "0.3.0"))
    }

    func testShouldDownloadANewerVersionEvenIfSomethingElseIsAlreadyStaged() {
        XCTAssertTrue(UpdateChecker.shouldDownload(remoteVersion: "0.4.0", alreadyStagedVersion: "0.3.0"))
    }

    // MARK: - verifySignedByExpectedTeam (fail-closed on anything but a real signed build)

    func testVerifySignedByExpectedTeamRejectsNonexistentPath() {
        let bogusPath = FileManager.default.temporaryDirectory.appendingPathComponent("does-not-exist-\(UUID().uuidString).app")
        XCTAssertFalse(UpdateChecker.verifySignedByExpectedTeam(at: bogusPath))
    }

    func testVerifySignedByExpectedTeamRejectsUnsignedBundle() {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("fake-\(UUID().uuidString).app")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        XCTAssertFalse(UpdateChecker.verifySignedByExpectedTeam(at: dir))
    }

    // MARK: - SHA-256 verification (core/api::update, through the bridge)

    func testSha256OfKnownFixtureVerifiesAndAMutationRefuses() throws {
        // "hello world\n" -- `echo "hello world" | shasum -a 256`.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("sha256-fixture-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let fixtureURL = dir.appendingPathComponent("fixture.txt")
        try "hello world\n".data(using: .utf8)!.write(to: fixtureURL)
        let mutatedURL = dir.appendingPathComponent("mutated.txt")
        try "hello worle\n".data(using: .utf8)!.write(to: mutatedURL)
        let known = "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"

        XCTAssertTrue(CoreMemoryStore.verifyUpdateSha256(path: fixtureURL, expectedSha256: known))
        XCTAssertTrue(CoreMemoryStore.verifyUpdateSha256(path: fixtureURL, expectedSha256: known.uppercased()), "case-insensitive")
        XCTAssertTrue(CoreMemoryStore.verifyUpdateSha256(path: fixtureURL, expectedSha256: nil), "legacy manifest without a hash")
        XCTAssertFalse(CoreMemoryStore.verifyUpdateSha256(path: mutatedURL, expectedSha256: known))
        XCTAssertFalse(CoreMemoryStore.verifyUpdateSha256(path: dir.appendingPathComponent("missing"), expectedSha256: known))
    }

    // MARK: - stagedAppBundle (the zip's bundle name is not pinned; rename design step 4a)

    private func makeTempDir(_ name: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("\(name)-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testStagedAppBundleFindsTheOneBundleWhateverItsName() throws {
        for name in ["Kestrel.app", "Whimbrel.app"] {
            let dir = try makeTempDir("staged")
            defer { try? FileManager.default.removeItem(at: dir) }
            try FileManager.default.createDirectory(at: dir.appendingPathComponent(name), withIntermediateDirectories: true)
            XCTAssertEqual(UpdateChecker.stagedAppBundle(in: dir)?.lastPathComponent, name)
        }
    }

    func testStagedAppBundleRefusesNoneSeveralOrAFileNamedLikeOne() throws {
        let empty = try makeTempDir("staged-empty")
        defer { try? FileManager.default.removeItem(at: empty) }
        XCTAssertNil(UpdateChecker.stagedAppBundle(in: empty))

        let two = try makeTempDir("staged-two")
        defer { try? FileManager.default.removeItem(at: two) }
        try FileManager.default.createDirectory(at: two.appendingPathComponent("Kestrel.app"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: two.appendingPathComponent("Whimbrel.app"), withIntermediateDirectories: true)
        XCTAssertNil(UpdateChecker.stagedAppBundle(in: two), "two bundles is ambiguous, not a guess")

        let file = try makeTempDir("staged-file")
        defer { try? FileManager.default.removeItem(at: file) }
        try Data().write(to: file.appendingPathComponent("Kestrel.app"))
        XCTAssertNil(UpdateChecker.stagedAppBundle(in: file), "a plain file is not a bundle")

        XCTAssertNil(UpdateChecker.stagedAppBundle(in: empty.appendingPathComponent("missing")))
    }

    // MARK: - RelaunchHelper.sh (run against fake bundles; `open` replaced by /usr/bin/true)

    private static let helperScript = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Whimbrel/Support/RelaunchHelper.sh")

    private func makeFakeBundle(at url: URL, version: String) throws {
        try FileManager.default.createDirectory(at: url.appendingPathComponent("Contents"), withIntermediateDirectories: true)
        try version.data(using: .utf8)!.write(to: url.appendingPathComponent("Contents/version.txt"))
    }

    private func fakeBundleVersion(at url: URL) -> String? {
        try? String(contentsOf: url.appendingPathComponent("Contents/version.txt"), encoding: .utf8)
    }

    /// A pid that has already exited, so the helper's wait loop returns at once.
    private func exitedPid() throws -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/true")
        try p.run()
        p.waitUntilExit()
        return p.processIdentifier
    }

    private func runHelper(staged: URL, current: URL) throws {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = [Self.helperScript.path, String(try exitedPid()), staged.path, current.path]
        var env = ProcessInfo.processInfo.environment
        env["RELAUNCH_OPEN_CMD"] = "/usr/bin/true"
        p.environment = env
        try p.run()
        p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0, "helper must always exit 0 and relaunch something")
    }

    func testRelaunchHelperSwapsInPlaceWhenTheNameIsUnchanged() throws {
        let root = try makeTempDir("relaunch-same")
        defer { try? FileManager.default.removeItem(at: root) }
        let current = root.appendingPathComponent("Applications/Kestrel.app")
        let staged = root.appendingPathComponent("staged/Kestrel.app")
        try makeFakeBundle(at: current, version: "old")
        try makeFakeBundle(at: staged, version: "new")

        try runHelper(staged: staged, current: current)

        XCTAssertEqual(fakeBundleVersion(at: current), "new")
        XCTAssertFalse(FileManager.default.fileExists(atPath: current.path + ".old"), "backup removed after a good swap")
    }

    func testRelaunchHelperInstallsARenamedBundleBesideTheOldOneAndRemovesTheOld() throws {
        let root = try makeTempDir("relaunch-rename")
        defer { try? FileManager.default.removeItem(at: root) }
        let current = root.appendingPathComponent("Applications/Kestrel.app")
        let staged = root.appendingPathComponent("staged/Whimbrel.app")
        try makeFakeBundle(at: current, version: "old")
        try makeFakeBundle(at: staged, version: "new")

        try runHelper(staged: staged, current: current)

        let target = root.appendingPathComponent("Applications/Whimbrel.app")
        XCTAssertEqual(fakeBundleVersion(at: target), "new")
        XCTAssertFalse(FileManager.default.fileExists(atPath: current.path), "old bundle removed only after the copy succeeded")
    }

    func testRelaunchHelperLeavesTheOldBundleWhenARenamedCopyFails() throws {
        let root = try makeTempDir("relaunch-rename-fail")
        defer { try? FileManager.default.removeItem(at: root) }
        let current = root.appendingPathComponent("Applications/Kestrel.app")
        let staged = root.appendingPathComponent("staged/Whimbrel.app") // never created: ditto fails
        try makeFakeBundle(at: current, version: "old")

        try runHelper(staged: staged, current: current)

        XCTAssertEqual(fakeBundleVersion(at: current), "old")
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("Applications/Whimbrel.app").path))
    }

    func testRelaunchHelperRestoresTheOriginalWhenAnInPlaceCopyFails() throws {
        let root = try makeTempDir("relaunch-same-fail")
        defer { try? FileManager.default.removeItem(at: root) }
        let current = root.appendingPathComponent("Applications/Kestrel.app")
        let staged = root.appendingPathComponent("staged/Kestrel.app") // never created: ditto fails
        try makeFakeBundle(at: current, version: "old")

        try runHelper(staged: staged, current: current)

        XCTAssertEqual(fakeBundleVersion(at: current), "old")
        XCTAssertFalse(FileManager.default.fileExists(atPath: current.path + ".old"))
    }
}
