import Foundation
import Security

/// The only door to the Keychain's `SecItem` API. In the app it forwards to the real
/// Keychain. In a unit-test host it uses an in-memory store instead: every test run launches
/// a freshly built, differently signed copy of the app, and the real Keychain would ask for
/// the user's password on each run (and "Always Allow" cannot stick across rebuilds).
/// `KeychainAccessTests` fails if any other source file calls `SecItem*` directly.
enum KeychainAccess {
    /// True when the process is an XCTest host.
    static let isTestHost: Bool = {
        let env = ProcessInfo.processInfo.environment
        return env["XCTestConfigurationFilePath"] != nil || env["XCTestBundlePath"] != nil
    }()

    /// Test hosts, plus (iOS debug builds only) the `-WhimbrelDemoSynced` demo, whose made-up
    /// group key must never reach the real Keychain.
    private static let useMemory: Bool = {
        #if DEBUG && os(iOS)
        if ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoSynced") { return true }
        #endif
        return isTestHost
    }()

    private static let memory = InMemoryKeychain()

    @discardableResult
    static func add(_ attributes: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus {
        useMemory ? memory.add(attributes, result) : SecItemAdd(attributes, result)
    }

    static func copyMatching(_ query: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus {
        useMemory ? memory.copyMatching(query, result) : SecItemCopyMatching(query, result)
    }

    @discardableResult
    static func update(_ query: CFDictionary, _ attributes: CFDictionary) -> OSStatus {
        useMemory ? memory.update(query, attributes) : SecItemUpdate(query, attributes)
    }

    @discardableResult
    static func delete(_ query: CFDictionary) -> OSStatus {
        useMemory ? memory.delete(query) : SecItemDelete(query)
    }
}

/// A process-local stand-in for the generic-password items Whimbrel uses, keyed by
/// (service, account). Supports exactly the query shapes the app issues: class + service +
/// account, optionally returning the data.
final class InMemoryKeychain {
    private let lock = NSLock()
    private var items: [String: Data] = [:]

    private func key(_ dictionary: CFDictionary) -> String? {
        let d = dictionary as NSDictionary
        guard let service = d[kSecAttrService as String] as? String,
              let account = d[kSecAttrAccount as String] as? String else { return nil }
        return service + "\u{1}" + account
    }

    @discardableResult
    func add(_ attributes: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus {
        guard let k = key(attributes) else { return errSecParam }
        let data = (attributes as NSDictionary)[kSecValueData as String] as? Data ?? Data()
        lock.lock()
        defer { lock.unlock() }
        if items[k] != nil { return errSecDuplicateItem }
        items[k] = data
        return errSecSuccess
    }

    func copyMatching(_ query: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>?) -> OSStatus {
        guard let k = key(query) else { return errSecParam }
        lock.lock()
        let data = items[k]
        lock.unlock()
        guard let data else { return errSecItemNotFound }
        if let result, ((query as NSDictionary)[kSecReturnData as String] as? Bool) == true {
            result.pointee = data as NSData
        }
        return errSecSuccess
    }

    @discardableResult
    func update(_ query: CFDictionary, _ attributes: CFDictionary) -> OSStatus {
        guard let k = key(query) else { return errSecParam }
        lock.lock()
        defer { lock.unlock() }
        guard items[k] != nil else { return errSecItemNotFound }
        if let data = (attributes as NSDictionary)[kSecValueData as String] as? Data { items[k] = data }
        return errSecSuccess
    }

    @discardableResult
    func delete(_ query: CFDictionary) -> OSStatus {
        guard let k = key(query) else { return errSecParam }
        lock.lock()
        defer { lock.unlock() }
        return items.removeValue(forKey: k) == nil ? errSecItemNotFound : errSecSuccess
    }
}
