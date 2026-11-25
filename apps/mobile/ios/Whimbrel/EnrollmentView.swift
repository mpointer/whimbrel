import SwiftUI
import UIKit

/// Sign in with Google, then "Waiting for approval" with the comparison code, then the
/// synced meeting count. Also drives the models from the scene phase: sync and the
/// approval poll run only in the foreground.
struct EnrollmentView: View {
    @ObservedObject var model: EnrollmentModel
    @ObservedObject var sync: SyncModel
    let coordinator: ReminderCoordinator
    let router: ReminderRouter
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            if model.status == .approved {
                // Not `.id(themes.revision)`: that would reset the tabs, open meeting and models
                // on a theme change. Each screen re-renders its own content (`themed()`).
                RootView(sync: sync, enrollment: model, router: router)
            } else {
                // Scrolls, so the code and the buttons stay reachable at the largest text sizes;
                // centered when it fits.
                GeometryReader { proxy in
                    ScrollView {
                        VStack(spacing: WTokens.Space.lg) {
                            Text("Whimbrel")
                                .wType(WTokens.TypeScale.pageTitle)
                                .foregroundStyle(Color.theme(.textPrimary))
                                .accessibilityAddTraits(.isHeader)
                            content
                        }
                        .padding(WTokens.Space.xl)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 480)
                        .frame(maxWidth: .infinity, minHeight: proxy.size.height)
                    }
                }
                .environment(\.wStrings, .app)
                .id(themes.revision)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.theme(.backgroundPage))
        .task { await model.resume() }
        .task(id: sync.readRevision) {
            // Each sync: a fresh set of pending reminders, and the permission ask if it is still owed.
            await coordinator.reschedule()
            await coordinator.askPermissionIfNeeded()
        }
        .onAppear(perform: updateSync)
        .onChange(of: model.status) { updateSync() }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .background:
                model.enterBackground()
                // Before updateSync() stops the loop: let a round in flight finish (#572).
                sync.holdForRoundInFlight(begin: { expired in
                    let id = UIApplication.shared.beginBackgroundTask(withName: "Finish sync round", expirationHandler: expired)
                    return id == .invalid ? nil : id
                }, end: { UIApplication.shared.endBackgroundTask($0) })
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
            secondary(String(localized: "Sign in with the Google account you use for Whimbrel on your Mac."))
            signInButton
        case .signingIn:
            ProgressView()
            secondary(String(localized: "Signing in…"))
        case let .pending(code):
            codeBlock(title: String(localized: "Waiting for approval"), code: code,
                      detail: model.swapRequested ? String(localized: "Approve this phone on your computer.") : String(localized: "On your Mac, open Whimbrel Settings → Devices and approve this iPhone. Check this matches the code on your Mac before approving."))
        case let .waitingForGroupKey(code):
            codeBlock(title: String(localized: "Approve this iPhone again"), code: code,
                      detail: String(localized: "This iPhone was approved, but the encryption key didn't arrive in time. Approve it again from Whimbrel Settings → Devices on your Mac. Check this matches the code on your Mac before approving."))
        case .needsDesktopFirst:
            primary(String(localized: "Set up Whimbrel on your Mac first, then come back and sign in here."))
            Button("Try again") { Task { await model.signIn() } }
                .buttonStyle(.w(.primary, fill: true))
        case let .deviceLimit(_, holders):
            DeviceLimitView(model: model, holders: holders)
        case .approved:
            EmptyView() // shown full screen by `body` as `RootView`
        case .removed:
            primary(String(localized: "This iPhone was removed from your Whimbrel account."))
            signInButton
        case let .error(message):
            primary(message)
            signInButton
        }
    }

    private var signInButton: some View {
        Button("Sign in with Google") { Task { await model.signIn() } }
            .buttonStyle(.w(.primary, fill: true))
    }

    private func primary(_ text: String) -> some View {
        Text(text).wType(WTokens.TypeScale.body).foregroundStyle(Color.theme(.textPrimary))
    }

    private func secondary(_ text: String) -> some View {
        Text(text).wType(WTokens.TypeScale.body).foregroundStyle(Color.theme(.textSecondary))
    }

    private func codeBlock(title: String, code: String, detail: String) -> some View {
        VStack(spacing: WTokens.Space.lg) {
            Text(title).wType(WTokens.TypeScale.title).foregroundStyle(Color.theme(.textPrimary))
                .accessibilityAddTraits(.isHeader)
            // The code to compare, set apart on its own card so it is the one thing to look at.
            Text(code)
                .font(.system(size: 30, weight: .semibold, design: .monospaced))
                .minimumScaleFactor(0.5)
                .lineLimit(1)
                .foregroundStyle(Color.theme(.textPrimary))
                .padding(.horizontal, WTokens.Space.md)
                .padding(.vertical, WTokens.Space.lg)
                .frame(maxWidth: .infinity)
                .wCard(tint: Color.theme(.highlight))
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
    private let pendingA = CoreMemoryStore.generateSyncDeviceKeypair()!
    private let pendingB = CoreMemoryStore.generateSyncDeviceKeypair()!
    func signIn() async throws {}
    func idToken() async throws -> String { "demo" }
    func authenticate(idToken: String, devicePublicKey: String) async throws -> GoogleAuthResult {
        let wrapped = CoreMemoryStore.wrapSyncGroupKey(senderSecretKeyBase64: mac.secretKeyBase64, recipientPublicKeyBase64: devicePublicKey, groupKeyBase64: groupKey)
        return .approved(tursoToken: "demo", tursoDatabaseUrl: "demo", wrappedGroupKey: wrapped)
    }
    func listDevices(idToken: String, devicePublicKey: String) async throws -> [SyncDeviceRecord] {
        var devices = [SyncDeviceRecord(id: "mac", accountId: "demo", publicKey: mac.publicKeyBase64, name: "Mac", platform: "macos", status: "approved", enrolledAtUnixMs: 0, revokedAtUnixMs: nil)]
        // `-WhimbrelDemoDevices`: a laptop waiting for approval, and two more that show one code
        // (the same made-up key), so the "don't approve" warning can be seen.
        if ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoDevices") {
            devices.append(SyncDeviceRecord(id: "laptop", accountId: "demo", publicKey: pendingA.publicKeyBase64, name: "Work laptop", platform: "windows", status: "pending", enrolledAtUnixMs: 0, revokedAtUnixMs: nil))
            devices.append(SyncDeviceRecord(id: "twin1", accountId: "demo", publicKey: pendingB.publicKeyBase64, name: "Mac mini", platform: "macos", status: "pending", enrolledAtUnixMs: 0, revokedAtUnixMs: nil))
            devices.append(SyncDeviceRecord(id: "twin2", accountId: "demo", publicKey: pendingB.publicKeyBase64, name: nil, platform: "macos", status: "pending", enrolledAtUnixMs: 0, revokedAtUnixMs: nil))
        }
        return devices
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

    func moreToPull() -> Bool { false }
    func olderHistoryLoading() -> Bool { false }

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
        // `-WhimbrelDemoDeviceLimit`: the free plan's phone slot is taken by another phone.
        if ProcessInfo.processInfo.arguments.contains("-WhimbrelDemoDeviceLimit") {
            return .deviceLimit(kind: "phone", holders: [LimitHolder(id: "other-phone", name: "Pixel 9", platform: "android")])
        }
        return noDesktop ? .approved(tursoToken: "demo", tursoDatabaseUrl: "demo", wrappedGroupKey: nil) : .pending
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
