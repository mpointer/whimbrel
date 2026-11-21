import XCTest
@testable import Whimbrel

/// The app's launch (X4 Task 11b-2) through the real C ABI, on an injected data root, a
/// throwaway Keychain service in the test host's in-memory Keychain and a `UserDefaults`
/// suite. Nothing here names the real data folder, the real defaults or the real Keychain
/// service, and nothing starts an MCP server (`startMcp` is a recorder).
@MainActor
final class AppLaunchTests: XCTestCase {
    private var container: URL!
    private var root: URL { container.appendingPathComponent("root", isDirectory: true) }
    private var suiteName: String!
    private var defaults: UserDefaults!
    private var keys: CountingKeys!

    /// `DbKeyStore` on a throwaway service, counting every write the launch makes.
    final class CountingKeys: LaunchKeyStore {
        let inner: DbKeyStore
        var writes: [String] = []
        init(service: String) { inner = DbKeyStore(service: service) }
        func lookup() -> DbKeyLookup { inner.lookup() }
        func lookupPending() -> DbKeyLookup { inner.lookupPending() }
        func storePending(_ key: Data) throws { writes.append("storePending"); try inner.storePending(key) }
        func storeKey(_ key: Data) throws { writes.append("storeKey"); try inner.storeKey(key) }
        func replaceAfterRecovery(proven key: Data, replacing rejected: Data?) throws {
            writes.append("replace"); try inner.replaceAfterRecovery(proven: key, replacing: rejected)
        }
        func promotePendingKey(openedWith opened: Data, replacing rejected: Data?) throws {
            writes.append("promote"); try inner.promotePendingKey(openedWith: opened, replacing: rejected)
        }
        func deletePending() throws { writes.append("deletePending"); try inner.deletePending() }
        func migrationWrite(_ key: Data, account: String) -> OSStatus {
            writes.append("migrationWrite:\(account)"); return inner.migrationWrite(key, account: account)
        }
    }

    override func setUp() {
        super.setUp()
        container = FileManager.default.temporaryDirectory.appendingPathComponent("app-launch-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        suiteName = "app-launch-tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)!
        keys = CountingKeys(service: "app-launch-tests.\(UUID().uuidString)")
    }

    override func tearDown() {
        try? keys.inner.delete()
        defaults.removePersistentDomain(forName: suiteName)
        try? FileManager.default.removeItem(at: container)
        super.tearDown()
    }

    static let words = (1...24).map { "w\($0)" }

    private func makeLaunch(onboardingKey: Data? = nil,
                            restoreFromWords: @escaping () -> Result<Data, CoreRecoveryError> = { .failure(.badWords) },
                            mcp: @escaping (Data?) -> Void) -> AppLaunch {
        let presenter = LaunchUIPresenter(setupActions: { existing in
            .init(minimumLength: 12, passphraseLength: { $0.count },
                  begin: { _ in .success(CoreSetupMaterial(key: existing ?? onboardingKey ?? Data(count: 32),
                                                           recoveryFile: "FILE", words: Self.words)) },
                  verify: { _, p in p == "correct horse battery" },
                  saveFile: { _ in .saved(path: "/Volumes/USB/x", sameDiskAsDatabase: false) },
                  printWords: { _ in .printed }, reusesKey: existing != nil)
        }, recoveryActions: { useKey in
            .init(chooseFile: { .cancelled }, restoreFromFile: { _, _ in .failure(.malformed) },
                  restoreFromWords: { _ in restoreFromWords() }, useKey: useKey)
        })
        let env = LaunchEnvironment(dataRoot: root, modelsDir: nil, keys: keys, defaults: defaults)
        return AppLaunch(environment: env, presenter: presenter, physicalMemory: { 32 << 30 }, startMcp: mcp)
    }

    private func meeting(_ title: String) -> CoreNewMeeting {
        CoreNewMeeting(deviceId: "dev", startUnixMs: 1000, endUnixMs: nil, title: title,
                       attendees: [], trigger: .manual, audioPath: nil, status: .completed)
    }

