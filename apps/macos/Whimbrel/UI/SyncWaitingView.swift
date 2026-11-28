import AuthenticationServices
import SwiftUI

/// `ASWebAuthenticationPresentationContextProviding` is an `@objc`
/// protocol, so only a class (never a SwiftUI `View` struct) can conform
/// to it — this is that class, shared by every view in this app that
/// needs to anchor an `ASWebAuthenticationSession` (currently
/// `SyncWaitingView` and `DevicesSettingsPane`).
final class WindowPresentationContextProvider: NSObject, ASWebAuthenticationPresentationContextProviding {
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        NSApplication.shared.windows.first { $0.isKeyWindow } ?? NSApplication.shared.windows.first ?? NSWindow()
    }
}
