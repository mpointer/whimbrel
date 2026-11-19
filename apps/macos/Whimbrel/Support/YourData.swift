import Foundation
import AppKit
import os
import Security

/// The Your data pane's facts and actions (trust surface spec,
/// docs/superpowers/specs/2026-11-04-trust-surface-design.md). The pure parts live here so
/// the test target can check them without an `AppState`.
enum YourData {
    /// One row of "What leaves this computer": a channel, when it sends, what the other
    /// side can see, and this Mac's current state for it.
    struct Outbound: Equatable, Identifiable {
        enum Channel: String, CaseIterable { case providers, syncRelay, updateCheck, google, typesafe }
        let channel: Channel
        var id: Channel { channel }
        let title: String
        let when: String
        let sees: String
        let state: String
        /// True when something is being sent on this channel today.
        let active: Bool
    }

    /// The inputs the rows are built from; the pane fills them from `AppState`.
    struct Inputs: Equatable {
        var hostedProviders: [String]
        var localOnly: Bool
        var syncOn: Bool
        var syncDevices: Int
        var googleAccounts: [String]
        var typesafeOn: Bool
        /// Whimbrel AI's data sentence when it is a hosted provider here, else nil.
        var whimbrelAITerms: String? = nil
    }

    /// "Keys for A, B." for providers that use a key; Whimbrel AI has none (prepaid credit),
    /// so it gets its own sentence, alone or after the keys.
    static func providerState(_ hosted: [String]) -> String {
        let keyed = hosted.filter { $0 != "Whimbrel AI" }
        var parts: [String] = []
        if !keyed.isEmpty { parts.append(String(localized: "Keys for \(keyed.joined(separator: ", ")).")) }
        if keyed.count != hosted.count { parts.append(String(localized: "Whimbrel AI (prepaid credit).")) }
        return parts.joined(separator: " ")
    }

    static func outbound(_ i: Inputs) -> [Outbound] {
        let hostedOn = !i.localOnly && !i.hostedProviders.isEmpty
        let providers = Outbound(
            channel: .providers,
            title: String(localized: "AI providers"),
            when: String(localized: "When you ask a question, summarize a meeting, or run a routine with a hosted model."),
            sees: [String(localized: "The provider whose key you added receives the text needed for that request, under its own terms. Known secrets are redacted first."),
                   hostedOn ? String(localized: "The text of the window you ask about is sent to the model you chose, as in Ask.") : nil,
                   i.whimbrelAITerms].compactMap { $0 }.joined(separator: " "),
            state: i.localOnly
                ? String(localized: "Local-only mode: nothing is sent.")
                : i.hostedProviders.isEmpty
                    ? String(localized: "No hosted keys. Only local models run.")
                    : Self.providerState(i.hostedProviders),
            active: hostedOn
        )
        let relay = Outbound(
            channel: .syncRelay,
            title: String(localized: "Sync relay"),
            when: String(localized: "Continuously, while sync is on."),
            sees: String(localized: "The relay stores encrypted data it cannot read. It can see your account email and identifier, each device's name, platform, public key and enrollment dates, the type and time of each synced item, and the names of synced settings fields. Never content, never keys."),
            state: i.syncOn ? String(localized: "On, \(i.syncDevices) device(s).") : String(localized: "Off."),
            active: i.syncOn
        )
        let update = Outbound(
            channel: .updateCheck,
            title: String(localized: "Update check"),
            when: String(localized: "When the app starts, then once a day."),
            sees: String(localized: "A random identifier that changes every week, the platform and the app version, so we can count installs. It is not linked to you, your account or your devices."),
            state: String(localized: "Always on. It carries nothing about you."),
            active: true
        )
        let google = Outbound(
            channel: .google,
            title: String(localized: "Google"),
            when: String(localized: "When a Google account is connected."),
            sees: String(localized: "Google sees the app reading your Gmail and Calendar. The data is stored in this Mac's database. It is only sent on to us if you turn on a feature listed on this page."),
            state: i.googleAccounts.isEmpty ? String(localized: "No account connected.") : i.googleAccounts.joined(separator: ", "),
            active: !i.googleAccounts.isEmpty
        )
        let typesafe = Outbound(
            channel: .typesafe,
            title: String(localized: "TypeSafe reply ranking"),
            when: String(localized: "When it is on, as email arrives."),
            sees: String(localized: "TypeSafe receives email subjects and snippets to score which threads need a reply."),
            state: i.typesafeOn ? String(localized: "On.") : String(localized: "Off (the default)."),
            active: i.typesafeOn
        )
        return [providers, relay, update, google, typesafe]
    }

    /// The AI providers row's spend line (the AI pane's last-30-day ledger), largest first;
    /// nil until a hosted call has been metered.
    static func spendLine(_ totals: CoreUsageTotals?) -> String? {
        guard let totals, totals.last30DaysCents > 0 else { return nil }
        let dollars = { (cents: Int64) in String(format: "$%.2f", Double(cents) / 100) }
        let names = ["anthropic": "Anthropic", "openai": "OpenAI", "google": "Google", "openrouter": "OpenRouter"]
        let parts = totals.last30DaysByProvider.filter { $0.cents > 0 }.sorted { $0.cents > $1.cents }
            .map { "\(names[$0.provider] ?? $0.provider) \(dollars($0.cents))" }
        return String(localized: "Last 30 days: \(dollars(totals.last30DaysCents)) in hosted model calls (\(parts.joined(separator: ", "))).")
    }