    /// Every file under `dir` (relative path, size, modification time).
    private func snapshot(_ dir: URL) -> [String: String] {
        var out: [String: String] = [:]
        let fm = FileManager.default
        guard let e = fm.enumerator(at: dir, includingPropertiesForKeys: [.fileSizeKey, .contentModificationDateKey]) else { return out }
        for case let url as URL in e {
            let v = try? url.resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey])
            let rel = String(url.path.dropFirst(dir.path.count))
            out[rel] = "\(v?.fileSize ?? -1)@\(v?.contentModificationDate?.timeIntervalSince1970 ?? 0)"
        }
        return out
    }

    private func names(_ dir: URL) -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []).sorted()
    }

    private func header(_ url: URL) -> Data { (try? FileHandle(forReadingFrom: url).read(upToCount: 16)) ?? Data() }

    private func runFirstWindow(_ launch: AppLaunch) {
        let done = expectation(description: "first window")
        Task { @MainActor in
            await launch.firstWindowRendered()
            done.fulfill()
        }
        wait(for: [done], timeout: 10)
    }

    // MARK: The existing plain install

    /// The regression the whole task turns on: an existing plain install with no flags opens
    /// plain, with no prompt, no key, no migration and no new files beside what today's open
    /// makes, and nothing outside the injected root changes.
    func testAnExistingPlainInstallOpensPlainWithNoPromptNoKeyNoMigrationNoNewFiles() throws {
        // Two identical plain installs: one opened the way the app opened before X4, one by the launch.
        let control = container.appendingPathComponent("control", isDirectory: true)
        try FileManager.default.createDirectory(at: control, withIntermediateDirectories: true)
        let db = root.appendingPathComponent("core-memory.sqlite")
        let controlDb = control.appendingPathComponent("core-memory.sqlite")
        for path in [db, controlDb] {
            _ = try XCTUnwrap(CoreMemoryStore(path: path, modelsDir: nil)?.insertMeeting(meeting("Plain meeting")))
        }
        let sibling = container.appendingPathComponent("sibling", isDirectory: true)
        try FileManager.default.createDirectory(at: sibling, withIntermediateDirectories: true)
        try Data("untouched".utf8).write(to: sibling.appendingPathComponent("note.txt"))
        let outsideBefore = snapshot(sibling)
        let headerBefore = header(db)
        XCTAssertEqual(headerBefore.prefix(15), Data("SQLite format 3".utf8))

        // The real app's launch settings, read only (the test host shares its defaults).
        let launchKeys = [UserDefaultsLaunchSettings.fastSearchKey, UserDefaultsLaunchSettings.migrateAtNextLaunchKey,
                          UserDefaultsLaunchSettings.goodLaunchesKey]
        let realBefore = launchKeys.map { String(describing: UserDefaults.standard.object(forKey: $0)) }
        let todaysOpen = try XCTUnwrap(CoreMemoryStore(path: controlDb, modelsDir: nil))
        var mcp: [Data?] = []
        let launch = makeLaunch { mcp.append($0) }
        launch.startAndWait(timeout: 20)

        guard case let .opened(o, mcpAllowed)? = launch.result else { return XCTFail("no open: \(String(describing: launch.result))") }
        XCTAssertNil(o.key, "opened plain")
        XCTAssertNil(o.store.dbKey)
        XCTAssertEqual(o.store.recentMeetings().map(\.title), ["Plain meeting"])
        XCTAssertTrue(mcpAllowed)
        XCTAssertEqual(mcp.count, 1)
        XCTAssertNil(mcp.first ?? Data(), "the MCP server is started once, with no key")
        // No prompt, no screen, no onboarding.
        XCTAssertFalse(launch.presenter.needsWindow)
        guard case .starting = launch.presenter.stage else { return XCTFail("a screen showed: \(launch.presenter.stage)") }
        XCTAssertTrue(launch.coordinator.notes.isEmpty, "no note: \(launch.coordinator.notes)")
        // No key created, no flag or counter written.
        XCTAssertEqual(keys.writes, [])
        XCTAssertEqual(keys.lookup(), .absent)
        XCTAssertEqual(keys.lookupPending(), .absent)
        runFirstWindow(launch)
        XCTAssertEqual(keys.writes, [])
        let written = defaults.persistentDomain(forName: suiteName) ?? [:]
        XCTAssertTrue(written.isEmpty, "settings written: \(written)")
        // No migration, no new files: the same files as the control's open, the file still plain.
        XCTAssertEqual(header(db), headerBefore)
        // The one new file is Task 13c's "a store was opened here" marker (no user data).
        XCTAssertEqual(names(root), (names(control) + ["store_existed"]).sorted())
        for leftover in [".plain.bak", ".enc.tmp", ".migration"] {
            XCTAssertFalse(names(root).contains { $0.contains(leftover) }, leftover)
        }
        // Nothing outside the injected root.
        XCTAssertEqual(snapshot(sibling), outsideBefore)
        XCTAssertEqual(names(container), ["control", "root", "sibling"])
        XCTAssertNil(CoreMemoryStore.databaseKey, "the global key is only set by the live MCP closure")
        XCTAssertEqual(launchKeys.map { String(describing: UserDefaults.standard.object(forKey: $0)) }, realBefore,
                       "the real defaults are untouched")
        withExtendedLifetime(todaysOpen) {}
    }

    // MARK: A new install

    /// AppHost subscribes before a slow open finishes (an open over the 10 s wait, or a launch that
    /// showed a screen first) and reads `launchedStore` in its sink. It must see the store there:
    /// a `@Published` sink ran before the value was stored, so the open was dropped and the window
    /// stayed on "Starting Whimbrel…" (0.9.80 device check).
    func testASubscriberThatExistedBeforeTheOpenFinishedSeesTheOpenedStore() async throws {
        let db = root.appendingPathComponent("core-memory.sqlite")
        _ = try XCTUnwrap(CoreMemoryStore(path: db, modelsDir: nil)?.insertMeeting(meeting("Plain meeting")))
        let launch = makeLaunch(mcp: { _ in })
        var seen: [LaunchedStore?] = []
        let cancellable = launch.resultPublisher.sink { r in
            guard r != nil else { return }
            seen.append(launch.launchedStore)
        }
        launch.start()
        for _ in 0..<10000 where seen.isEmpty { try await Task.sleep(nanoseconds: 1_000_000) }
        cancellable.cancel()
        XCTAssertEqual(seen.count, 1)
        XCTAssertNotNil(seen.first ?? nil, "launchedStore was nil inside the sink: the open would be dropped")
        XCTAssertNotNil(seen.first??.store)
    }

    func testANewInstallRunsTheRecoverySetupThenOpensAnEncryptedStore() async throws {
        let setupKey = Data(repeating: 7, count: 32)
        var mcp: [Data?] = []
        let launch = makeLaunch(onboardingKey: setupKey) { mcp.append($0) }
        launch.start()
        for _ in 0..<5000 where !launch.presenter.waitingForUser { try await Task.sleep(nanoseconds: 1_000_000) }
        guard case let .onboarding(model) = launch.presenter.stage else { return XCTFail("\(launch.presenter.stage)") }
        XCTAssertEqual(names(root), [], "nothing is created before the recovery key exists")
        XCTAssertEqual(keys.writes, [])
        model.passphrase = "correct horse battery"
        model.confirmation = "correct horse battery"
        await model.createRecoveryKey()
        await model.saveFile()
        model.testPassphrase = "correct horse battery"
        await model.runTest()
        model.savedConfirmed = true
        await model.perform(.continueToEncrypt)
        for _ in 0..<10000 where launch.result == nil { try await Task.sleep(nanoseconds: 1_000_000) }
        guard case let .opened(o, _)? = launch.result else { return XCTFail("\(String(describing: launch.result))") }
        XCTAssertEqual(o.key, setupKey)
        XCTAssertEqual(keys.writes, ["storePending", "storeKey"], "pending first, then db-key")
        XCTAssertEqual(keys.lookup(), .found(setupKey))
        XCTAssertEqual(mcp, [setupKey])
        let db = root.appendingPathComponent("core-memory.sqlite")
        XCTAssertNotEqual(header(db).prefix(15), Data("SQLite format 3".utf8), "encrypted on disk")
    }

    /// The opt-in from Settings: the next launch migrates with a determinate progress screen.
    func testTheOptInMigratesAtTheNextLaunchWithAProgressScreen() async throws {
        let db = root.appendingPathComponent("core-memory.sqlite")
        _ = try XCTUnwrap(CoreMemoryStore(path: db, modelsDir: nil)?.insertMeeting(meeting("Before")))
        let key = try keys.inner.createPending()
        defaults.set(true, forKey: UserDefaultsLaunchSettings.migrateAtNextLaunchKey)
        var mcp: [Data?] = []
        let launch = makeLaunch { mcp.append($0) }
        var stages: [Double] = []
        let watch = launch.presenter.$stage.sink { if case let .migrating(f) = $0 { stages.append(f) } }
        launch.start()
        for _ in 0..<20000 where launch.result == nil { try await Task.sleep(nanoseconds: 1_000_000) }
        watch.cancel()
        guard case let .opened(o, _)? = launch.result else { return XCTFail("\(String(describing: launch.result))") }
        XCTAssertEqual(o.key, key)
        XCTAssertEqual(o.store.recentMeetings().map(\.title), ["Before"])
        XCTAssertFalse(stages.isEmpty, "the progress screen showed")
        XCTAssertEqual(stages, stages.sorted(), "never goes back")
        XCTAssertEqual(mcp, [key])
        XCTAssertFalse(launch.settings.migrateAtNextLaunch)
    }

    // MARK: Recovery, end to end

    /// The presenter and the coordinator together on a real encrypted file whose db-key is
    /// wrong: the recovery screen shows, a wrong restored key is reported on it with nothing
    /// written, the right one opens the database, and only then is it stored, with the
    /// rejected key kept under db-key-previous.
    func testARestoredKeyOpensTheDatabaseAfterAWrongOne() async throws {
        let k = Data((0..<32).map { UInt8(($0 * 13 + 5) % 256) })
        let wrong = Data(repeating: 0x5A, count: 32)
        let typo = Data(repeating: 0x33, count: 32)
        let db = root.appendingPathComponent("core-memory.sqlite")
        do {
            let (created, store) = CoreMemoryStore.openKeyed(path: db, modelsDir: nil, lookup: .found(k))
            XCTAssertEqual(created, .opened)
            _ = try XCTUnwrap(store?.insertMeeting(meeting("Encrypted with K")))
        }
        try keys.inner.store(wrong, account: DbKeyStore.keyAccount)
        var restored = [typo, k]
        var mcp: [Data?] = []
        let launch = makeLaunch(restoreFromWords: { .success(restored.removeFirst()) }) { mcp.append($0) }
        launch.start()
        for _ in 0..<5000 where !launch.presenter.waitingForUser { try await Task.sleep(nanoseconds: 1_000_000) }
        guard case let .recovery(model) = launch.presenter.stage else { return XCTFail("\(launch.presenter.stage)") }
        XCTAssertEqual(model.reason, .keyRejected)
        model.method = .words
        model.words = "typed words"
        await model.restore()
        XCTAssertEqual(model.error, EncryptionCopy.keyForAnotherDatabase)
        XCTAssertEqual(keys.writes, [], "a wrong key is never written")
        XCTAssertNil(launch.result)
        model.words = "the right words"
        await model.restore()
        XCTAssertTrue(model.restored)
        for _ in 0..<5000 where launch.result == nil { try await Task.sleep(nanoseconds: 1_000_000) }
        guard case let .opened(o, _)? = launch.result else { return XCTFail("\(String(describing: launch.result))") }
        XCTAssertEqual(o.key, k)
        XCTAssertEqual(o.store.recentMeetings().map(\.title), ["Encrypted with K"])
        XCTAssertEqual(keys.writes, ["replace"], "stored only after it opened the database")
        XCTAssertEqual(keys.lookup(), .found(k))
        XCTAssertEqual(keys.inner.lookupPrevious(), [wrong], "the rejected key is kept, never destroyed")
        XCTAssertEqual(mcp, [k])
    }

    // MARK: Restart (fix round 1)

    /// The restart script, run for real with stand-ins for `open` and the alert: it opens the
    /// app only once the old process is gone; still alive after the wait, it shows the
    /// "quit and open again" instruction and opens nothing (never a second instance on the
    /// same database).
    func testTheRestartOpensOnlyAfterTheOldProcessExited() throws {
        let log = container.appendingPathComponent("restart.log")
        let opener = container.appendingPathComponent("open.sh")
        let alerter = container.appendingPathComponent("alert.sh")
        try "#!/bin/sh\necho \"open $*\" >> \"\(log.path)\"\n".write(to: opener, atomically: true, encoding: .utf8)
        try "#!/bin/sh\necho \"alert $*\" >> \"\(log.path)\"\n".write(to: alerter, atomically: true, encoding: .utf8)
        for f in [opener, alerter] { try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: f.path) }

        func run(waitingFor pid: Int32, tenths: Int) throws {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/bin/sh")
            p.arguments = AppRestart.arguments(pid: pid, otherPid: nil, appPath: "/Applications/Whimbrel.app",
                                               message: "Quit and open again", opener: opener.path, alerter: alerter.path,
                                               waitTenths: tenths)
            try p.run()
            p.waitUntilExit()
        }
        // A process that has exited: opened.
        let done = Process()
        done.executableURL = URL(fileURLWithPath: "/usr/bin/true")
        try done.run()
        done.waitUntilExit()
        try run(waitingFor: done.processIdentifier, tenths: 20)
        XCTAssertEqual(try String(contentsOf: log, encoding: .utf8), "open -n /Applications/Whimbrel.app\n")
        // A process still running after the wait: the instruction, and no second instance.
        try FileManager.default.removeItem(at: log)
        let alive = Process()
        alive.executableURL = URL(fileURLWithPath: "/bin/sleep")
        alive.arguments = ["30"]
        try alive.run()
        defer { alive.terminate() }
        try run(waitingFor: alive.processIdentifier, tenths: 3)
        XCTAssertEqual(try String(contentsOf: log, encoding: .utf8), "alert Quit and open again\n")
    }

    /// If the helper cannot be written or the waiting shell cannot start, the user is told
    /// to quit and open Whimbrel again; nothing is terminated or opened.
    func testARestartThatCannotStartSaysToQuitAndOpenAgain() {
        var messages: [String] = []
        var terminated = 0
        var ran = 0
        AppRestart.relaunchAfterExit(waitingAlsoFor: nil,
                                     helperDir: container.appendingPathComponent("missing/dir", isDirectory: true),
                                     run: { _ in ran += 1 }, showMessage: { messages.append($0) }, terminate: { terminated += 1 })
        XCTAssertEqual(messages, [EncryptionCopy.restartFailed])
        XCTAssertEqual(ran, 0)
        XCTAssertEqual(terminated, 0)
        struct Refused: Error {}
        AppRestart.relaunchAfterExit(waitingAlsoFor: nil, helperDir: container,
                                     run: { _ in throw Refused() }, showMessage: { messages.append($0) }, terminate: { terminated += 1 })
        XCTAssertEqual(messages, [EncryptionCopy.restartFailed, EncryptionCopy.restartFailed])
        XCTAssertEqual(terminated, 0)
        AppRestart.relaunchAfterExit(waitingAlsoFor: nil, helperDir: container,
                                     run: { _ in ran += 1 }, showMessage: { messages.append($0) }, terminate: { terminated += 1 })
        XCTAssertEqual(ran, 1)
        XCTAssertEqual(terminated, 1, "started: this process quits so the shell can open the app")
        XCTAssertEqual(messages.count, 2)
    }

    // MARK: The live composition is the only place that names real paths

    func testOnlyTheLiveCompositionNamesTheRealDataFolderDefaultsOrKeychain() throws {
        let app = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel")
        for file in ["Support/AppLaunch.swift", "UI/LaunchScreens.swift", "UI/Settings/EncryptionSettingsLive.swift"] {
            let text = try String(contentsOf: app.appendingPathComponent(file), encoding: .utf8)
            for banned in ["SpikePaths", ".standard", "DbKeyStore()", "applicationSupportDirectory"] {
                XCTAssertFalse(text.contains(banned), "\(banned) in \(file)")
            }
        }
        let live = try String(contentsOf: app.appendingPathComponent("Support/AppLaunchLive.swift"), encoding: .utf8)
        XCTAssertTrue(live.contains("SpikePaths.root"))
    }
}

