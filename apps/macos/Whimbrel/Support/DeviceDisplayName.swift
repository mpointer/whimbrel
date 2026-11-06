import Foundation

/// The label for a synced device: its own name, or "Android phone" for an unnamed Android device.
enum DeviceDisplayName {
    static func name(_ name: String?, platform: String?) -> String? {
        if let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty {
            return trimmed
        }
        return platform == "android" ? String(localized: "Android phone") : nil
    }
}