    /// Bytes under `url`, following no symlinks; 0 for a missing folder.
    static func folderBytes(_ url: URL, fileManager fm: FileManager = .default) -> Int64 {
        guard let enumerator = fm.enumerator(at: url, includingPropertiesForKeys: [.totalFileAllocatedSizeKey, .isRegularFileKey], options: [.skipsHiddenFiles]) else { return 0 }
        var total: Int64 = 0
        for case let file as URL in enumerator {
            guard let values = try? file.resourceValues(forKeys: [.totalFileAllocatedSizeKey, .isRegularFileKey]),
                  values.isRegularFile == true else { continue }
            total += Int64(values.totalFileAllocatedSize ?? 0)
        }
        return total
    }

    static func formatBytes(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    /// Everything "Delete everything on this computer" removes: every entry directly under
    /// the data folder except the ones in `keep` (the downloaded models, which are not the
    /// user's data and cost hundreds of megabytes to fetch again, and the list of connected
    /// Google accounts the new library reconnects, `KeptGoogleAccounts`). Sorted, for a stable
    /// list in the confirmation sheet.
    static func entriesToDelete(in root: URL, keep: Set<String> = ["models", KeptGoogleAccounts.fileName],
                                fileManager fm: FileManager = .default) -> [URL] {
        let entries = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil, options: [])) ?? []
        return entries
            .filter { !keep.contains($0.lastPathComponent) }
            .sorted { $0.lastPathComponent.localizedCaseInsensitiveCompare($1.lastPathComponent) == .orderedAscending }
    }

    /// This Mac's recordings and slide images in the media folder (default `~/Pictures/Whimbrel`),
    /// which is outside the data folder: its own `Audio/<device>` folder and every
    /// `Snapshots/` meeting folder Whimbrel made (the ones holding `SnapshotStorage.markerName`).
    /// Another Mac's `Audio/<device>` in a shared media folder is never listed.
    // ponytail: snapshot folders carry no device id, so in a media folder shared by two Macs
    // the other Mac's slide folders are listed too; the sheet names the folder.
    static func mediaEntriesToDelete(audio: URL, snapshots: URL, fileManager fm: FileManager = .default) -> [URL] {
        // lstat only: a link meeting folder or a link marker is not Whimbrel's.
        let meetingFolders = ((try? fm.contentsOfDirectory(at: snapshots, includingPropertiesForKeys: nil, options: [])) ?? [])
            .filter { lstatType($0, fm) == .typeDirectory && lstatType($0.appendingPathComponent(SnapshotStorage.markerName), fm) == .typeRegular }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        return (lstatType(audio, fm) != nil ? [audio] : []) + meetingFolders
    }

    /// The file type of `url` itself (a link is `.typeSymbolicLink`, never followed); nil when absent.
    private static func lstatType(_ url: URL, _ fm: FileManager) -> FileAttributeType? {
        (try? fm.attributesOfItem(atPath: url.path))?[.type] as? FileAttributeType
    }

    /// What `deleteMediaNow` did.
    struct MediaDeleteResult: Equatable {
        /// Entries that are still there.
        var notRemoved: [URL] = []
        /// Nothing was deleted because the media folder is not safe to delete from: this folder.
        var refused: URL? = nil
    }

    /// Delete everything's recordings and slides, in the app at confirm time (never through
    /// the launch note, whose folder anything could write). `audio` is `<media>/Audio/<this
    /// device>`, `snapshots` is `<media>/Snapshots`. Refused (nothing deleted) when `<media>`,
    /// `Audio` or `Snapshots` is a link (lstat, never resolved), or when `<media>` is `/`, the
    /// home folder or one of its standard folders (Documents, Desktop, Downloads, Music,
    /// Pictures, Movies). A link entry is removed as the link. Never touches the database
    /// files or the keys.
    static func deleteMediaNow(audio: URL, snapshots: URL,
                               home: URL = FileManager.default.homeDirectoryForCurrentUser,
                               fileManager fm: FileManager = .default,
                               beforeRemove: () -> Void = {}) -> MediaDeleteResult {
        let audioParent = audio.deletingLastPathComponent()
        let media = snapshots.deletingLastPathComponent()
        let refuse = MediaDeleteResult(refused: media)
        guard audioParent.lastPathComponent == "Audio", snapshots.lastPathComponent == "Snapshots",
              audioParent.deletingLastPathComponent().standardizedFileURL.path == media.standardizedFileURL.path else { return refuse }
        // `<media>`, `Audio` and `Snapshots` must not be links (a missing one is fine: nothing there).
        for folder in [media, audioParent, snapshots] where lstatType(folder, fm) == .typeSymbolicLink { return refuse }
        let banned = [""] + ["Documents", "Desktop", "Downloads", "Music", "Pictures", "Movies"]
        let bannedPaths = Set(banned.map { ($0.isEmpty ? home : home.appendingPathComponent($0)).resolvingSymlinksInPath().standardizedFileURL.path } + ["/"])
        if bannedPaths.contains(media.resolvingSymlinksInPath().standardizedFileURL.path) { return refuse }
        // From here every step works on folders opened without following a link (O_NOFOLLOW)
        // and removes relative to their descriptors, so a folder swapped for a link after the
        // checks is never followed: the removal stays in the folders that were opened.
        let mediaFd = open(media.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        if mediaFd < 0 { return errno == ENOENT ? MediaDeleteResult() : refuse }
        defer { close(mediaFd) }
        let child = { (name: String) -> (fd: Int32, refused: Bool) in
            let fd = openat(mediaFd, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
            return (fd, fd < 0 && errno != ENOENT)
        }
        let audioDir = child("Audio"), snapDir = child("Snapshots")
        defer { for fd in [audioDir.fd, snapDir.fd] where fd >= 0 { close(fd) } }
        if audioDir.refused || snapDir.refused { return refuse }
        // This Mac's audio folder, and the Snapshots meeting folders that are real folders
        // carrying a regular-file marker (all judged with fstatat, never following a link).
        var targets: [(fd: Int32, name: String, url: URL)] = []
        if audioDir.fd >= 0, entryType(audioDir.fd, audio.lastPathComponent) != nil {
            targets.append((audioDir.fd, audio.lastPathComponent, audio))
        }
        if snapDir.fd >= 0 {
            for name in names(in: snapDir.fd).sorted() where entryType(snapDir.fd, name) == S_IFDIR {
                let meeting = openat(snapDir.fd, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
                guard meeting >= 0 else { continue }
                let marked = entryType(meeting, SnapshotStorage.markerName) == S_IFREG
                close(meeting)
                if marked { targets.append((snapDir.fd, name, snapshots.appendingPathComponent(name, isDirectory: true))) }
            }
        }
        beforeRemove()
        let left = targets.filter { !removeTreeNoFollow(dirfd: $0.fd, name: $0.name) }.map(\.url)
        return MediaDeleteResult(notRemoved: left)
    }

    /// The type bits (`S_IFDIR`, `S_IFREG`, `S_IFLNK`...) of `name` in the folder `dirfd`,
    /// never following a link; nil when absent.
    private static func entryType(_ dirfd: Int32, _ name: String) -> mode_t? {
        var st = stat()
        guard fstatat(dirfd, name, &st, AT_SYMLINK_NOFOLLOW) == 0 else { return nil }
        return st.st_mode & S_IFMT
    }

    /// The entry names in the folder `dirfd` (not `.`/`..`).
    private static func names(in dirfd: Int32) -> [String] {
        let dup = Darwin.dup(dirfd)
        guard dup >= 0, let dir = fdopendir(dup) else { if dup >= 0 { close(dup) }; return [] }
        defer { closedir(dir) }
        rewinddir(dir)
        var out: [String] = []
        while let e = readdir(dir) {
            var raw = e.pointee.d_name
            let size = MemoryLayout.size(ofValue: raw)
            let name = withUnsafePointer(to: &raw) {
                $0.withMemoryRebound(to: CChar.self, capacity: size) { String(cString: $0) }
            }
            if name != "." && name != ".." { out.append(name) }
        }
        return out
    }

    /// Removes `name` from the folder `dirfd` and, for a folder, everything in it, by
    /// descriptor and never following a link: a link (at any depth) is removed as the link;
    /// a sub-folder is opened with `O_NOFOLLOW | O_DIRECTORY` before its entries go, so one
    /// swapped for a link meanwhile is not entered. True when `name` is gone.
    static func removeTreeNoFollow(dirfd: Int32, name: String) -> Bool {
        var st = stat()
        guard fstatat(dirfd, name, &st, AT_SYMLINK_NOFOLLOW) == 0 else { return errno == ENOENT }
        let type = st.st_mode & S_IFMT
        guard type == S_IFDIR else { return unlinkat(dirfd, name, 0) == 0 || errno == ENOENT }
        let fd = openat(dirfd, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard fd >= 0 else { return false }
        var ok = true
        for child in names(in: fd) where !removeTreeNoFollow(dirfd: fd, name: child) { ok = false }
        close(fd)
        return ok && (unlinkat(dirfd, name, AT_REMOVEDIR) == 0 || errno == ENOENT)
    }

    /// The pane's line after a media delete that refused or left something, or nil.
    static func mediaDeleteMessage(_ r: MediaDeleteResult) -> String? {
        if let folder = r.refused {
            return String(localized: "Whimbrel did not delete recordings or slides because their folder is not safe to delete from (a link, or a top-level folder such as Documents). Remove them by hand from:") + "\n" + folder.path
        }
        return r.notRemoved.isEmpty ? nil : r.notRemoved.map(\.path).joined(separator: "\n")
    }

    /// Removes the entries; returns the ones that could not be removed.
    @discardableResult
    static func delete(_ entries: [URL], fileManager fm: FileManager = .default) -> [URL] {
        entries.filter { (try? fm.removeItem(at: $0)) == nil && fm.fileExists(atPath: $0.path) }
    }

    /// The note "Delete everything" leaves in the data folder: the paths to remove and the
    /// process that must quit first. The removal itself runs at the next launch, before the
    /// database opens, because removing an open SQLite file under a running app crashes it
    /// or lets it write a half-empty database back.
    static let pendingDeleteName = ".delete-on-launch.json"

    private struct PendingDelete: Codable { let pid: Int32; let paths: [String] }

    static func scheduleDelete(_ entries: [URL], in root: URL, pid: Int32 = ProcessInfo.processInfo.processIdentifier) throws {
        let data = try JSONEncoder().encode(PendingDelete(pid: pid, paths: entries.map(\.path)))
        try data.write(to: root.appendingPathComponent(pendingDeleteName), options: .atomic)
    }

    /// Delete everything's steps for the database and its keys (X4 Task 12), injectable so
    /// tests can record their order and make one fail. `live` is the app's.
    struct DeleteEverythingSteps {
        /// Removes the memory database's files (core's list) and returns the ones still there.
        var removeDatabaseFiles: (URL) -> [URL]
        /// Erases every database key; returns the accounts that could not be erased.
        var deleteKeys: () -> [String]
        /// Clears the launch flags that describe the deleted database.
        var clearLaunchFlags: () -> Void
        /// Removes the "a store was opened here" marker (`store_existed`, Task 13c item 4), so the
        /// next launch shows no "library not found" notice. False when it could not be removed.
        var clearStoreMarker: () -> Bool = { true }
        /// Leaves the one-shot marker that makes the next onboarding say the library is new
        /// (`FreshLibraryMarker`, Task 13c). Runs after the flags, only when the keys step ran.
        var markFreshLibrary: () -> Void = {}

        static func live(root: URL = SpikePaths.root, keys: DbKeyStore = DbKeyStore(), defaults: UserDefaults = .standard) -> Self {
            Self(removeDatabaseFiles: CoreDeleteEverything.removeDatabaseFiles,
                 deleteKeys: { keys.deleteAll().map(\.account) },
                 clearLaunchFlags: {
                     // "Faster search" is a preference, not a fact about the deleted file: kept.
                     defaults.removeObject(forKey: UserDefaultsLaunchSettings.migrateAtNextLaunchKey)
                     defaults.removeObject(forKey: UserDefaultsLaunchSettings.goodLaunchesKey)
                     // The permissions first-run sheet returns once (Mike, 2026-11-16); the OS
                     // grants themselves are never touched.
                     FirstRun.resetAfterDeleteEverything(defaults)
                 },
                 clearStoreMarker: { CoreLaunch.clearStoreExisted(dbPath: LaunchEnvironment.dbPath(in: root)) },
                 markFreshLibrary: { FreshLibraryMarker.mark(defaults) })
        }
    }

    /// What a scheduled Delete everything did.
    struct PendingDeleteResult: Equatable {
        /// Entries (and database files) that are still there.
        var notRemoved: [URL]
        /// Of those, the memory database's files: while any remains, the keys are kept.
        var databaseFilesRemaining: [URL]
        /// Keychain accounts that could not be erased (names only).
        var keysNotRemoved: [String]
        /// The key step ran (every database file was gone first).
        var keysDeleted: Bool
        /// Nothing was deleted: the old instance was still running, or the note could not be
        /// read. The database opens as before; the user is told at once.
        var didNotRun = false
        /// Note entries outside Whimbrel's folders (as written in the note): never deleted.
        var rejected: [String] = []
        /// The note, when it could not be read or removed (it may run again next launch).
        var noteLeft: URL? = nil
        /// The `store_existed` marker, when it could not be removed.
        var storeMarkerLeft: URL? = nil

        var incomplete: Bool {
            didNotRun || !notRemoved.isEmpty || !keysNotRemoved.isEmpty || !rejected.isEmpty || noteLeft != nil || storeMarkerLeft != nil
        }
    }

    /// Runs a scheduled "Delete everything", once the instance that scheduled it has quit.
    /// Returns nil when nothing is pending. The note is removed whatever happens, so a later
    /// launch never deletes data made after this one: if the old instance is still running
    /// after `timeout` (or the note can't be read), nothing is removed, every entry comes
    /// back as not removed, and no key or flag is touched.
    @discardableResult
    static func runPendingDelete(in root: URL, steps: DeleteEverythingSteps,
                                 timeout: TimeInterval = 10,
                                 isRunning: (Int32) -> Bool = { kill($0, 0) == 0 },
                                 fileManager fm: FileManager = .default) -> PendingDeleteResult? {
        let note = root.appendingPathComponent(pendingDeleteName)
        // Only a missing note means nothing is pending. Any other read failure, or a note that
        // does not decode, deletes nothing and leaves the note for the user to see.
        let data: Data
        do {
            data = try Data(contentsOf: note)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return nil
        } catch {
            return PendingDeleteResult(notRemoved: [note], databaseFilesRemaining: [], keysNotRemoved: [], keysDeleted: false,
                                       didNotRun: true, noteLeft: note)
        }
        guard let pending = try? JSONDecoder().decode(PendingDelete.self, from: data) else {
            return PendingDeleteResult(notRemoved: [note], databaseFilesRemaining: [], keysNotRemoved: [], keysDeleted: false,
                                       didNotRun: true, noteLeft: note)
        }
        // From here the note is removed whatever happens, so a later launch never deletes data
        // made after this one; a note that cannot be removed is said in the alert.
        var noteLeft: URL?
        let removeNote = { () -> URL? in
            do { try fm.removeItem(at: note); return nil } catch { return fm.fileExists(atPath: note.path) ? note : nil }
        }
        let entries = pending.paths.map { URL(fileURLWithPath: $0) }
        let deadline = Date().addingTimeInterval(timeout)
        while isRunning(pending.pid) {
            if Date() >= deadline {
                noteLeft = removeNote()
                return PendingDeleteResult(notRemoved: entries, databaseFilesRemaining: [], keysNotRemoved: [], keysDeleted: false,
                                           didNotRun: true, noteLeft: noteLeft)
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        // The note sits in the data folder, where anything could write it: only entries
        // confined to Whimbrel's own folders are deleted (`confine`); the rest are named.
        var allowed: [URL] = []
        var rejected: [String] = []
        for raw in pending.paths {
            // The kept Google accounts list survives the delete (Task 15), even when a note
            // names it; it is skipped, not rejected (a rejected entry is reported as foreign).
            if URL(fileURLWithPath: raw).lastPathComponent.lowercased() == KeptGoogleAccounts.fileName.lowercased() { continue }
            if let ok = confine(raw, root: root) { allowed.append(ok) } else { rejected.append(raw) }
        }
        // 1. The database files first (core's list: the database, its sidecars, the
        //    unencrypted copy, the encryption temp and moved-aside files, the state file).
        let dbFilesLeft = steps.removeDatabaseFiles(LaunchEnvironment.dbPath(in: root))
        // 2. The rest of what the note lists (already-gone database files are not reported).
        //    A symlink entry is removed itself (`removeItem` never follows it).
        var left = delete(allowed, fileManager: fm)
        for url in dbFilesLeft where !left.contains(where: { $0.path == url.path }) { left.append(url) }
        left += rejected.map { URL(fileURLWithPath: $0) }
        noteLeft = removeNote()
        // 3. The keys and the launch flags only once no database file is left: an encrypted
        //    file whose key is gone can never be opened again, so a file that would not go
        //    keeps its key and the user is told which files remain.
        guard dbFilesLeft.isEmpty else {
            return PendingDeleteResult(notRemoved: left, databaseFilesRemaining: dbFilesLeft, keysNotRemoved: [], keysDeleted: false,
                                       rejected: rejected, noteLeft: noteLeft)
        }
        let keysNotRemoved = steps.deleteKeys()
        steps.clearLaunchFlags()
        let markerLeft = steps.clearStoreMarker() ? nil : root.appendingPathComponent("store_existed")
        steps.markFreshLibrary()
        return PendingDeleteResult(notRemoved: left, databaseFilesRemaining: [], keysNotRemoved: keysNotRemoved, keysDeleted: true,
                                   rejected: rejected, noteLeft: noteLeft, storeMarkerLeft: markerLeft)
    }

    /// The note's rule (core `api::delete_everything::confine_to_root`): `raw` may be deleted
    /// only as an absolute direct child of the data folder, with every symlink on the way
    /// resolved and no `.`/`..`; never the note itself. Returns the resolved path, or nil. The
    /// media folder is never in the note: recordings and slides go at confirm time
    /// (`deleteMediaNow`).
    static func confine(_ raw: String, root: URL) -> URL? {
        guard raw.hasPrefix("/"), let p = CoreDeleteEverything.confine(root: root, candidate: raw) else { return nil }
        // Never the note itself (any case: the data folder may be case-insensitive).
        guard p.lastPathComponent.lowercased() != pendingDeleteName.lowercased() else { return nil }
        return p
    }

    /// The Delete everything sheet's body: what goes, and what stays (Task 15: connected
    /// Google accounts stay connected).
    static func deleteSheetBody(dataFolder: String, mediaFolder: String, media: String) -> String {
        String(localized: "This removes the database and the data folder (\(dataFolder)), and this Mac's recordings and slide images in \(mediaFolder) (\(media)), then restarts Whimbrel as new. Settings, keys, connected Google accounts and downloaded models stay.")
    }

    /// The sheet's error when the kept Google accounts list could not be written (Task 15,
    /// Mike 2026-11-17: the delete stops, as on Windows).
    static let googleAccountsNotKeptFailure = String(localized: "Could not keep your connected Google accounts. Nothing was deleted; try again.")

    static let deleteDidNotRunMessage = String(localized: "Delete everything did not finish. Your recordings and slides were deleted unless Your data says otherwise. Nothing else has been deleted, and your data opens as before. Use Delete everything again.")

    /// "Delete and restart"'s words when the restart cannot start or the old instance does
    /// not close (`AppRestart.relaunchAfterExit`).
    static let restartMessages = AppRestart.Messages(
        failed: String(localized: "Whimbrel could not restart by itself. Your recordings and slides were deleted unless Your data says otherwise. Nothing else has been deleted yet. Quit Whimbrel and open it again to finish Delete everything."),
        stuck: String(localized: "Whimbrel did not close in time, so it was not opened again. Your recordings and slides were deleted unless Your data says otherwise. Nothing else has been deleted yet. Quit Whimbrel and open it again to finish Delete everything."))

    /// What the launch after an unfinished Delete everything tells the user at once, or nil
    /// when no database file and no key is left (other leftovers are listed in the Your data
    /// pane only).
    static func deleteIncompleteMessage(_ r: PendingDeleteResult) -> String? {
        var parts: [String] = []
        if r.didNotRun { parts.append(deleteDidNotRunMessage) }
        if !r.databaseFilesRemaining.isEmpty {
            parts.append(String(localized: "Delete everything did not finish. These memory database files could not be removed, so Whimbrel kept the key that opens them:"))
            parts.append(r.databaseFilesRemaining.map(\.path).joined(separator: "\n"))
            parts.append(String(localized: "Use Delete everything again, or quit Whimbrel and remove them by hand."))
        }
        if !r.rejected.isEmpty {
            parts.append(String(localized: "These entries in the delete note are outside Whimbrel's folders, so they were not deleted:"))
            parts.append(r.rejected.joined(separator: "\n"))
        }
        if let note = r.noteLeft {
            parts.append(String(localized: "Whimbrel could not read or remove its delete note. Remove this file by hand, or Delete everything may run again at the next launch:"))
            parts.append(note.path)
        }
        if let marker = r.storeMarkerLeft {
            parts.append(EncryptionCopy.storeMarkerNotRemoved)
            parts.append(marker.path)
        }
        if !r.keysNotRemoved.isEmpty {
            parts.append(String(localized: "Delete everything removed the memory database, but these Keychain items could not be removed:"))
            parts.append(r.keysNotRemoved.joined(separator: "\n"))
            parts.append(String(localized: "Delete them in Keychain Access, or use Delete everything again."))
        }
        return parts.isEmpty ? nil : parts.joined(separator: "\n\n")
    }

    // MARK: Set-aside files (Task 13c item 3)

    /// The Your data row's text for the set-aside `.foreign-*` files; nil (row hidden) when none.
    static func foreignFilesSummary(_ files: [CoreForeignFiles.File]) -> String? {
        files.isEmpty ? nil : EncryptionCopy.foreignFilesSummary(count: files.count, size: formatBytes(files.reduce(0) { $0 + $1.size }))
    }

    /// The confirmation's body: the count and size shown, and that they cannot be recovered.
    static func foreignFilesConfirmBody(_ files: [CoreForeignFiles.File]) -> String {
        EncryptionCopy.foreignFilesConfirmBody(count: files.count, size: formatBytes(files.reduce(0) { $0 + $1.size }))
    }

    /// What to say after "Delete these files": nil when everything went; the names that would
    /// not go; or that the folder could not be read (`nil` result, nothing deleted).
    static func foreignFilesDeleteMessage(_ r: CoreForeignFiles.DeleteResult?) -> String? {
        guard let r else { return EncryptionCopy.foreignFilesUnreadable }
        return r.failed.isEmpty ? nil : ([EncryptionCopy.foreignFilesNotDeleted] + r.failed).joined(separator: "\n")
    }

    /// UserDefaults key holding what the last "Delete everything" could not remove, one path
    /// (or Keychain item) per line; the Your data pane shows it until dismissed.
    static let deleteLeftoversKey = "yourData.deleteLeftovers"

    /// The launch after an unfinished Delete everything, before the database launch runs:
    /// the leftovers go to the Your data pane, and when a database file or a key is left the
    /// user is told at once (the launch may otherwise stop on an encryption screen that never
    /// says a delete was under way). Nothing is retried by itself.
    static func reportIncompleteDelete(_ r: PendingDeleteResult, defaults: UserDefaults = .standard) {
        let lines = r.notRemoved.map(\.path) + (r.storeMarkerLeft.map { [$0.path] } ?? [])
            + r.keysNotRemoved.map { String(localized: "Keychain item \($0)") }
        // After what the confirm step left (recordings and slides), if anything.
        let earlier = defaults.string(forKey: deleteLeftoversKey).map { $0.isEmpty ? [] : [$0] } ?? []
        defaults.set((earlier + lines).joined(separator: "\n"), forKey: deleteLeftoversKey)
        Logger(subsystem: "com.mpointer.kestrel", category: "your-data")
            .error("Delete everything could not remove \(r.notRemoved.count) item(s) and \(r.keysNotRemoved.count) key(s); database files left: \(r.databaseFilesRemaining.count).")
        guard let message = deleteIncompleteMessage(r) else { return }
        _ = NSApplication.shared
        NSApp.activate(ignoringOtherApps: true)
        AppRestart.showAlert(message)
    }
}

/// Connected Google accounts stay connected through Delete everything (Task 15, Mike
/// 2026-11-17; Windows does the same). The account list lives in the database
/// (`connector_account`), which Delete everything removes, while the OAuth tokens live in
/// the Keychain and stay. So the confirm step writes each Google connector's emails and
/// default to `fileName` in the data folder, which the delete keeps; the first open of the
/// new database re-adds each account whose sign-in is still in the Keychain, then removes
/// the file. Gmail's history id and Calendar's sync token are in the database, so the new
/// library fetches the mail and events again. Nothing is disconnected or revoked.
/// Outlook 365 is not kept (out of scope): after Delete everything its accounts must be
/// connected again, as before.
enum KeptGoogleAccounts {
    static let fileName = ".kept-google-accounts.json"
    /// The Google connectors whose accounts are kept (`ConnectedAccountStore` ids).
    static let connectorIds = ["calendar", "gmail"]

    struct Connector: Codable, Equatable {
        var emails: [String]
        var defaultEmail: String?
    }

    struct Contents: Codable, Equatable {
        var version = 1
        /// Keyed by connector id (`connectorIds`).
        var connectors: [String: Connector]
    }

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "your-data")

    /// The Keychain account that holds `email`'s tokens for `connectorId`: the same name each
    /// connector state's `makeHelper` gives its `GoogleOAuthHelper`.
    static func keychainAccount(connectorId: String, email: String) -> String? {
        switch connectorId {
        case "gmail": return "gmail-connector:\(email)"
        case "calendar": return "google-calendar-connector:\(email)"
        default: return nil
        }
    }

    /// Whether `email`'s sign-in for `connectorId` is still in the Keychain. Asks only whether
    /// the item exists; never reads the tokens.
    static func hasStoredSignIn(connectorId: String, email: String) -> Bool {
        guard let account = keychainAccount(connectorId: connectorId, email: email) else { return false }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: GoogleOAuthHelper.keychainService,
            kSecAttrAccount as String: account,
        ]
        return KeychainAccess.copyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    /// The account names of every Google OAuth item in the Keychain (service
    /// `GoogleOAuthHelper.keychainService`). Attributes only: never `kSecReturnData`, so no
    /// token is read. Empty when there is none (`errSecItemNotFound`); nil when the Keychain
    /// could not be read (any other status), which stops Delete everything.
    static func googleOAuthAccountNames(
        copyMatching: (CFDictionary, UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus = KeychainAccess.copyMatching
    ) -> [String]? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: GoogleOAuthHelper.keychainService,
            kSecMatchLimit as String: kSecMatchLimitAll,
            kSecReturnAttributes as String: true,
        ]
        var result: CFTypeRef?
        let status = copyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess, let items = result as? [[String: Any]] else {
            log.error("Kept Google accounts: the Keychain could not be listed (status \(status, privacy: .public)).")
            return nil
        }
        return items.compactMap { $0[kSecAttrAccount as String] as? String }
    }

    /// The kept list rebuilt from Keychain account names (no open store at confirm time):
    /// `gmail-connector:<email>` and `google-calendar-connector:<email>`, without the
    /// `#issuer` records; no default (the restore makes the first account the default).
    static func connectors(fromKeychainAccounts accounts: [String]) -> [String: Connector] {
        var out: [String: Connector] = [:]
        for connectorId in connectorIds {
            guard let prefix = keychainAccount(connectorId: connectorId, email: "") else { continue }
            let emails = Set(accounts.filter { $0.hasPrefix(prefix) && !$0.hasSuffix("#issuer") }
                .map { String($0.dropFirst(prefix.count)) }.filter { !$0.isEmpty })
            if !emails.isEmpty { out[connectorId] = Connector(emails: emails.sorted(), defaultEmail: nil) }
        }
        return out
    }

    /// Writes the list (mode 0600: it holds email addresses), atomically: a temporary file
    /// created 0600 in the same folder, renamed over `fileName`, then the folder is synced.
    /// With no account at all nothing is written and any earlier list is removed (only called
    /// so with an open store, where the user really has no Google account), so it is never
    /// restored later.
    static func write(_ connectors: [String: Connector], in root: URL) throws {
        let kept = connectors.filter { connectorIds.contains($0.key) && !$0.value.emails.isEmpty }
        let target = root.appendingPathComponent(fileName)
        guard !kept.isEmpty else {
            if unlink(target.path) != 0, errno != ENOENT { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
            return
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(Contents(connectors: kept))
        let temp = root.appendingPathComponent(fileName + ".tmp-\(UUID().uuidString)")
        let fd = open(temp.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard fd >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        var failure: Int32 = 0
        if fchmod(fd, 0o600) != 0 { failure = errno }
        if failure == 0 {
            failure = data.withUnsafeBytes { raw -> Int32 in
                var offset = 0
                while offset < raw.count {
                    let n = Darwin.write(fd, raw.baseAddress! + offset, raw.count - offset)
                    if n < 0 { if errno == EINTR { continue }; return errno }
                    offset += n
                }
                return 0
            }
        }
        if failure == 0, fsync(fd) != 0 { failure = errno }
        if close(fd) != 0, failure == 0 { failure = errno }
        if failure == 0, rename(temp.path, target.path) != 0 { failure = errno }
        if failure != 0 {
            unlink(temp.path)
            throw POSIXError(POSIXErrorCode(rawValue: failure) ?? .EIO)
        }
        // The rename itself reaches the disk (best effort: the file is already in place).
        let dir = open(root.path, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        if dir >= 0 { _ = fsync(dir); close(dir) }
    }

    /// What Delete everything's confirm step did with the list.
    enum KeepOutcome: Equatable {
        /// The list was written (this run's file).
        case wrote
        /// No store is open and a list from an earlier Delete everything is there (its restore
        /// has not run yet): left exactly as it is.
        case keptExisting
        /// No Google account to keep (no file left behind).
        case nothingToKeep
        /// The list could not be written: the delete must stop (Mike 2026-11-17, as Windows).
        case failed
    }

    /// Delete everything's confirm step. `connectors` is nil when no store is open ("continue
    /// without store"): the connector states then show no account, so the list is rebuilt from
    /// the Keychain's Google OAuth items (`keychainAccounts`, names only; a Keychain that cannot
    /// be read is `.failed`), and an existing list is never overwritten or removed. With a store
    /// open the connector states' accounts are written over any existing list (an empty one
    /// removes it: the user really has no account). Such a list can only be one the restore
    /// could not remove, whose accounts were already restored into the open store, so replacing
    /// or removing it loses nothing. Logs counts and error codes only.
    static func keepForDelete(_ connectors: [String: Connector]?, in root: URL,
                              keychainAccounts: () -> [String]? = { googleOAuthAccountNames() },
                              write: ([String: Connector], URL) throws -> Void = KeptGoogleAccounts.write) -> KeepOutcome {
        let target = root.appendingPathComponent(fileName)
        let wanted: [String: Connector]
        if let connectors {
            wanted = connectors
        } else {
            var st = stat()
            if lstat(target.path, &st) == 0 || errno != ENOENT { return .keptExisting }
            // A Keychain that cannot be read is a failed keep: the delete stops.
            guard let names = keychainAccounts() else { return .failed }
            wanted = Self.connectors(fromKeychainAccounts: names)
            log.info("\(rebuildLogMessage(accounts: wanted.values.reduce(0) { $0 + $1.emails.count }), privacy: .public)")
        }
        let count = wanted.filter { connectorIds.contains($0.key) }.values.reduce(0) { $0 + $1.emails.count }
        if count == 0, connectors == nil { return .nothingToKeep }
        do {
            try write(wanted, root)
            return count == 0 ? .nothingToKeep : .wrote
        } catch {
            let code = (error as? POSIXError)?.code.rawValue ?? Int32((error as NSError).code)
            log.error("\(writeFailedLogMessage(accounts: count, code: code), privacy: .public)")
            return .failed
        }
    }

    /// What a restore did.
    struct RestoreResult: Equatable {
        /// Accounts present after the restore (re-added, or already there).
        var restored = 0
        /// Accounts left out because their sign-in is no longer in the Keychain.
        var dropped = 0
        /// The file could not be read or decoded; it was removed and nothing was added.
        var unreadable = false
        /// The file could not be removed afterwards (the next launch restores again, adding nothing).
        var fileLeft = false
    }

    /// The first open of the new database (`AppState.init`, before the connector states read
    /// their accounts): re-adds each kept account whose sign-in is still in the Keychain,
    /// through `ConnectedAccountStore`, sets the kept default, then removes the file. Nil when
    /// there is no file or no open store (the file then waits for a launch that opens one).
    /// Idempotent: an account already there is not added twice, so a crash before the file
    /// goes only repeats the same adds. An unreadable file is removed and logged; it never
    /// stops the launch.
    @MainActor
    @discardableResult
    static func restore(in root: URL, store: CoreMemoryStore?,
                        hasSignIn: (String, String) -> Bool = { hasStoredSignIn(connectorId: $0, email: $1) },
                        defaults: UserDefaults = .standard,
                        fileManager fm: FileManager = .default) -> RestoreResult? {
        guard let store else { return nil }
        let file = root.appendingPathComponent(fileName)
        let data: Data
        do {
            data = try Data(contentsOf: file)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return nil
        } catch {
            return finish(RestoreResult(unreadable: true), file: file, fm: fm)
        }
        guard let contents = try? JSONDecoder().decode(Contents.self, from: data) else {
            return finish(RestoreResult(unreadable: true), file: file, fm: fm)
        }
        var result = RestoreResult()
        for connectorId in connectorIds {
            guard let kept = contents.connectors[connectorId], !kept.emails.isEmpty else { continue }
            let registry = ConnectedAccountStore(connectorId: connectorId, store: store, defaults: defaults)
            for email in kept.emails {
                guard hasSignIn(connectorId, email) else { result.dropped += 1; continue }
                if !registry.emails.contains(email) { registry.addDuringInit(email) }
                if registry.emails.contains(email) { result.restored += 1 }
            }
            // Whenever the kept default is present, newly added or not: a crash after the adds
            // but before this step leaves every account present at the next launch, and the
            // default must still be applied (as Windows). A file that cannot be removed applies
            // it again at each launch; that rarity is logged (`fileLeft`).
            if let wanted = kept.defaultEmail, registry.emails.contains(wanted), registry.defaultEmail != wanted {
                registry.makeDefaultDuringInit(wanted)
            }
        }
        return finish(result, file: file, fm: fm)
    }

    private static func finish(_ r: RestoreResult, file: URL, fm: FileManager) -> RestoreResult {
        var r = r
        if (try? fm.removeItem(at: file)) == nil, fm.fileExists(atPath: file.path) { r.fileLeft = true }
        if r.unreadable || r.fileLeft {
            log.error("\(restoreLogMessage(r), privacy: .public)")
        } else {
            log.info("\(restoreLogMessage(r), privacy: .public)")
        }
        return r
    }

    /// The restore's log line: counts only, never an email address.
    static func restoreLogMessage(_ r: RestoreResult) -> String {
        if r.unreadable {
            return "Kept Google accounts: the list could not be read, so no account was restored" + (r.fileLeft ? "; the file could not be removed." : "; the file was removed.")
        }
        return "Kept Google accounts: restored \(r.restored) account(s), dropped \(r.dropped) with no stored sign-in"
            + (r.fileLeft ? "; the file could not be removed." : ".")
    }

    /// The confirm step's failure log line: counts and an error code only.
    static func writeFailedLogMessage(accounts: Int, code: Int32) -> String {
        "Kept Google accounts: could not write the list of \(accounts) account(s) (error \(code)); Delete everything stopped."
    }

    /// The store-less rebuild's log line: a count only.
    static func rebuildLogMessage(accounts: Int) -> String {
        "Kept Google accounts: no store is open, so the list was rebuilt from \(accounts) Keychain sign-in(s)."
    }
}
