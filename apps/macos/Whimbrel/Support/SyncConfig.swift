import Foundation

enum SyncConfigError: Error {
    case missingConfig(String)
}

/// Reads sync-related secrets/config from Info.plist, populated at build
/// time from a gitignored Secrets.xcconfig (never committed) — see
/// apps/macos/Secrets.xcconfig.example and the "Sync configuration"
/// section of apps/macos/README.md.
enum SyncConfig {
    static func googleOAuthClientId() throws -> String {
        try infoPlistString("GoogleOAuthClientID")
    }

    static func googleOAuthClientSecret() throws -> String {
        try infoPlistString("GoogleOAuthClientSecret")
    }

    static func microsoftOAuthClientId() throws -> String {
        try infoPlistString("MicrosoftOAuthClientID")
    }

    static func workerBaseURL() throws -> URL {
        let raw = try infoPlistString("SyncWorkerBaseURL")
        guard let url = URL(string: raw) else {
            throw SyncConfigError.missingConfig("SyncWorkerBaseURL is not a valid URL: \(raw)")
        }
        return url
    }

    private static func infoPlistString(_ key: String) throws -> String {
        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String, !value.isEmpty else {
            throw SyncConfigError.missingConfig(key)
        }
        return value
    }
}
