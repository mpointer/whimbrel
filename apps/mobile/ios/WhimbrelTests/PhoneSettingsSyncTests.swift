import XCTest
@testable import Whimbrel

/// Scripted core: one pulled delta per round, and a plan that honours the key rule (a key
/// field only while its toggle is on) like `apply_plan` does.
final class FakeSettingsPuller: SettingsPulling, @unchecked Sendable {
    /// Runs inside `applyPlan`, like a revoke landing mid-pull.
    var onApplyPlan: (() -> Void)?
    var rounds: [[String: PulledSettingsField]] = []
    private(set) var localJSONs: [String] = []
    private(set) var toggles: [[String: Bool]] = []
    private(set) var timestamps: [[String: Int64]] = []

    func iterate(credentials: SyncCredentials, localJSON: String) -> [String: PulledSettingsField]? {
        localJSONs.append(localJSON)
        return rounds.isEmpty ? [:] : rounds.removeFirst()
    }

    private var current: [String: PulledSettingsField] = [:]
    func applyPlan(pulled: [String: PulledSettingsField], localTimestamps: [String: Int64], apiKeySyncEnabled: [String: Bool]) -> [PulledSettingsField]? {
        toggles.append(apiKeySyncEnabled)
        onApplyPlan?()
        timestamps.append(localTimestamps)
        return pulled.filter { key, field in
            if key.hasPrefix("apiKey."), !key.hasSuffix(".syncEnabled") { return apiKeySyncEnabled[key] == true }
            return field.updatedAtUnixMs > (localTimestamps[key] ?? 0)
        }.map { key, field in
            PulledSettingsField(fieldKey: key, value: field.value, updatedAtUnixMs: field.updatedAtUnixMs, isDeleted: field.isDeleted)
        }
    }
}

final class PhoneSettingsSyncTests: XCTestCase {
    private let creds = SyncCredentials(tursoToken: "t", tursoDatabaseUrl: "u", groupKeyBase64: "k", deviceId: "phone")
    private let p = "com.mpointer.kestrel.phone."
    private var suite = ""
    private var defaults: UserDefaults!
    private var puller: FakeSettingsPuller!
    private var sync: PhoneSettingsSync!
    private let keys = PhoneKeyStore()

    override func setUp() {
        suite = "PhoneSettingsSyncTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
        puller = FakeSettingsPuller()
        sync = PhoneSettingsSync(puller: puller, defaults: defaults, keys: keys)
        AskProvider.allCases.forEach { keys.delete(for: $0) }
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        AskProvider.allCases.forEach { keys.delete(for: $0) }
    }

    private func field(_ value: String, at ts: Int64 = 100, deleted: Bool = false) -> PulledSettingsField {
        PulledSettingsField(fieldKey: "", value: value, updatedAtUnixMs: ts, isDeleted: deleted)
    }

    func test_toggle_then_key_arrives_next_round() {
        let delta = ["apiKey.anthropic.syncEnabled": field("true"), "apiKey.anthropic": field("sk-x")]
        puller.rounds = [delta, delta]
        sync.pullOnce(credentials: creds)
        XCTAssertNil(keys.key(for: .anthropic))
        XCTAssertEqual(puller.toggles[0]["apiKey.anthropic"], false)
        sync.pullOnce(credentials: creds)
        XCTAssertEqual(puller.toggles[1]["apiKey.anthropic"], true)
        XCTAssertEqual(keys.key(for: .anthropic), "sk-x")
        XCTAssertEqual(defaults.object(forKey: p + "apiKey.anthropic.syncEnabled") as? Bool, true)
        XCTAssertEqual(defaults.object(forKey: p + "settingsTs.apiKey.anthropic") as? Int64, 100)
    }

    func test_deleted_key_field_removes_keychain_item() {
        defaults.set(true, forKey: p + "apiKey.openai.syncEnabled")
        keys.save("sk-old", for: .openai)
        puller.rounds = [["apiKey.openai": field("", at: 200, deleted: true)]]
        sync.pullOnce(credentials: creds)
        XCTAssertNil(keys.key(for: .openai))
    }

