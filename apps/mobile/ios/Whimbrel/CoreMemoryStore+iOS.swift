import Foundation

// The Mac's `final class CoreMemoryStore` lives in CoreBridge.swift, which is Mac-only.
// On iOS this is the type the shared SyncCoreBridge.swift extends, plus the few store
// calls the phone makes (the same FFI calls the Mac's CoreBridge.swift wraps).
#if os(iOS)
/// The phone's local memory store (pull-only: no capture, no ingest). Not thread-safe:
/// `SyncModel` serializes every call on one queue, as the Mac does with `accessQueue`.
final class CoreMemoryStore {
    let handle: OpaquePointer

    /// Opens (creating if needed) the store at `path`, with no embedding model.
    init?(path: URL) {
        guard let handle = path.path.withCString({ whimbrel_memory_open($0, nil) }) else { return nil }
        self.handle = handle
    }

    deinit {
        whimbrel_memory_close(handle)
    }

    /// `Application Support/Sync/memory.sqlite`. The store holds decrypted content, so its
    /// directory is kept out of backups (see `prepareStoreDirectory`).
    static func defaultPath() throws -> URL {
        let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        // Dev builds before this fix kept the store directly in Application Support, where
        // backups include it. M4.0 has no released users: delete it (it re-syncs).
        for name in ["memory.sqlite", "memory.sqlite-wal", "memory.sqlite-shm"] {
            try? FileManager.default.removeItem(at: support.appendingPathComponent(name))
        }
        let dir = support.appendingPathComponent("Sync", isDirectory: true)
        try prepareStoreDirectory(dir)
        return dir.appendingPathComponent("memory.sqlite")
    }

    /// Readable after first unlock (so a later background pull works); the SQLite file and
    /// its -wal/-shm inherit the directory's class when created.
    static let storeDirectoryAttributes: [FileAttributeKey: Any] = [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]

    /// Creates `dir` if missing with `storeDirectoryAttributes`, (re)applies them, and
    /// excludes it (and so everything in it) from iCloud and Finder backups.
    static func prepareStoreDirectory(_ dir: URL) throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: storeDirectoryAttributes)
        try FileManager.default.setAttributes(storeDirectoryAttributes, ofItemAtPath: dir.path)
        var dir = dir
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try dir.setResourceValues(values)
    }

    @discardableResult
    func setDeviceId(_ deviceId: String) -> Bool {
        deviceId.withCString { whimbrel_memory_set_device_id(handle, $0) }
    }

    /// One pull-only content-sync round (`whimbrel_sync_content_pull_only`). Blocks for
    /// up to the core's 30 s timeout; never call on the main thread.
    func syncContentPullOnly(tursoURL: String, tursoToken: String, groupKeyBase64: String, deviceId: String, captureWindowDays: Int32) -> Bool {
        tursoURL.withCString { url in
            tursoToken.withCString { token in
                groupKeyBase64.withCString { key in
                    deviceId.withCString { device in
                        whimbrel_sync_content_pull_only(handle, url, token, key, device, captureWindowDays)
                    }
                }
            }
        }
    }

    /// CoreBridge.swift's `syncSettingsIteration`: one settings round trip, returning the
    /// pulled delta (nil on any failure). Blocking network call; uses no store handle.
    static func syncSettingsIteration(tursoURL: String, tursoToken: String, groupKeyBase64: String, localJSON: String) -> [String: PulledSettingsField]? {
        guard let json = tursoURL.withCString({ url in
            tursoToken.withCString { token in
                groupKeyBase64.withCString { key in
                    localJSON.withCString { local in whimbrel_sync_settings_iteration(url, token, key, local) }
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Iteration: Decodable { let pulled: [String: PulledSettingsField] }
        return (try? JSONDecoder().decode(Iteration.self, from: Data(String(cString: json).utf8)))?.pulled
    }

    /// CoreBridge.swift's `syncSettingsApplyPlan` (pure): the pulled fields to apply.
    static func syncSettingsApplyPlan(pulled: [String: PulledSettingsField], localTimestamps: [String: Int64], apiKeySyncEnabled: [String: Bool]) -> [PulledSettingsField]? {
        let enc = JSONEncoder()
        guard let p = (try? enc.encode(pulled)).flatMap({ String(data: $0, encoding: .utf8) }),
              let t = (try? enc.encode(localTimestamps)).flatMap({ String(data: $0, encoding: .utf8) }),
              let k = (try? enc.encode(apiKeySyncEnabled)).flatMap({ String(data: $0, encoding: .utf8) }) else { return nil }
        guard let json = p.withCString({ pp in t.withCString { tt in k.withCString { kk in whimbrel_sync_settings_apply_plan(pp, tt, kk) } } }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Plan: Decodable { let fields: [PulledSettingsField] }
        return (try? JSONDecoder().decode(Plan.self, from: Data(String(cString: json).utf8)))?.fields
    }

    /// How many entries the Mac's Meetings list shows for this store: the same call and
    /// limit as `MeetingsListView` (`recentMeetingsGrouped(limit: 200)`), so copies of one
    /// call count once. Nil on failure.
    func groupedMeetingCount(limit: Int = 200) -> Int? {
        guard let json = whimbrel_meetings_recent_grouped(handle, limit) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: json).utf8)) as? [Any])?.count
    }

    /// CoreBridge.swift's `lastSyncContentError`: why the last content sync failed.
    static func lastSyncContentError() -> String? {
        guard let msg = whimbrel_sync_content_last_error() else { return nil }
        defer { whimbrel_core_free_string(msg) }
        return String(cString: msg)
    }

    #if DEBUG
    /// Demo seeding only (`-WhimbrelDemoSynced`): a completed meeting, as a Mac would push.
    func insertCompletedMeetingForDemo(deviceId: String, title: String, startUnixMs: Int64, minutes: Int64) {
        let meeting: [String: Any] = [
            "device_id": deviceId, "start_unix_ms": startUnixMs, "end_unix_ms": NSNull(), "title": title,
            "attendees": [String](), "trigger": "calendar", "audio_path": NSNull(), "status": "recording",
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: meeting),
              let reply = String(data: data, encoding: .utf8).flatMap({ s in s.withCString { whimbrel_memory_insert_meeting(handle, $0) } })
        else { return }
        defer { whimbrel_core_free_string(reply) }
        guard let id = (try? JSONSerialization.jsonObject(with: Data(String(cString: reply).utf8)) as? [String: Any])?["id"] as? Int64 else { return }
        whimbrel_memory_complete_meeting(handle, id, startUnixMs + minutes * 60_000, nil)
    }
    #endif
}

