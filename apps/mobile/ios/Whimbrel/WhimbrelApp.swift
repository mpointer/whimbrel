import OSLog
import SwiftUI

@main
struct WhimbrelApp: App {
    @StateObject private var enrollment: EnrollmentModel
    @StateObject private var sync: SyncModel

    init() {
        if let themesDirectory = ThemeStore.bundledThemesDirectory {
            ThemeStore.shared.install(ThemeLoader.loadDirectory(themesDirectory))
        }
        let enrollment = Self.makeEnrollment()
        let sync = SyncModel(openStore: Self.makeStore, credentials: enrollment)
        sync.settingsPuller = CoreSettingsPuller()
        if !Self.isDemo {
            // Revoked: close the store, then delete its directory (the demos use a temp store).
            // Marker first, so a wipe cut short by suspension or exit is finished at next launch.
            enrollment.onWipe = {
                StoreRemoval.markPending()
                sync.reset { StoreRemoval.finish(remove: Self.removeStoreDirectory) }
            }
            StoreRemoval.finishIfPending(remove: Self.removeStoreDirectory)
        }
        _enrollment = StateObject(wrappedValue: enrollment)
        _sync = StateObject(wrappedValue: sync)
    }

    var body: some Scene {
        WindowGroup { EnrollmentView(model: enrollment, sync: sync) }
    }

    private static func removeStoreDirectory() throws {
        try FileManager.default.removeItem(at: try CoreMemoryStore.defaultPath().deletingLastPathComponent())
    }

    private static var isDemo: Bool {
        #if DEBUG
        let arguments = ProcessInfo.processInfo.arguments
        return arguments.contains("-WhimbrelDemoEnrollment") || arguments.contains("-WhimbrelDemoNeedsDesktop") || arguments.contains("-WhimbrelDemoSynced")
        #else
        return false
        #endif
    }

    /// Runs on the sync model's store queue.
    private static func makeStore() -> MeetingSyncStore? {
        #if DEBUG
        // The demo never touches real state: a throwaway store in the temp directory (and
        // an in-memory Keychain, see KeychainAccess).
        if ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoSynced") {
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("WhimbrelDemo", isDirectory: true)
            try? FileManager.default.removeItem(at: dir)
            guard (try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)) != nil,
                  let store = CoreMemoryStore(path: dir.appendingPathComponent("memory.sqlite")) else { return nil }
            return DemoSyncStore(store: store)
        }
        #endif
        guard let path = try? CoreMemoryStore.defaultPath(), let store = CoreMemoryStore(path: path) else { return nil }
        return store
    }

    @MainActor
    private static func makeEnrollment() -> EnrollmentModel {
        #if DEBUG
        let arguments = ProcessInfo.processInfo.arguments
        if arguments.contains("-WhimbrelDemoEnrollment") || arguments.contains("-WhimbrelDemoNeedsDesktop") {
            let model = EnrollmentModel(backend: DemoEnrollmentBackend())
            Task { await model.signIn() }
            return model
        }
        if arguments.contains("-WhimbrelDemoSynced") {
            let model = EnrollmentModel(backend: DemoSyncedBackend())
            if model.status != .approved { Task { await model.signIn() } }
            return model
        }
        #endif
        return EnrollmentConfig.makeModel()
    }
}

/// The "store directory still to delete" marker: set before a wipe starts, cleared only once
/// the directory is gone, so an interrupted wipe finishes at the next launch.
enum StoreRemoval {
    static let pendingKey = "kestrel.sync.storeRemovalPending"
    private static let log = Logger(subsystem: "com.mpointer.whimbrel.mobile", category: "wipe")

    static func markPending(_ defaults: UserDefaults = .standard) {
        defaults.set(true, forKey: pendingKey)
    }

    static func finish(_ defaults: UserDefaults = .standard, remove: () throws -> Void) {
        do {
            try remove()
        } catch let error as CocoaError where error.code == .fileNoSuchFile {
            // Already gone.
        } catch {
            log.error("store directory removal failed: \(String(describing: type(of: error)), privacy: .public)")
            return
        }
        defaults.removeObject(forKey: pendingKey)
    }

    static func finishIfPending(_ defaults: UserDefaults = .standard, remove: () throws -> Void) {
        if defaults.bool(forKey: pendingKey) { finish(defaults, remove: remove) }
    }
}
