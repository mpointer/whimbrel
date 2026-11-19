import Combine
import Foundation

/// Where a launch looks and what it may touch: the data root (the database is
/// `<root>/core-memory.sqlite`), the Keychain, the per-device settings and the models folder.
/// The app passes the live ones (`AppLaunch.live`, the only place that names them); tests pass
/// a temp root, a throwaway Keychain service and a `UserDefaults` suite.
struct LaunchEnvironment {
    var dataRoot: URL
    var modelsDir: URL?
    var keys: LaunchKeyStore
    var defaults: UserDefaults

    var dbPath: URL { Self.dbPath(in: dataRoot) }

    /// The main database under a data root. Delete everything (`YourData`) removes this path's
    /// files, so it must stay the one the launch opens.
    static func dbPath(in dataRoot: URL) -> URL { dataRoot.appendingPathComponent("core-memory.sqlite") }
}

/// The main store a launch opened, as `AppState` takes it.
struct LaunchedStore {
    let store: CoreMemoryStore?
    /// The key the store opened with; nil for a plain store.
    let key: Data?
    /// The plan started the MCP server (always the last launch step). `AppState` starts the
    /// process only when this is true and the user's toggle is on.
    let mcpAllowed: Bool
}

/// Runs one launch (X4 Task 11b-2): the coordinator over the real C ABI, with the presenter
/// the launch window draws. It never opens the store itself, never starts the MCP server
/// itself (the coordinator's `startMcpServer` step hands the key to `startMcp`), and calls
/// nothing that is not in the environment or the injected closures.
@MainActor
final class AppLaunch: ObservableObject {
    enum Result {
        case opened(LaunchOpened<CoreMemoryStore>, mcpAllowed: Bool)
        /// The plain open failed and the user went on without the store (no MCP server).
        case openedWithoutStore
        case quit
    }

    /// The app's launch, set by `WhimbrelMain` before the scenes exist.
    static var shared: AppLaunch?

    /// Set once the launch finished; `resultPublisher` announces it after it is stored.
    private(set) var result: Result? {
        didSet { resultSubject.send(result) }
    }
    /// The launch result, sent after `result` holds it, so a subscriber can read `launchedStore`.
    /// A `@Published` sink runs before the new value is stored: `AppHost` read nil there and
    /// dropped any open that finished after it subscribed (an open over the 10 s wait, or any
    /// launch that showed a screen first), which stayed on "Starting Whimbrel…" (0.9.80 device check).
    var resultPublisher: AnyPublisher<Result?, Never> { resultSubject.eraseToAnyPublisher() }
    private let resultSubject = CurrentValueSubject<Result?, Never>(nil)
    let presenter: LaunchUIPresenter
    let coordinator: LaunchCoordinator<CoreMemoryStore>
    let environment: LaunchEnvironment
    let settings: UserDefaultsLaunchSettings
    /// A meeting is being recorded now. False until `AppState` exists (it sets this to its
    /// recorder's state), so nothing records during a launch.
    var recording: () -> Bool = { false }
    private var mcpAllowed = false
    private var started = false

    /// `startMcp`: the key the main store opened with (nil for plain), when the plan reaches
    /// `StartMcpServer`. `physicalMemory`: overridable for tests.
    init(environment: LaunchEnvironment, presenter: LaunchUIPresenter,
         physicalMemory: @escaping () -> UInt64 = { ProcessInfo.processInfo.physicalMemory },
         startMcp: @escaping (Data?) -> Void) {
        self.environment = environment
        self.presenter = presenter
        let defaults = environment.defaults
        presenter.consumeFreshLibraryMarker = { FreshLibraryMarker.consume(defaults) }
        settings = UserDefaultsLaunchSettings(defaults: environment.defaults)
        var box: (() -> Bool)?
        var mcpBox: ((Data?) -> Void)?
        var c = LaunchCoordinator<CoreMemoryStore>.Collaborators.core(
            dbPath: environment.dbPath, modelsDir: environment.modelsDir, keys: environment.keys, settings: settings,
            presenter: presenter, recording: { box?() ?? false }, startMcpServer: { mcpBox?($0) })
        c.physicalMemory = physicalMemory
        coordinator = LaunchCoordinator(c)
        box = { [weak self] in self?.recording() ?? false }
        mcpBox = { [weak self] key in
            self?.mcpAllowed = true
            startMcp(key)
        }
    }

    /// Starts the launch once.
    func start() {
        guard !started else { return }
        started = true
        Task { @MainActor in
            let outcome = await coordinator.run()
            switch outcome {
            case let .opened(o):
                presenter.launchFinished(opened: true)
                result = .opened(o, mcpAllowed: mcpAllowed)
            case .openedWithoutStore:
                presenter.launchFinished(opened: false)
                result = .openedWithoutStore
            case .quit:
                presenter.launchFinished(opened: false)
                result = .quit
            }
        }
    }

    /// Starts the launch and runs the main run loop until it finished or needs the user, at
    /// most `timeout`. Called before the app's scenes exist, so a plain or keyed open that
    /// needs no screen is done before the first window draws (the app opens as before).
    func startAndWait(timeout: TimeInterval) {
        start()
        let deadline = Date(timeIntervalSinceNow: timeout)
        while result == nil, !presenter.needsWindow, Date() < deadline {
            _ = RunLoop.main.run(mode: .default, before: Date(timeIntervalSinceNow: 0.01))
        }
    }

    /// The first main window has rendered: the good-launch bookkeeping may run (once).
    func firstWindowRendered() async {
        await coordinator.firstWindowRendered()
    }

    /// `AppState`'s store, once opened.
    var launchedStore: LaunchedStore? {
        switch result {
        case let .opened(o, mcp)?: return LaunchedStore(store: o.store, key: o.key, mcpAllowed: mcp)
        // AppState runs with no store (its pre-X4 "database didn't open" handling), and the
        // MCP server never starts on a database the launch did not open.
        case .openedWithoutStore?: return LaunchedStore(store: nil, key: nil, mcpAllowed: false)
        default: return nil
        }
    }

    var openMs: UInt64? {
        guard case let .opened(o, _)? = result else { return nil }
        return o.openMs
    }
}