/// The Ask calls (CoreBridge.swift's `prepareAsk`/`finalizeAsk`/`discardAsk`, minus the Mac
/// types) and the citation cards for an answer.
extension CoreMemoryStore: PhoneAskStore {
    /// Same context limit as the Mac's Ask.
    private static let askContextLimit = 5

    func prepareAsk(query: String, historyJSON: String, routingJSON: String) -> PrepareResult {
        guard let json = query.withCString({ q in
            historyJSON.withCString { h in
                routingJSON.withCString { r in whimbrel_llm_prepare_ask(handle, q, Self.askContextLimit, h, r) }
            }
        }) else { return .failed }
        defer { whimbrel_core_free_string(json) }
        struct Envelope: Decodable {
            struct Data: Decodable {
                let prepareId: String
                let prompt: String
                private enum CodingKeys: String, CodingKey { case prepareId = "prepare_id", prompt }
            }
            struct Failure: Decodable { let kind: String }
            let ok: Bool
            let data: Data?
            let error: Failure?
        }
        guard let env = try? JSONDecoder().decode(Envelope.self, from: Data(String(cString: json).utf8)) else { return .failed }
        if env.ok, let d = env.data { return .ok(prepareId: d.prepareId, prompt: d.prompt) }
        return env.error?.kind == "spend_cap" ? .spendCap : .failed
    }

    /// Citations for a finished answer, as cards: nil only if the core call failed.
    func finalizeAsk(prepareId: String, text: String, model: String, inputTokens: Int64, outputTokens: Int64) -> [CitationCard]? {
        guard let json = prepareId.withCString({ p in
            text.withCString { t in
                model.withCString { m in whimbrel_llm_finalize_ask(handle, p, t, m, inputTokens, outputTokens) }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Finalized: Decodable { let citations: [CitationRef] }
        guard let refs = (try? JSONDecoder().decode(Finalized.self, from: Data(String(cString: json).utf8)))?.citations else { return nil }
        return resolveCitations(refs)
    }

    func discardAsk(prepareId: String) {
        prepareId.withCString { whimbrel_llm_discard_ask($0) }
    }

    func resolveCitations(_ refs: [CitationRef]) -> [CitationCard] {
        guard !refs.isEmpty, let data = try? JSONEncoder().encode(refs), let refsJSON = String(data: data, encoding: .utf8),
              let json = refsJSON.withCString({ whimbrel_citations_resolve(handle, $0) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode([CitationCard].self, from: Data(String(cString: json).utf8))) ?? []
    }
}
/// A capture row as the phone shows it.
struct PhoneCaptureItem: Decodable, Equatable {
    let windowTitle: String
    let appBundleId: String
    let url: String?
    let text: String

    var title: String { windowTitle.isEmpty ? appBundleId : windowTitle }

    private enum CodingKeys: String, CodingKey { case windowTitle = "window_title", appBundleId = "app_bundle_id", url, text }
}

/// Separate from `PhoneReadStore` so the screens' fakes needn't know about captures.
protocol PhoneCaptureStore: AnyObject {
    func captureItem(id: Int64) -> PhoneCaptureItem?
}

extension CoreMemoryStore: PhoneCaptureStore {
    func captureItem(id: Int64) -> PhoneCaptureItem? {
        guard let json = whimbrel_memory_get_capture_item(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(PhoneCaptureItem.self, from: Data(String(cString: json).utf8))
    }
}
#endif
