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

/// Shown inside the Devices settings pane while this device is waiting
/// either for another device to approve it, or (the handoff-TTL-expiry
/// edge case) for another device to re-approve it because the wrapped
/// group key wasn't delivered in time.
struct SyncWaitingView: View {
    @ObservedObject var syncState: SyncState
    private let presentationContextProvider = WindowPresentationContextProvider()

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            ProgressView()
                .controlSize(.small)
            Text(title)
                .font(.whimbrelBody(.headline))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text(message)
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextSecondary)
            if syncState.status == .waitingForGroupKey {
                Button("Check Again") {
                    Task { await syncState.enableSync(presentationContextProvider: presentationContextProvider) }
                }
                .buttonStyle(.whimbrelSecondary)
            }
        }
        .padding(Spacing.lg)
    }

    private var title: String {
        switch syncState.status {
        case .waitingForGroupKey: "Waiting for a device to finish setting this one up"
        default: "Waiting for approval on another device"
        }
    }

    private var message: String {
        switch syncState.status {
        case .waitingForGroupKey:
            "This device was approved, but the handoff to send it the sync key expired before it could be delivered. Open Whimbrel on an already-signed-in device, go to Settings → Devices, and approve this device again."
        default:
            "Open Whimbrel on a device that's already signed in, go to Settings → Devices, and approve this one — it'll be picked up automatically within a few seconds."
        }
    }
}
