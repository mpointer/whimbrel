import Foundation

/// Whether the Devices pane still shows the mobile beta card or collapses it to one line.
enum PhoneCardState: Equatable {
    case beta
    case connected

    /// `.connected` once an approved iOS or Android device is enrolled on the account.
    static func state(devices: [SyncDeviceRecord]) -> PhoneCardState {
        let paired = devices.contains { $0.status == "approved" && ($0.platform == "ios" || $0.platform == "android") }
        return paired ? .connected : .beta
    }
}
