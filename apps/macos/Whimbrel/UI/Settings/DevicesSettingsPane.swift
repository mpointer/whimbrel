import SwiftUI

struct DevicesSettingsPane: View {
    @EnvironmentObject var appState: AppState
    private let presentationContextProvider = WindowPresentationContextProvider()
    @State private var devicePendingRevocation: SyncDeviceRecord?

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Devices")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Sync your timeline across Macs. Sign in once per device, then approve new devices from here.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            content
        }
        .onAppear { appState.syncState.startDevicesPolling() }
        .onDisappear { appState.syncState.stopDevicesPolling() }
        .alert("Revoke this device?", isPresented: Binding(
            get: { devicePendingRevocation != nil },
            set: { if !$0 { devicePendingRevocation = nil } }
        )) {
            Button("Revoke", role: .destructive) {
                if let device = devicePendingRevocation {
                    Task { await appState.syncState.revoke(device) }
                }
                devicePendingRevocation = nil
            }
            Button("Cancel", role: .cancel) {
                devicePendingRevocation = nil
            }
        } message: {
            Text("This device will lose access immediately. Every other device on this account will also need to silently reconnect. This can't be undone from here — the device would need to sign in again and be re-approved.")
        }
    }

    @ViewBuilder
    private var content: some View {
        switch appState.syncState.status {
        case .notSignedIn:
            Button("Enable Sync") {
                Task { await appState.syncState.enableSync(presentationContextProvider: presentationContextProvider) }
            }
            .buttonStyle(.whimbrelSecondary)
        case .signingIn:
            ProgressView("Signing in…").controlSize(.small)
        case .waitingForApproval, .waitingForGroupKey:
            SyncWaitingView(syncState: appState.syncState)
        case .ready:
            deviceList
        case let .error(message):
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text(message)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Button("Try Again") {
                    Task { await appState.syncState.enableSync(presentationContextProvider: presentationContextProvider) }
                }
                .buttonStyle(.whimbrelSecondary)
            }
        }
    }

    @ViewBuilder
    private var deviceList: some View {
        HStack {
            TextField("This device's name", text: Binding(
                get: { appState.syncState.deviceName },
                set: { appState.syncState.deviceName = $0 }
            ))
                .textFieldStyle(.roundedBorder)
            Button("Save") {
                Task { await appState.syncState.saveDeviceName() }
            }
            .buttonStyle(.whimbrelSecondary)
        }
        Divider().overlay(Color.whimbrelBorder)

        if let actionError = appState.syncState.actionError {
            Text(actionError)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
        if appState.syncState.devices.isEmpty {
            Text("Loading devices…")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            let codes: [String: String?] = Dictionary(
                appState.syncState.devices.map { ($0.id, CoreMemoryStore.deviceComparisonCode(publicKeyBase64: $0.publicKey)) },
                uniquingKeysWith: { first, _ in first })
            let collisions = collidingPendingDeviceIDs(appState.syncState.devices.map { ($0.id, $0.status, codes[$0.id] ?? nil) })
            ForEach(appState.syncState.devices) { device in
                HStack {
                    VStack(alignment: .leading) {
                        Text(device.name ?? device.platform ?? "Unnamed device")
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        Text(device.status.capitalized)
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                        if device.status == "pending" {
                            Text((codes[device.id] ?? nil).map { "Check this matches the code on the new device: \($0)" } ?? "Can't verify this device")
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelTextSecondary)
                            if collisions.contains(device.id) {
                                Text("Another pending device shows the same code — don't approve it")
                                    .font(.whimbrelBody(.caption))
                                    .foregroundStyle(Color.whimbrelError)
                            }
                        }
                    }
                    Spacer()
                    if device.status == "pending" {
                        Button("Approve") {
                            Task { await appState.syncState.approve(device) }
                        }
                        .buttonStyle(.whimbrelSecondary)
                        .disabled(collisions.contains(device.id))
                    } else if device.status == "approved", let currentKey = appState.syncState.currentDevicePublicKeyBase64, device.publicKey != currentKey {
                        Button("Re-send Key") {
                            Task { await appState.syncState.approve(device) }
                        }
                        .buttonStyle(.whimbrelSecondary)
                    }
                    if device.status != "revoked" {
                        Button("Revoke") {
                            devicePendingRevocation = device
                        }
                        .buttonStyle(.whimbrelSecondary)
                        .tint(Color.whimbrelError)
                    }
                }
                Divider().overlay(Color.whimbrelBorder)
            }
        }

        if let lastSyncError = appState.settingsSyncCoordinator.lastSyncError {
            Text(lastSyncError)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else if let lastSyncedAt = appState.settingsSyncCoordinator.lastSyncedAt {
            Text("Settings last synced \(lastSyncedAt.formatted(.relative(presentation: .named)))")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }

        if let lastSyncError = appState.contentSyncCoordinator.lastSyncError {
            Text(lastSyncError)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else if let lastSyncedAt = appState.contentSyncCoordinator.lastSyncedAt {
            Text("Content last synced \(lastSyncedAt.formatted(.relative(presentation: .named)))")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }
}
