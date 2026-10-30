import SwiftUI

/// Sign in with Google, then "Waiting for approval" with the comparison code, then the
/// synced meeting count. Also drives the models from the scene phase: sync and the
/// approval poll run only in the foreground.
struct EnrollmentView: View {
    @ObservedObject var model: EnrollmentModel
    @ObservedObject var sync: SyncModel
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            if model.status == .approved {
                // Not `.id(themes.revision)`: that would reset the tabs, open meeting and models
                // on a theme change. Each screen re-renders its own content (`themed()`).
                RootView(sync: sync, enrollment: model)
            } else {
                VStack(spacing: 20) {
                    Text("Whimbrel").font(.largeTitle.bold()).foregroundStyle(Color.theme(.textPrimary))
                    content
                }
                .padding(24)
                .multilineTextAlignment(.center)
                .id(themes.revision)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.theme(.backgroundPage))
        .task { await model.resume() }
        .onAppear(perform: updateSync)
        .onChange(of: model.status) { updateSync() }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .background: model.enterBackground()
            case .active: Task { await model.enterForeground() }
            default: break
            }
            updateSync()
        }
    }

    private func updateSync() {
        if model.status == .approved, scenePhase == .active {
            sync.start()
        } else if model.status != .approved || scenePhase == .background {
            sync.stop()
        }
    }

    @ViewBuilder private var content: some View {
        switch model.status {
        case let .notConfigured(message):
            signInButton.disabled(true)
            secondary(message)
        case .signedOut:
            secondary("Sign in with the Google account you use for Whimbrel on your Mac.")
            signInButton
        case .signingIn:
            ProgressView()
            secondary("Signing in…")
        case let .pending(code):
            codeBlock(title: "Waiting for approval", code: code,
                      detail: model.swapRequested ? "Approve this phone on your computer." : "On your Mac, open Whimbrel Settings → Devices and approve this iPhone. Check this matches the code on your Mac before approving.")
        case let .waitingForGroupKey(code):
            codeBlock(title: "Approve this iPhone again", code: code,
                      detail: "This iPhone was approved, but the encryption key didn't arrive in time. Approve it again from Whimbrel Settings → Devices on your Mac. Check this matches the code on your Mac before approving.")
        case .needsDesktopFirst:
            Text("Set up Whimbrel on your Mac first, then come back and sign in here.").foregroundStyle(Color.theme(.textPrimary))
            Button("Try again") { Task { await model.signIn() } }
                .buttonStyle(.borderedProminent)
                .tint(Color.theme(.accentPrimary))
        case let .deviceLimit(_, holders):
            DeviceLimitView(model: model, holders: holders)
        case .approved:
            EmptyView() // shown full screen by `body` as `RootView`
        case .removed:
            Text("This iPhone was removed from your Whimbrel account.").foregroundStyle(Color.theme(.textPrimary))
            signInButton
        case let .error(message):
            Text(message).foregroundStyle(Color.theme(.textPrimary))
            signInButton
        }
    }

    private var signInButton: some View {
        Button("Sign in with Google") { Task { await model.signIn() } }
            .buttonStyle(.borderedProminent)
            .tint(Color.theme(.accentPrimary))
    }

    private func secondary(_ text: String) -> some View {
        Text(text).foregroundStyle(Color.theme(.textSecondary))
    }

    private func codeBlock(title: String, code: String, detail: String) -> some View {
        VStack(spacing: 16) {
            Text(title).font(.title2.bold()).foregroundStyle(Color.theme(.textPrimary))
            Text(code)
                .font(.system(size: 34, weight: .semibold, design: .monospaced))
                .minimumScaleFactor(0.5)
                .lineLimit(1)
                .foregroundStyle(Color.theme(.accentPrimary))
                .accessibilityLabel("Comparison code \(code)")
            secondary(detail)
            ProgressView()
        }
    }
}

#if DEBUG
/// `-WhimbrelDemoSynced` (debug builds only): an approved phone without Google or a server.
/// The backend approves at once with a group key wrapped by a made-up Mac; `DemoSyncStore`
/// "pulls" by writing meetings (two copies of one call among them) into the real local
/// store, so the count comes from the real grouping call.
final class DemoSyncedBackend: EnrollmentBackend {
    private let mac = CoreMemoryStore.generateSyncDeviceKeypair()!
    private let groupKey = CoreMemoryStore.generateSyncGroupKey()!
    func signIn() async throws {}
    func idToken() async throws -> String { "demo" }
    func authenticate(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        let wrapped = CoreMemoryStore.wrapSyncGroupKey(senderSecretKeyBase64: mac.secretKeyBase64, recipientPublicKeyBase64: devicePublicKey, groupKeyBase64: groupKey)
        return .approved(tursoToken: "demo", tursoDatabaseUrl: "demo", wrappedGroupKey: wrapped)
    }
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] {
        [SyncDeviceRecord(id: "mac", accountId: "demo", publicKey: mac.publicKeyBase64, name: "Mac", platform: "macos", status: "approved", enrolledAtUnixMs: 0, revokedAtUnixMs: nil)]
    }
    func revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws {}
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws {}
    func approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws {}
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws {}
    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws {}
    func signOut() {}
}

final class DemoSyncStore: MeetingSyncStore {
    private let store: CoreMemoryStore
    init(store: CoreMemoryStore) { self.store = store }

    func pullOnly(_ credentials: SyncCredentials, captureWindowDays: Int32) -> Bool {
        guard store.groupedMeetingCount() == 0 else { return true }
        let day: Int64 = 86_400_000
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let titles = ["Standup", "Design review", "Customer call", "Planning", "Retro", "1:1"]
        for (i, title) in titles.enumerated() {
            store.insertCompletedMeetingForDemo(deviceId: "demo-mac-a", title: title, startUnixMs: now - Int64(i + 1) * day, minutes: 30)
        }
        // The 1:1 also recorded on a second Mac: one meeting, not two.
        store.insertCompletedMeetingForDemo(deviceId: "demo-mac-b", title: "1:1", startUnixMs: now - 6 * day + 60_000, minutes: 29)
        return true
    }

    func groupedMeetingCount() -> Int? { store.groupedMeetingCount() }
    func lastSyncError() -> String? { nil }
}

/// `-WhimbrelDemoEnrollment` launch argument (debug builds only): a fake backend that
/// signs in instantly and stays pending, to see the pending screen without Google.
/// `-WhimbrelDemoNeedsDesktop` instead answers "approved" on an account with no desktop.
final class DemoEnrollmentBackend: EnrollmentBackend {
    private let noDesktop = ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoNeedsDesktop")
    func signIn() async throws {}
    func idToken() async throws -> String { "demo" }
    func authenticate(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        noDesktop ? .approved(tursoToken: "demo", tursoDatabaseUrl: "demo", wrappedGroupKey: nil) : .pending
    }
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] { [] }
    func revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) async throws {}
    func setDeviceName(idToken: String, devicePublicKey: String, name: String) async throws {}
    func approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) async throws {}
    func swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) async throws {}
    func joinWaitlist(idToken: String, devicePublicKey: String, wants: [String]) async throws {}
    func signOut() {}
}
#endif
