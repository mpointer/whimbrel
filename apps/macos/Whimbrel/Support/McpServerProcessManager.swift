import AppKit
import Foundation
import OSLog

/// Spawns and stops the real `whimbrel-mcp-server` binary (Task 4, `core/mcp-server/src/
/// bin/whimbrel-mcp-server.rs`, bundled into `Contents/Resources` by `project.yml` — see
/// that file's comment next to the `whimbrel-mcp-server` resource entry) as a long-lived
/// child process, driven by the "Enable MCP Server" Settings toggle.
///
/// `enabled` is persisted the same way `MeetingTriggerService.heuristicDetectionEnabled`
/// is: a `didSet`-backed `UserDefaults` write with no side effect of its own. The actual
/// start/stop side effect is wired up separately, in `AppState.init`, via
/// `mcpServerProcess.$enabled.sink` — mirroring exactly how `AppState.init` already wires
/// `meetingTrigger.$heuristicDetectionEnabled` to `meetingHeuristicDetector.start()`/
/// `stop()`. `@Published`'s projected publisher emits the current value immediately on
/// subscribe, so that sink also covers app launch: if the toggle was left on from a
/// previous session, the server resumes automatically, with no extra code needed for
/// that case (same reasoning already documented on the `heuristicDetectionEnabled` sink).
///
/// This is genuinely new precedent for this codebase: the only existing `Process()` uses
/// (`McpNodeLocator.envNodeResolves()`'s `env node --version` probe, `ModelDownloadService`'s
/// `tar` extraction) are one-shot, run-to-completion calls that never outlive the function
/// that spawned them. This one is a real persistent child process, so it needs real
/// termination handling on two distinct paths: an explicit toggle-off (`stop()`, called
/// directly) and a full app quit. For the latter, `Process()` children are NOT killed
/// automatically when their parent exits — left alone they'd be reparented to launchd as
/// orphans — so this observes `NSApplication.willTerminateNotification` (the same
/// notification-based lifecycle hook `PermissionsManager` already uses for
/// `didBecomeActiveNotification`, via its `activationObserver`) and calls `stop()` there.
/// Deliberately passes `queue: nil` (not `.main`, unlike `PermissionsManager`'s observer):
/// per `NotificationCenter.addObserver(forName:object:queue:using:)`'s own documentation,
/// `queue: nil` runs the block *synchronously on the posting thread*, whereas a non-nil
/// queue only *enqueues* it -- asynchronously, with no guarantee it runs before `terminate()`
/// finishes tearing the app down and calls `exit()`. `PermissionsManager`'s `.main` queue is
/// fine for its own use (a display refresh has no ordering requirement against app exit),
/// but that precedent does NOT transfer here: this call has to actually complete, on the
/// main thread, before the app process goes away, to avoid orphaning the child. `terminate()`
/// itself only needs to be *called* before exit (SIGTERM delivery to the child doesn't
/// require this parent process to still be alive), so no `waitUntilExit()` is needed.
@MainActor
final class McpServerProcessManager: ObservableObject {
    @Published private(set) var isRunning = false
    /// Set on a failed `start()` (binary missing, `Process.run()` throwing) or an
    /// unexpected exit (the child died on its own, not via our own `stop()`) — surfaced
    /// in Settings the same way `MCPClientsSettingsPane.errorMessage` already is.
    @Published private(set) var lastError: String?
    @Published var enabled: Bool {
        didSet { UserDefaults.standard.set(enabled, forKey: Self.enabledDefaultsKey) }
    }

    private static let enabledDefaultsKey = "com.mpointer.kestrel.mcpServerEnabled"

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "mcp-server-process")
    private var process: Process?
    private var terminationObserver: NSObjectProtocol?

    init() {
        enabled = UserDefaults.standard.bool(forKey: Self.enabledDefaultsKey)
        terminationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.stop()
            }
        }
    }

    deinit {
        if let terminationObserver {
            NotificationCenter.default.removeObserver(terminationObserver)
        }
    }

    func start() {
        guard process == nil else { return }
        guard let binaryURL = Bundle.main.url(forResource: "whimbrel-mcp-server", withExtension: nil) else {
            lastError = "MCP server binary not found in the app bundle."
            log.error("whimbrel-mcp-server binary not found in Bundle.main resources")
            return
        }
        reapStaleInstance()
        let task = Process()
        task.executableURL = binaryURL
        task.arguments = ["--db", SpikePaths.database.path]
        task.terminationHandler = { [weak self] finishedProcess in
            DispatchQueue.main.async {
                guard let self else { return }
                // If `self.process` is still set here, this fired because the child
                // exited on its own (crash, bind failure, etc.) rather than because our
                // own `stop()` already cleared it below -- surface that distinctly rather
                // than silently leaving the toggle showing "on" for a server that's
                // actually gone.
                if self.process != nil, finishedProcess.terminationStatus != 0 {
                    self.lastError = "MCP server exited unexpectedly (status \(finishedProcess.terminationStatus))."
                    self.log.error("whimbrel-mcp-server exited unexpectedly with status \(finishedProcess.terminationStatus)")
                }
                self.isRunning = false
                self.process = nil
            }
        }
        do {
            try task.run()
            process = task
            isRunning = true
            lastError = nil
        } catch {
            lastError = "Failed to start the MCP server: \(error.localizedDescription)"
            log.error("Failed to start whimbrel-mcp-server: \(error.localizedDescription, privacy: .public)")
        }
    }

    func stop() {
        guard let process else { return }
        process.terminate()
        self.process = nil
        isRunning = false
    }

    /// A prior run can leave an orphaned `whimbrel-mcp-server` behind if the app was
    /// killed non-gracefully -- Xcode's Stop button sends SIGKILL directly, which skips
    /// `willTerminateNotification` and the `stop()` call it triggers above -- and that
    /// orphan keeps holding `LISTEN_ADDR`, so every subsequent `start()` fails with
    /// `AddrInUse`. `self.process` is guaranteed nil by the time this runs (guarded in
    /// `start()`), so any process still named exactly "whimbrel-mcp-server" here is
    /// necessarily a stray, not ours -- safe to reap unconditionally before binding a
    /// fresh one. `pkill`/`pgrep -x` only ever runs synchronously and briefly (this is
    /// the rare stale-orphan path; the common case is `pkill` exiting 1 instantly with
    /// nothing to reap), matching the blocking `waitUntilExit()` style already used by
    /// `McpNodeLocator`'s one-shot `Process()` calls.
    private func reapStaleInstance() {
        let pkill = Process()
        pkill.executableURL = URL(fileURLWithPath: "/usr/bin/pkill")
        pkill.arguments = ["-x", "whimbrel-mcp-server"]
        pkill.standardOutput = FileHandle.nullDevice
        pkill.standardError = FileHandle.nullDevice
        guard (try? pkill.run()) != nil else { return }
        pkill.waitUntilExit()
        guard pkill.terminationStatus == 0 else { return }

        for _ in 0..<20 where isStaleInstanceStillRunning() {
            Thread.sleep(forTimeInterval: 0.05)
        }
    }

    private func isStaleInstanceStillRunning() -> Bool {
        let pgrep = Process()
        pgrep.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        pgrep.arguments = ["-x", "whimbrel-mcp-server"]
        pgrep.standardOutput = FileHandle.nullDevice
        pgrep.standardError = FileHandle.nullDevice
        guard (try? pgrep.run()) != nil else { return false }
        pgrep.waitUntilExit()
        return pgrep.terminationStatus == 0
    }
}
