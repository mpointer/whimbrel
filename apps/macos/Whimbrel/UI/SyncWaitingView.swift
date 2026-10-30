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
    @State private var showingFreshStart = false
    @State private var startingFresh = false

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
            if syncState.status == .waitingForApproval, let holder = syncState.swapHolderName {
                freshStart(holder: holder)
            }
        }
        .padding(Spacing.lg)
    }

    /// Offered only on a pending swap, once the user says the old computer isn't available.
    @ViewBuilder
    private func freshStart(holder: String) -> some View {
        if showingFreshStart {
            Text(SyncState.freshStartWarning(holderName: holder))
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextSecondary)
            HStack {
                Button("Start fresh", role: .destructive) {
                    Task {
                        startingFresh = true
                        await syncState.startFresh()
                        startingFresh = false
                    }
                }
                .buttonStyle(.whimbrelSecondary)
                .tint(Color.whimbrelError)
                .disabled(startingFresh)
                Button("Cancel") { showingFreshStart = false }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(startingFresh)
            }
        } else {
            Button("\(SyncState.holderAtSentenceStart(holder)) isn't available?") { showingFreshStart = true }
                .buttonStyle(.link)
        }
        if let actionError = syncState.actionError {
            Text(actionError)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelError)
        }
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
            if let holder = syncState.swapHolderName {
                SyncState.swapWaitingMessage(holderName: holder)
            } else {
                "Open Whimbrel on a device that's already signed in, go to Settings → Devices, and approve this one — it'll be picked up automatically within a few seconds."
            }
        }
    }
}
