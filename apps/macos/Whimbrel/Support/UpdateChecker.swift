import Foundation
import OSLog

/// FR: auto-triggered update downloads (docs/superpowers/specs/2026-08-06-auto-update-
/// downloads-design.md). Checks a version manifest on launch and every 24h, downloads a
/// newer version's zip silently in the background, and leaves it staged for the UI layer
/// (Task 4) to offer a "Restart Now" prompt. Never installs anything itself.
@MainActor
final class UpdateChecker: ObservableObject {
    /// `nonisolated` (rather than the default MainActor isolation) so the unzip/verify work
    /// in `download(version:from:)` can log from inside a detached background task — see
    /// that function's comment. `Logger` is an immutable, `Sendable` value, so this is safe.
    nonisolated private let log = Logger(subsystem: "com.mpointer.kestrel", category: "update-checker")

    struct DownloadedUpdate {
        let version: String
        /// Path to the downloaded, unzipped .app on disk, ready for the relaunch helper.
        let appPath: URL
    }

    /// `Decodable` mirror of core/api::update's `Manifest` (M3.0 Task 4): decoding,
    /// the version compare and the SHA-256 check live in Rust behind
    /// `CoreMemoryStore.checkUpdateManifest` / `verifyUpdateSha256`.
    struct VersionManifest: Decodable {
        let version: String
        let url: String
        let sha256: String?
        let commit: String?
        let tag: String?
        let publishedAt: String?
        let releaseNotesURL: String?

        enum CodingKeys: String, CodingKey {
            case version, url, sha256, commit, tag
            case publishedAt = "published_at"
            case releaseNotesURL = "release_notes_url"
        }
    }

    @Published private(set) var downloadedUpdate: DownloadedUpdate?

    /// The staged copy is gone from disk (the temp-dir cleaner reaped it): forget it, so
    /// the next check downloads that version again instead of skipping it as staged.
    func discardMissingStagedUpdate() {
        guard let update = downloadedUpdate,
              !FileManager.default.fileExists(atPath: update.appPath.path) else { return }
        downloadedUpdate = nil
        status = .idle
    }

    private var periodicTask: Task<Void, Never>?
    private let manifestURL: URL
    private let currentVersion: String

    static let checkIntervalSeconds: UInt64 = 24 * 60 * 60

    init(
        manifestURL: URL = URL(string: "https://updates.whimbrelapp.com/version.json")!,
        currentVersion: String = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.0.0",
        fetchManifest: ManifestFetch? = nil,
        downloader: Downloader? = nil
    ) {
        self.fetchManifest = fetchManifest
        self.downloader = downloader
        self.manifestURL = manifestURL
        self.currentVersion = currentVersion
    }

