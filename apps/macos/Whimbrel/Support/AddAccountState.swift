import Foundation

/// Transient state for one connector's "Add Account" action -- shared by
/// `CalendarConnectorState` and `GmailConnectorState`, both of which model this
/// separately from their settled `accounts` list (an in-flight consent attempt isn't
/// "about" any particular already-connected account).
enum AddAccountState: Equatable {
    case idle
    case connecting
    case error(String)
}