/// A second copy of Whimbrel must not open the database or start capture while one runs
/// (#873, and two copies that both auto-recorded every calendar meeting). Pure decision plus
/// the exclusive lock that settles two copies launching at the same moment.
final class SingleInstanceGuardTests: XCTestCase {
    private let own = SingleInstance.Running(pid: 100, bundleID: "com.example.whimbrel")

    func testProceedsWhenNoOtherCopyAndLockHeld() {
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [own], lock: .acquired), .proceed)
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [], lock: .acquired), .proceed)
    }

    func testIgnoresOtherBundlesAndTerminatedCopies() {
        let other = SingleInstance.Running(pid: 200, bundleID: "com.apple.finder")
        let dying = SingleInstance.Running(pid: 300, bundleID: "com.example.whimbrel", isTerminated: true)
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [own, other, dying], lock: .acquired), .proceed)
    }

    func testAnotherCopyWithTheSameBundleIdMeansAlreadyRunning() {
        let twin = SingleInstance.Running(pid: 200, bundleID: "com.example.whimbrel")
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [own, twin], lock: .acquired),
                       .alreadyRunning(otherPID: 200))
    }

    func testLostLockMeansAlreadyRunningEvenWhenNoCopyIsVisibleYet() {
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [own], lock: .heldByOther),
                       .alreadyRunning(otherPID: nil))
    }

    func testUnavailableLockFailsOpenUnlessAnotherCopyIsVisible() {
        let twin = SingleInstance.Running(pid: 200, bundleID: "com.example.whimbrel")
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [own], lock: .unavailable), .proceed)
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [own, twin], lock: .unavailable),
                       .alreadyRunning(otherPID: 200))
    }

    func testHeldByOtherIsAlreadyRunning() {
        XCTAssertEqual(SingleInstance.decide(ownPID: 100, bundleID: "com.example.whimbrel", running: [], lock: .heldByOther),
                       .alreadyRunning(otherPID: nil))
    }

    func testLockIsExclusiveAndReleasedWhenDropped() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("single-instance-\(UUID().uuidString).lock")
        defer { try? FileManager.default.removeItem(at: url) }
        var first: SingleInstance.Lock? = SingleInstance.Lock.take(url: url).lock
        XCTAssertNotNil(first)
        let second = SingleInstance.Lock.take(url: url)
        XCTAssertNil(second.lock)
        XCTAssertEqual(second.result, .heldByOther, "a second holder is refused while the first lives")
        first = nil
        XCTAssertEqual(SingleInstance.Lock.take(url: url).result, .acquired, "released when the first holder goes away")
    }

    func testLockThatCannotBeCreatedIsUnavailableNotHeld() {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("no-such-dir-\(UUID().uuidString)/x.lock")
        let taken = SingleInstance.Lock.take(url: url)
        XCTAssertNil(taken.lock)
        XCTAssertEqual(taken.result, .unavailable)
    }

    func testTheAlertCopyIsPlain() {
        XCTAssertEqual(SingleInstance.message, "Whimbrel is already running.")
        XCTAssertFalse(SingleInstance.message.contains("\u{2014}"))
    }
}
