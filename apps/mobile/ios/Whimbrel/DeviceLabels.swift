import Foundation

/// Names for the account's devices, so a meeting says where it was recorded. The account's
/// device list (public key to name) comes from the server; meetings carry the public key.
struct DeviceLabels: Equatable {
    var names: [String: String] = [:]
    var ownDeviceId: String?

    static func names(from records: [SyncDeviceRecord]) -> [String: String] {
        var result: [String: String] = [:]
        for record in records {
            let name = record.name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !name.isEmpty, result[record.publicKey] == nil { result[record.publicKey] = name }
        }
        return result
    }

    func label(for deviceId: String) -> String {
        if let name = names[deviceId] { return name }
        if deviceId == ownDeviceId { return "This iPhone" }
        return "Device \(deviceId.prefix(6))"
    }
}
