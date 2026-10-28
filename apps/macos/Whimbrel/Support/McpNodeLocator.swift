import Foundation

/// Locates the two runtime dependencies the Google Calendar MCP connector
/// needs to actually run: a system `node` binary (not bundled — a real,
/// separate dependency; see docs/superpowers/specs/
/// 2026-07-31-google-calendar-connector-completion-design.md), and the
/// connector's own bundled script (dist/index.js + node_modules, copied
/// into the app's Resources by project.yml's folder-reference sources —
/// see Task 2's own verification step for how that path was confirmed).
enum McpNodeLocator {
    /// Common Homebrew/system install locations, checked in order. If
    /// neither exists, probes whether `/usr/bin/env node` actually resolves
    /// to something runnable (covers nvm/fnm and other non-Homebrew installs
    /// that only show up via the shell's real PATH, which a GUI app launched
    /// from Finder/LaunchServices does NOT inherit by default). Returns nil
    /// only if node genuinely can't be found anywhere — the caller treats
    /// that as "Node.js isn't installed," a real, user-facing, distinctly-
    /// worded error state (see CalendarConnectorState). Deliberately does
    /// NOT unconditionally fall back to `/usr/bin/env`: that binary always
    /// exists on macOS regardless of whether `node` does, so an unconditional
    /// fallback would make `nodeNotFound` unreachable.
    static func nodeExecutablePath() -> String? {
        let directCandidates = ["/opt/homebrew/bin/node", "/usr/local/bin/node"]
        for path in directCandidates {
            if FileManager.default.isExecutableFile(atPath: path) {
                return path
            }
        }
        return envNodeResolves() ? "/usr/bin/env" : nil
    }

    /// Actually runs `env node --version` (rather than just checking that
    /// `/usr/bin/env` exists, which is true on every macOS install
    /// regardless of whether `node` does) to confirm `env` can genuinely
    /// resolve a `node` binary via the process's inherited PATH.
    private static func envNodeResolves() -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["node", "--version"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus == 0
        } catch {
            return false
        }
    }

    /// True if `resolvedPath` (the value `nodeExecutablePath()` already
    /// computed) is `/usr/bin/env` — callers need to pass `"node"` as the
    /// first argument in that case. Takes the already-resolved path rather
    /// than calling `nodeExecutablePath()` again itself, so the `env node`
    /// probe it performs on the fallback path only ever runs once per
    /// connection attempt.
    static func nodeNeedsEnvWrapper(resolvedPath: String) -> Bool {
        resolvedPath == "/usr/bin/env"
    }

    /// Path to the bundled connector's entry point inside the app's own
    /// Resources. Confirmed empirically via Task 2 Step 4: a Debug build's
    /// `find .../Whimbrel.app/Contents/Resources -maxdepth 2 -iname dist -o
    /// -iname node_modules` showed the two folder-reference resources land
    /// directly under `Contents/Resources/dist` and
    /// `Contents/Resources/node_modules` (just the source folder's own
    /// name, not its full relative path) — matching what was assumed here.
    /// Returns nil if the resource genuinely isn't there (e.g. a fresh
    /// checkout that never ran `npm ci`) so `connectorScriptNotFound` can
    /// actually fire instead of failing later as a generic spawn error.
    static func calendarConnectorScriptPath() -> String? {
        guard let path = Bundle.main.resourceURL?.appendingPathComponent("dist/index.js").path,
              FileManager.default.fileExists(atPath: path) else {
            return nil
        }
        return path
    }

    /// Path to the bundled Gmail MCP connector's entry point. A second
    /// connector's own `dist/index.js` can't share the Calendar connector's
    /// `dist` name at `Contents/Resources/dist` (XcodeGen folder-reference
    /// resources land at `Contents/Resources/<source folder's own
    /// basename>`, so two same-named `dist`/`node_modules` folders from
    /// different connectors would collide there — one would silently
    /// overwrite the other). Task 8's `project.yml` change routes Gmail's
    /// `dist`/`node_modules` folder-reference resources through XcodeGen's
    /// documented `buildPhase: copyFiles` + `subpath` mechanism (confirmed
    /// against XcodeGen's own ProjectSpec.md) instead of the default Copy
    /// Bundle Resources phase Calendar's two entries use.
    ///
    /// That mechanism's `subpath` renames the outer destination directory
    /// but — confirmed empirically via a real `xcodegen generate` +
    /// `xcodebuild` build, then `find`/`shasum`/`grep` against the actual
    /// built `.app`'s `Contents/Resources` — a Copy Files build phase still
    /// nests the copied folder reference one level deeper under its own
    /// source basename, NOT flat under the `subpath` a naive reading of that
    /// option might suggest.
    ///
    /// Both of Gmail's entries therefore share ONE `subpath` (`gmail`), so
    /// their differing source basenames land them as siblings:
    /// `gmail/dist/index.js` beside `gmail/node_modules/`. That adjacency is
    /// load-bearing, not cosmetic — Node resolves `require`/`import` by
    /// walking up from the script's own directory, so this is what makes the
    /// connector find ITS OWN bundled `zod`/`@modelcontextprotocol/sdk`.
    /// Under the earlier split subpaths (`gmail-dist`/`gmail-node_modules`)
    /// that walk found no `node_modules` until it reached
    /// `Contents/Resources/node_modules`, which is the CALENDAR connector's
    /// copy — working purely by coincidence, since both connectors pin the
    /// same dependency versions today, and silently breaking the moment they
    /// diverge or Calendar is removed. (Verification also confirmed
    /// Calendar's own `dist/index.js` is untouched and distinct:
    /// `create_event` appears only in Calendar's copy, `send_email` only in
    /// Gmail's, and the two files have different SHA1 hashes.)
    /// Returns nil if the resource genuinely isn't there.
    static func gmailConnectorScriptPath() -> String? {
        guard let path = Bundle.main.resourceURL?.appendingPathComponent("gmail/dist/index.js").path,
              FileManager.default.fileExists(atPath: path) else {
            return nil
        }
        return path
    }

    /// Path to the bundled Outlook 365 MCP connector's entry point. Same
    /// subpath-grouping mechanism as `gmailConnectorScriptPath()` -- see that
    /// method's doc comment. Returns nil if the resource genuinely isn't there.
    static func outlook365ConnectorScriptPath() -> String? {
        guard let path = Bundle.main.resourceURL?.appendingPathComponent("outlook365/dist/index.js").path,
              FileManager.default.fileExists(atPath: path) else {
            return nil
        }
        return path
    }
}