    func start() {
        guard periodicTask == nil else { return }
        periodicTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.checkNow()
                try? await Task.sleep(nanoseconds: Self.checkIntervalSeconds * 1_000_000_000)
            }
        }
    }

    /// Where the manual "Check for Updates" UI reads its state; background checks drive it too.
    enum UpdateStatus: Equatable {
        case idle, checking, upToDate(version: String), downloading(version: String), ready(version: String), failed
    }

    @Published private(set) var status: UpdateStatus = .idle

    /// Test seams: nil means the real network fetch / download-unzip-verify.
    typealias ManifestFetch = (URL) async -> Data?
    typealias Downloader = (_ version: String, _ url: URL, _ sha256: String?) async -> DownloadedUpdate?
    private let fetchManifest: ManifestFetch?
    private let downloader: Downloader?

    /// The check in progress, so a manual check during a background one (or two manual
    /// ones) joins it instead of starting a second manifest fetch or download.
    private var inFlight: Task<Void, Never>?

    func checkNow() async {
        if let running = inFlight {
            await running.value
            return
        }
        let task = Task { await self.performCheck() }
        inFlight = task
        await task.value
        inFlight = nil
    }

    private func performCheck() async {
        status = .checking
        let data: Data?
        if let fetchManifest {
            data = await fetchManifest(manifestURL)
        } else if let (d, response) = try? await URLSession.shared.data(from: manifestURL),
                  let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 {
            data = d
        } else {
            data = nil
        }
        guard let data,
              let check = CoreMemoryStore.checkUpdateManifest(json: data, currentVersion: currentVersion) else {
            log.log("checkNow: manifest fetch/decode failed, skipping this check")
            status = .failed
            return
        }
        let manifest = check.manifest

        guard check.newer else {
            log.log("checkNow: remote \(manifest.version, privacy: .public) is not newer than local \(self.currentVersion, privacy: .public)")
            status = .upToDate(version: currentVersion)
            return
        }
        guard Self.shouldDownload(remoteVersion: manifest.version, alreadyStagedVersion: downloadedUpdate?.version) else {
            log.log("checkNow: \(manifest.version, privacy: .public) already staged, skipping re-download")
            status = .ready(version: manifest.version)
            return
        }
        guard let downloadURL = URL(string: manifest.url) else {
            log.log("checkNow: manifest url is not a valid URL: \(manifest.url, privacy: .public)")
            status = .failed
            return
        }

        status = .downloading(version: manifest.version)
        let staged: DownloadedUpdate?
        if let downloader {
            staged = await downloader(manifest.version, downloadURL, manifest.sha256)
        } else {
            staged = await download(version: manifest.version, from: downloadURL, expectedSha256: manifest.sha256)
        }
        if let staged {
            downloadedUpdate = staged
            status = .ready(version: staged.version)
        } else {
            status = .failed
        }
    }

    private func download(version: String, from url: URL, expectedSha256: String?) async -> DownloadedUpdate? {
        guard let (tempZipURL, response) = try? await URLSession.shared.download(from: url),
              let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            log.log("download: failed to fetch \(url.absoluteString, privacy: .public)")
            return nil
        }

        // ditto (unzipping a ~260MB bundle) and the spctl/codesign verification below are
        // slow enough (several seconds) to freeze the UI if run inline on this @MainActor
        // method — which happens whenever a background 24h check finds a real update.
        // Doing the unzip + verification on a detached background task keeps the UI
        // responsive; only the final `downloadedUpdate` assignment needs the main actor.
        let appPath: URL? = await Task.detached(priority: .utility) { [log] in
            // A nil expectedSha256 is a legacy no-hash manifest and passes (core/api::update).
            guard CoreMemoryStore.verifyUpdateSha256(path: tempZipURL, expectedSha256: expectedSha256) else {
                log.error("download: sha256 mismatch for \(version, privacy: .public), refusing to stage")
                try? FileManager.default.removeItem(at: tempZipURL)
                return nil
            }

            let destinationDir = FileManager.default.temporaryDirectory.appendingPathComponent("whimbrel-update-\(version)", isDirectory: true)
            try? FileManager.default.removeItem(at: destinationDir)
            try? FileManager.default.createDirectory(at: destinationDir, withIntermediateDirectories: true)

            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
            process.arguments = ["-x", "-k", tempZipURL.path, destinationDir.path]
            do {
                try process.run()
                process.waitUntilExit()
                guard process.terminationStatus == 0 else {
                    log.log("download: ditto unzip failed with status \(process.terminationStatus, privacy: .public)")
                    return nil
                }
            } catch {
                log.log("download: failed to launch ditto: \(error.localizedDescription, privacy: .public)")
                return nil
            }

            guard let appPath = UpdateChecker.stagedAppBundle(in: destinationDir) else {
                log.log("download: expected exactly one .app after unzip in \(destinationDir.path, privacy: .public)")
                return nil
            }

            guard UpdateChecker.verifySignedByExpectedTeam(at: appPath) else {
                log.error("download: \(appPath.path, privacy: .public) failed signature/notarization verification, refusing to stage")
                try? FileManager.default.removeItem(at: destinationDir)
                return nil
            }

            return appPath
        }.value

        guard let appPath else { return nil }

        log.log("download: staged \(version, privacy: .public) at \(appPath.path, privacy: .public)")
        return DownloadedUpdate(version: version, appPath: appPath)
    }

    /// The updates bucket (server-updates/, published by release.sh) is a plain R2
    /// bucket — a leaked wrangler token or a misconfigured ACL is enough to overwrite
    /// its contents. Without this check, a bucket write compromise would be sufficient
    /// to get arbitrary code executed once the relaunch helper (Task 4) installs
    /// whatever download() staged. `spctl` reproduces release.sh's own final acceptance
    /// gate (line ~151: `spctl -a -vv --type execute`) — valid Developer ID signature
    /// plus a real notarization ticket. The TeamIdentifier check on top of that pins
    /// acceptance to our own account (release.sh's hardcoded TEAM_ID) rather than any
    /// Apple-notarized binary, so another valid-but-unrelated Developer ID can't pass.
    nonisolated static let expectedTeamID = "GP8PFLM3Q3"

    nonisolated static func verifySignedByExpectedTeam(at appPath: URL) -> Bool {
        let spctl = Process()
        spctl.executableURL = URL(fileURLWithPath: "/usr/sbin/spctl")
        spctl.arguments = ["-a", "--type", "execute", appPath.path]
        guard (try? spctl.run()) != nil else { return false }
        spctl.waitUntilExit()
        guard spctl.terminationStatus == 0 else { return false }

        let codesign = Process()
        codesign.executableURL = URL(fileURLWithPath: "/usr/bin/codesign")
        codesign.arguments = [
            "--verify", "-R",
            "=anchor apple generic and certificate leaf[subject.OU] = \"\(expectedTeamID)\"",
            appPath.path
        ]
        guard (try? codesign.run()) != nil else { return false }
        codesign.waitUntilExit()
        return codesign.terminationStatus == 0
    }

    // MARK: - Pure logic (directly tested)

    /// The one `.app` the update zip unpacked. The bundle's name is not pinned: the
    /// rename release ships `Whimbrel.app` where every earlier zip held `Whimbrel.app`
    /// (docs/superpowers/specs/2026-10-21-kestrel-to-whimbrel-rename-design.md, step 4a),
    /// and `RelaunchHelper.sh` installs whatever name is staged. Exactly one bundle is
    /// required; none or several is refused rather than guessed at.
    nonisolated static func stagedAppBundle(in directory: URL) -> URL? {
        guard let entries = try? FileManager.default.contentsOfDirectory(
            at: directory, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles]
        ) else { return nil }
        let bundles = entries.filter { entry in
            entry.pathExtension == "app"
                && ((try? entry.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false)
        }
        return bundles.count == 1 ? bundles[0] : nil
    }

    /// `alreadyStagedVersion` is whatever `downloadedUpdate?.version` currently holds —
    /// re-downloading the same version on every 24h tick would be wasteful; a genuinely
    /// newer version (staged version bumped again before the user restarted) must still
    /// download.
    nonisolated static func shouldDownload(remoteVersion: String, alreadyStagedVersion: String?) -> Bool {
        remoteVersion != alreadyStagedVersion
    }
}
