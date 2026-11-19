import AppKit
import Foundation

/// The live launch: the ONLY place the launch names the real data folder, the real
/// `UserDefaults` and the real Keychain service (a source-scan test holds the other launch
/// files to that). Never called by the test host (`WhimbrelMain` runs `TestHostApp` there).
extension AppLaunch {
    static func live() -> AppLaunch {
        let environment = LaunchEnvironment(
            dataRoot: SpikePaths.root,
            modelsDir: EmbeddingModelLocator.modelsDirectoryPath().map(URL.init(fileURLWithPath:)),
            keys: DbKeyStore(),
            defaults: .standard)
        let presenter = LaunchUIPresenter(
            setupActions: { EncryptionSetupModel.Actions.live(existingKey: $0, databaseFolder: SpikePaths.root) },
            recoveryActions: { EncryptionRecoveryModel.Actions.live(useKey: $0) })
        return AppLaunch(environment: environment, presenter: presenter) { key in
            // Set before anything reads it: the per-call opens (skin calls) and the MCP
            // server's stdin use the key the main store opened with (nil for plain).
            CoreMemoryStore.databaseKey = key
        }
    }
}

/// Restarts Whimbrel only after this process (and the MCP server child, if one runs) has
/// exited, so the next launch's migration finds the memory database closed. A shell waits
/// for the process ids, then opens the app again; the ids and the app path are arguments,
/// never part of the script text.
enum AppRestart {
    /// Waits up to `$7` tenths of a second for each process id; if one is still alive then,
    /// it never opens a second instance on the same database: the alert (`$6`) tells the
    /// user to quit and open Whimbrel again. Otherwise the opener (`$5`) opens the app.
    static let script = """
    for pid in "$1" "$2"; do
      [ -n "$pid" ] || continue
      n=0; while kill -0 "$pid" 2>/dev/null && [ $n -lt "$7" ]; do sleep 0.1; n=$((n+1)); done
      if kill -0 "$pid" 2>/dev/null; then "$6" "$4"; exit 0; fi
    done
    "$5" -n "$3"
    """

    /// `/bin/sh` arguments: the ids, the app, the message and the commands are arguments,
    /// never part of the script text. `alerter` is called with the message.
    static func arguments(pid: Int32, otherPid: Int32?, appPath: String, message: String, opener: String, alerter: String,
                          waitTenths: Int) -> [String] {
        ["-c", script, "sh", String(pid), otherPid.map(String.init) ?? "", appPath, message, opener, alerter, String(waitTenths)]
    }

    /// Shows `$1` in a plain alert (osascript, so it works after this process is gone).
    static let alertScript = "on run argv\ndisplay alert (item 1 of argv)\nend run"

    /// Starts the waiting shell, then quits this process. If the helper cannot be written or
    /// the shell cannot start, nothing is quit or opened and the user is told to quit and
    /// open Whimbrel again (`restartFailed`).
    /// What the user is told when the restart cannot start (`failed`) or the old process does
    /// not exit in time (`stuck`).
    struct Messages {
        let failed: String
        let stuck: String
        static let encryption = Messages(failed: EncryptionCopy.restartFailed, stuck: EncryptionCopy.restartStuck)
    }

    static func relaunchAfterExit(waitingAlsoFor otherPid: Int32?,
                                  helperDir: URL = FileManager.default.temporaryDirectory,
                                  messages: Messages = .encryption,
                                  run: (Process) throws -> Void = { try $0.run() },
                                  showMessage: (String) -> Void = showAlert,
                                  terminate: () -> Void = { NSApp.terminate(nil) }) {
        let alerter = helperDir.appendingPathComponent("whimbrel-restart-alert.sh")
        let alert = "#!/bin/sh\nexec /usr/bin/osascript -e '\(alertScript.replacingOccurrences(of: "\n", with: "' -e '"))' \"$1\"\n"
        guard (try? alert.write(to: alerter, atomically: true, encoding: .utf8)) != nil,
              (try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: alerter.path)) != nil else {
            showMessage(messages.failed)
            return
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = arguments(pid: ProcessInfo.processInfo.processIdentifier, otherPid: otherPid,
                                      appPath: Bundle.main.bundleURL.path, message: messages.stuck,
                                      opener: "/usr/bin/open", alerter: alerter.path, waitTenths: 300)
        do {
            try run(process)
            terminate()
        } catch {
            // Not restarted: the user quits and opens Whimbrel; encryption still runs then.
            showMessage(messages.failed)
        }
    }

    static func showAlert(_ message: String) {
        let alert = NSAlert()
        alert.messageText = message
        alert.runModal()
    }
}

/// Cmd-Q, the menu bar's Quit and any other app termination go through here (X4 Task 11b-2):
/// during the recovery setup, once a fresh key's file or words exist, the app does not quit;
/// the setup's "Quit anyway?" confirmation shows instead (its Quit ends the launch, which
/// then terminates the app normally).
final class AppTerminationGuard: NSObject, NSApplicationDelegate {
    @MainActor
    static func reply(_ presenter: LaunchUIPresenter?) -> NSApplication.TerminateReply {
        guard let presenter, presenter.quitNeedsConfirmation else { return .terminateNow }
        presenter.requestQuit()
        NSApp?.activate(ignoringOtherApps: true)
        return .terminateCancel
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        MainActor.assumeIsolated {
            let reply = Self.reply(AppLaunch.shared?.presenter)
            guard reply == .terminateNow else { return reply }
            // A recording start whose meeting row is still being written: give it a bounded time to
            // land, so quitting never throws the row away. Without blocking main: the continue
            // path needs main to finish, and AppKit keeps the main queue running while it waits.
            return PendingRecordingStarts.shared.quitReply(timeout: PendingRecordingStarts.quitWaitSeconds) {
                NSApp.reply(toApplicationShouldTerminate: true)
            }
        }
    }
}
