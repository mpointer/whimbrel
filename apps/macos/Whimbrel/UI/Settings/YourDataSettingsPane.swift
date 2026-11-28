import SwiftUI
import AppKit

/// UserDefaults keys for the Obsidian vault the topic scheduler keeps up to date.
enum VaultSettings {
    static let enabledKey = "vault.enabled"
    static let pathKey = "vault.path"
    static let lastUpdatedKey = "vault.lastUpdated"
    static let failedKey = "vault.failed"
}
