import OSLog
import SwiftUI

@main
struct WhimbrelApp: App {
    @StateObject private var enrollment: EnrollmentModel
    @StateObject private var sync: SyncModel
    /// The coordinator holds the scheduler (the notification delegate); these two are the
    /// strong references that keep the enrollment and sync models alive for its weak seams.
    @StateObject private var coordinator: ReminderCoordinator
    @StateObject private var router: ReminderRouter
    @Environment(\.scenePhase) private var scenePhase

    init() {
        if let themesDirectory = ThemeStore.bundledThemesDirectory {
            ThemeStore.shared.install(ThemeLoader.loadDirectory(themesDirectory))
        }
        let enrollment = Self.makeEnrollment()
        let sync = SyncModel(openStore: Self.makeStore, credentials: enrollment)
        sync.settingsPuller = Self.settingsPuller(isDemo: Self.isDemo)
        if !Self.isDemo {
            // Revoked: close the store, then delete its directory (the demos use a temp store).
            // Marker first, so a wipe cut short by suspension or exit is finished at next launch.
            enrollment.onWipe = {
                StoreRemoval.markPending()
                sync.reset { StoreRemoval.finish(remove: Self.removeStoreDirectory) }
            }
            StoreRemoval.finishIfPending(remove: Self.removeStoreDirectory)
        }
        let router = ReminderRouter()
        let scheduler = NotificationScheduler(router: router)
        let coordinator = ReminderCoordinator(sync: sync, enrollment: enrollment, scheduler: scheduler,
                                              settings: ReminderSettingsStore(defaults: .standard), plan: PlanCache(defaults: .standard))
        let previousWipe = enrollment.onWipe
        enrollment.onWipe = {
            previousWipe?()
            Task { await scheduler.replaceAll([]) }
        }
        _enrollment = StateObject(wrappedValue: enrollment)
        _sync = StateObject(wrappedValue: sync)
        _coordinator = StateObject(wrappedValue: coordinator)
        _router = StateObject(wrappedValue: router)
        coordinator.isEnabled = !Self.isDemo
        if !Self.isDemo {
            ReminderBackgroundRefresh.register(sync: sync, coordinator: coordinator)
            ReminderBackgroundRefresh.schedule()
            Task { await coordinator.refreshPlanIfDue(force: true) }
            CoreMemoryStore.refreshModelDefaultsIfDue()
        }
    }

    var body: some Scene {
        WindowGroup {
            #if DEBUG
            if Self.isGallery {
                WGallery().onAppear { WGallery.installThemes() }
            } else {
                EnrollmentView(model: enrollment, sync: sync, coordinator: coordinator, router: router)
                    .environmentObject(coordinator)
            }
            #else
            EnrollmentView(model: enrollment, sync: sync, coordinator: coordinator, router: router)
                .environmentObject(coordinator)
            #endif
        }
        .onChange(of: scenePhase) { _, phase in
            coordinator.isForeground = phase == .active
            if phase == .background { ReminderBackgroundRefresh.schedule() }
        }
    }

    private static func removeStoreDirectory() throws {
        try FileManager.default.removeItem(at: try CoreMemoryStore.defaultPath().deletingLastPathComponent())
    }

    /// The demos have no real account: no settings pull, so nothing reaches the relay and no
    /// key or routing is written to this phone's real settings.
    static func settingsPuller(isDemo: Bool) -> SettingsPulling? {
        isDemo ? nil : CoreSettingsPuller()
    }

    private static var isDemo: Bool {
        #if DEBUG
        let arguments = ProcessInfo.processInfo.arguments
        return arguments.contains("-WhimbrelDemoEnrollment") || arguments.contains("-WhimbrelDemoNeedsDesktop") || arguments.contains("-WhimbrelDemoDeviceLimit") || arguments.contains("-WhimbrelDemoSynced") || isGallery
        #else
        return false
        #endif
    }

    #if DEBUG
    /// The component gallery (`-WhimbrelGallery`): a demo launch, so it reads no account and no store.
    private static var isGallery: Bool { ProcessInfo.processInfo.arguments.contains("-WhimbrelGallery") }
    #endif

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
        if arguments.contains("-WhimbrelDemoEnrollment") || arguments.contains("-WhimbrelDemoNeedsDesktop") || arguments.contains("-WhimbrelDemoDeviceLimit") {
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
