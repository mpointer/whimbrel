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

    /// True while sync is ready but the first device list has not arrived: the beta card would show for a poll tick and then
    /// collapse to "Phone: connected". After `waitedOut` (the list never came, for instance offline) the beta card shows.
    static func isLoading(ready: Bool, loaded: Bool, waitedOut: Bool) -> Bool {
        ready && !loaded && !waitedOut
    }
}
