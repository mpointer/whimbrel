import Foundation

/// One-time migration helper for a UserDefaults-backed model-path setting: prefers
/// `newKey`; if unset, falls back to `legacyKey`'s value and immediately re-saves it
/// under `newKey`, so the fallback only ever fires once per install; if neither key
/// has ever been set, returns `defaultPath`. Extracted as a standalone, injectable-
/// UserDefaults helper (rather than inlined three times across two enums) so the
/// migration logic itself is directly unit-testable.
enum ModelPathDefaultsMigration {
    static func resolve(newKey: String, legacyKey: String, defaultPath: URL, defaults: UserDefaults = .standard) -> URL {
        if let stored = defaults.string(forKey: newKey) {
            return URL(fileURLWithPath: stored)
        }
        if let legacy = defaults.string(forKey: legacyKey) {
            defaults.set(legacy, forKey: newKey)
            return URL(fileURLWithPath: legacy)
        }
        return defaultPath
    }
}

/// Copies every `littlebird.*` key to `com.mpointer.kestrel.*` unless the new key is already
/// set (#180 B3). A prefix sweep rather than a key list, because some keys are built at
/// runtime (`syncTimestamp.<field>`, `promptInstructions.<surface>`). Old keys stay so a
/// downgrade still finds its settings.
enum LegacyDefaultsMigration {
    static let legacyPrefix = "littlebird."
    static let newPrefix = "com.mpointer.kestrel."

    static func run(defaults: UserDefaults = .standard) {
        for (key, value) in defaults.dictionaryRepresentation() where key.hasPrefix(legacyPrefix) {
            let newKey = newPrefix + key.dropFirst(legacyPrefix.count)
            if defaults.object(forKey: newKey) == nil {
                defaults.set(value, forKey: newKey)
            }
        }
    }
}
