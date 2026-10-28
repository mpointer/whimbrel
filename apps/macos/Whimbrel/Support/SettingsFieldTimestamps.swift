import Foundation

/// Per-field "last locally modified" timestamps backing settings sync's
/// field-level last-writer-wins merge
/// (docs/superpowers/specs/2026-07-30-sync-data-layer-design.md). Each
/// field key gets its own UserDefaults entry, parallel to (but separate
/// from) the field's own value entry — e.g. `routing.localOnlyMode`'s
/// value lives under `com.mpointer.kestrel.routingLocalOnlyMode` (RoutingSettings'
/// own key), its timestamp lives under
/// `com.mpointer.kestrel.syncTimestamp.routing.localOnlyMode` here. `fieldKey` is
/// always the same string used as the Turso `settings_field.field_key`
/// value (e.g. "routing.localOnlyMode", "apiKey.anthropic") — see
/// `SettingsSyncCoordinator` for the full field list.
enum SettingsFieldTimestamps {
    private static func defaultsKey(for fieldKey: String) -> String {
        "com.mpointer.kestrel.syncTimestamp.\(fieldKey)"
    }

    /// Sets `fieldKey`'s timestamp to the current time — call this from a
    /// settings store's own `didSet` whenever the user (or any code path)
    /// changes that field locally.
    static func touch(_ fieldKey: String) {
        setTimestamp(Int64(Date().timeIntervalSince1970 * 1000), for: fieldKey)
    }

    /// `0` means "never modified locally" — always older than any real
    /// remote timestamp, so an unset field always loses to whatever's on
    /// the server the first time it syncs.
    static func timestamp(for fieldKey: String) -> Int64 {
        let stored = UserDefaults.standard.object(forKey: defaultsKey(for: fieldKey)) as? Int64
        return stored ?? 0
    }

    /// Sets an explicit timestamp — used when applying a value PULLED
    /// from sync, so the local timestamp matches the remote one instead
    /// of getting bumped to "now" by the store's own `didSet` (which
    /// would make the very next sync push it right back as if it were a
    /// fresh local change).
    static func setTimestamp(_ ms: Int64, for fieldKey: String) {
        UserDefaults.standard.set(ms, forKey: defaultsKey(for: fieldKey))
    }
}