    func test_routing_fields_land_in_defaults() {
        puller.rounds = [[
            "routing.preferredProvider": field("anthropic"),
            "routing.localOnlyMode": field("true"),
            "routing.anthropicDefaultModel": field("claude-x"),
            "routing.userDailyCapCents": field("250"),
            "routing.globalDailyCapCents": field(""),
            "summary.customPromptInstructions": field("be brief"),
        ]]
        sync.pullOnce(credentials: creds)
        let prefs = PhoneRoutingPrefs(defaults: defaults)
        XCTAssertEqual(prefs.preferredProvider, "anthropic")
        XCTAssertTrue(prefs.localOnlyMode)
        XCTAssertEqual(prefs.defaultModel(for: .anthropic), "claude-x")
        XCTAssertNil(prefs.defaultModel(for: .openai))
        XCTAssertEqual(prefs.userDailyCapCents, 250)
        XCTAssertNil(prefs.globalDailyCapCents)
        XCTAssertEqual(prefs.customInstructions, "be brief")
    }

    func test_never_pushes() throws {
        sync.pullOnce(credentials: creds)
        let json = try JSONSerialization.jsonObject(with: Data(try XCTUnwrap(puller.localJSONs.first).utf8)) as? [String: Any]
        XCTAssertEqual((json?["fields"] as? [String: Any])?.count, 0)
        XCTAssertEqual((json?["api_keys"] as? [Any])?.count, 0)
    }

    func test_unknown_field_is_ignored_and_timestamp_untouched() {
        puller.rounds = [["episode.hidden.1": field("true"), "profile.name": field("x")]]
        sync.pullOnce(credentials: creds)
        XCTAssertNil(defaults.object(forKey: p + "profile.name"))
        XCTAssertNil(defaults.object(forKey: p + "settingsTs.profile.name"))
        XCTAssertNil(defaults.object(forKey: p + "settingsTs.episode.hidden.1"))
    }

    func test_toggle_off_deletes_key() {
        defaults.set(true, forKey: p + "apiKey.google.syncEnabled")
        keys.save("sk-g", for: .google)
        puller.rounds = [["apiKey.google.syncEnabled": field("false", at: 300)]]
        sync.pullOnce(credentials: creds)
        XCTAssertNil(keys.key(for: .google))
        XCTAssertEqual(defaults.object(forKey: p + "apiKey.google.syncEnabled") as? Bool, false)
    }

    func test_wipe_clears_keys_prefs_and_timestamps() {
        keys.save("sk-x", for: .anthropic)
        puller.rounds = [["routing.preferredProvider": field("openai"), "apiKey.anthropic.syncEnabled": field("true")]]
        sync.pullOnce(credentials: creds)
        defaults.set("keep", forKey: "unrelated")
        PhoneSettingsSync.wipeLocal(defaults: defaults, keys: keys)
        XCTAssertNil(keys.key(for: .anthropic))
        XCTAssertNil(PhoneRoutingPrefs(defaults: defaults).preferredProvider)
        XCTAssertNil(defaults.object(forKey: p + "settingsTs.routing.preferredProvider"))
        XCTAssertNil(defaults.object(forKey: p + "apiKey.anthropic.syncEnabled"))
        XCTAssertEqual(defaults.string(forKey: "unrelated"), "keep")
    }

    func test_wipe_during_pull_drops_its_writes() {
        defaults.set(true, forKey: p + "apiKey.anthropic.syncEnabled")
        puller.onApplyPlan = { [defaults, keys] in PhoneSettingsSync.wipeLocal(defaults: defaults!, keys: keys) }
        puller.rounds = [["apiKey.anthropic": field("sk-x"), "routing.preferredProvider": field("openai")]]
        sync.pullOnce(credentials: creds)
        XCTAssertNil(keys.key(for: .anthropic))
        XCTAssertNil(PhoneRoutingPrefs(defaults: defaults).preferredProvider)
        XCTAssertNil(defaults.object(forKey: p + "settingsTs.apiKey.anthropic"))
        XCTAssertNil(defaults.object(forKey: p + "settingsTs.routing.preferredProvider"))
    }

    func test_wipe_after_credentials_captured_drops_writes() {
        defaults.set(true, forKey: p + "apiKey.anthropic.syncEnabled")
        let epoch = PhoneSettingsSync.currentWipeEpoch
        PhoneSettingsSync.wipeLocal(defaults: defaults, keys: keys)
        defaults.set(true, forKey: p + "apiKey.anthropic.syncEnabled")
        puller.rounds = [["apiKey.anthropic": field("sk-x")]]
        sync.pullOnce(credentials: creds, epoch: epoch)
        XCTAssertNil(keys.key(for: .anthropic))
        XCTAssertNil(defaults.object(forKey: p + "settingsTs.apiKey.anthropic"))
    }
}
