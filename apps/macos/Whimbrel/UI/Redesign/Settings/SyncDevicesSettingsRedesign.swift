import AppKit
import Combine
import CoreImage.CIFilterBuiltins
import SwiftUI
import WhimbrelUI

/// Sync & Devices in the redesigned Settings window. This file feeds the package's `WSyncDevicesSettingsView` from the app's
/// state and calls the functions the old Sync & Devices pane called (`SyncState.enableSync`, `approve`, `revoke`, `swap`,
/// `joinWaitlist`, `saveDeviceName`, `deleteSyncAccount`, `registerRunner`, the content sync round and the pending-clients
/// bridge). The classic pane is not changed. Billing, auth and sync are not touched here: this only shows what the server
/// and `SyncState` say. The purchase choice comes from `PlanSettingsAdapter`, the same reading of the server's plan answer
/// the Plan & billing section uses, so no plan value is read in this file.

// MARK: Pasteboard

enum SettingsPasteboard {
    /// The convention clipboard managers follow to skip a secret (nspasteboard.org): an item that also carries this type is not
    /// recorded in their history.
    static let concealedType = NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")

    /// Writes `value` to the pasteboard (the general one unless a test passes its own) as a concealed item. True only when the
    /// write succeeded: "Copied" is shown on that. The clipboard is not cleared afterwards.
    static func copy(_ value: String, to board: NSPasteboard = .general) -> Bool {
        let item = NSPasteboardItem()
        guard item.setString(value, forType: .string), item.setString("", forType: concealedType) else { return false }
        board.clearContents()
        return board.writeObjects([item])
    }
}

// MARK: Tell a friend

/// The message the menu bar row and the Settings card copy: a short line about Whimbrel and a plain link, no code and no reward.
/// It is plain text, not concealed: unlike a token, it is meant to be pasted anywhere and may sit in a clipboard history.
enum TellAFriend {
    /// The localized message. The template's one `%@` is where the link goes (`WTellAFriend.message`).
    static func message() -> String {
        WTellAFriend.message(template: String(
            localized: "I use Whimbrel for my meeting notes. It records without a bot, and my recordings stay on my computer. Free for Mac and Windows: %@",
            comment: "The message copied by Tell a friend, to paste into a chat or an email. The placeholder is the link to the Whimbrel website and must stay at the end of the message."))
    }

    /// Puts the message on the pasteboard (the general one unless a test passes its own). True only when the write succeeded.
    static func copy(to board: NSPasteboard = .general) -> Bool {
        board.clearContents()
        return board.setString(message(), forType: .string)
    }

    static func labels() -> WTellAFriendLabels {
        var l = WTellAFriendLabels()
        l.heading = String(localized: "Tell a friend", comment: "Settings: heading of the card that copies a message about Whimbrel. Also the name of the row in the menu bar menu.")
        l.blurb = String(localized: "Copy a short message with a link to Whimbrel, then paste it wherever you like. Nothing is sent from here.", comment: "Settings: under the Tell a friend heading.")
        l.copyButton = String(localized: "Copy message", comment: "Settings: button on the Tell a friend card.")
        l.copied = String(localized: "Copied")
        l.failed = String(localized: "Could not copy. Try again.", comment: "Settings: shown when the clipboard refused the Tell a friend message.")
        return l
    }
}

// MARK: Adapter (pure, tested)

enum SyncDevicesAdapter {
    static let betaURL = "https://whimbrelapp.com/mobile-beta/"
    static let rateLimitedMessage = String(localized: "Too many sign-in attempts. Try again in a few minutes.")

    static func phase(_ status: SyncStatus) -> WSyncPhase {
        switch status {
        case .notSignedIn: return .signedOut
        case .signingIn: return .signingIn
        case .waitingForApproval, .waitingForGroupKey: return .waiting
        case .ready: return .ready
        case .deviceLimit: return .deviceLimit
        case .error(let message): return .failed(message)
        case .rateLimited: return .failed(rateLimitedMessage)
        }
    }

    /// One row per device, with the same rules the classic list applies. `code` is the comparison code for a public key
    /// (core's `deviceComparisonCode`); `runnerLastRanMs` is the cloud runner's last run.
    static func rows(
        devices: [SyncDeviceRecord], thisDeviceKey: String?, runnerLastRanMs: Int64?, code: (String) -> String?, lastRan: (Int64) -> String
    ) -> [WDeviceRow] {
        let codes: [String: String?] = Dictionary(devices.map { ($0.id, code($0.publicKey)) }, uniquingKeysWith: { first, _ in first })
        let collisions = collidingPendingDeviceIDs(devices.map { ($0.id, $0.status, codes[$0.id] ?? nil) })
        return devices.map { device in
            let isRunner = CloudRoutineCopy.isRunner(device)
            let name = isRunner
                ? CloudRoutineCopy.runnerName
                : DeviceDisplayName.name(device.name, platform: device.platform) ?? device.platform ?? String(localized: "Unnamed device")
            return WDeviceRow.make(
                id: device.id, name: name, status: device.status, statusText: device.status == "resetting" ? "Resetting…" : device.status.capitalized,
                isRunner: isRunner, keyDiffersFromThisDevice: thisDeviceKey.map { device.publicKey != $0 } ?? false, code: codes[device.id] ?? nil,
                collides: collisions.contains(device.id), canRemoveRunner: CloudRoutineCopy.canRemoveRunner(device),
                lastRan: isRunner && device.status == "approved" ? runnerLastRanMs.map(lastRan) : nil)
        }
    }

    static func waitlist() -> [WWaitlistOption] {
        SyncState.waitlistOptions.map { WWaitlistOption(id: $0.want, label: $0.label) }
    }

    /// The runner's provision values for the shown-once sheet. Only the bundle is secret (it carries the sync key).
    static func provisionValues(_ provision: RunnerProvision, workerURL: String) -> [WShownValue] {
        [
            WShownValue(id: "provision", name: "RUNNER_PROVISION", value: provision.provisionBundle, isSecret: true),
            WShownValue(id: "approver", name: "RUNNER_APPROVER_PUBLIC_KEY", value: provision.approverPublicKey, isSecret: false),
            WShownValue(id: "worker", name: "RUNNER_WORKER_URL", value: workerURL, isSecret: false),
        ]
    }

    static func qrImage() -> Image? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(betaURL.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage, let cg = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return Image(nsImage: NSImage(cgImage: cg, size: output.extent.size))
    }

    /// Every sentence the redesigned section writes of its own, for the copy tests.
    static var ownStrings: [String] {
        let l = labels()
        return [
            l.intro, l.phoneBlurb, l.collisionTitle, l.collisionText, l.revokeTitle, l.revokeMessage, l.removeRunnerMessage, l.deleteAccountTitle,
            l.runnerNotShownAgain, l.runnerCloseQuestion, l.provisionInstructions, l.runnerPublicKeyHelp, l.removeDeviceMessage, l.waitlistPrompt,
            l.waitlistJoined, l.upgradePlus, l.upgradeCloud, l.planReadFailed,
        ] + TellAFriend.labels().all
    }

    static func labels() -> WSyncLabels {
        var l = WSyncLabels()
        l.heading = String(localized: "Devices")
        l.intro = String(localized: "Sync your timeline across Macs. Sign in once per device, then approve new devices from here.")
        l.statusLabel = String(localized: "Sync", comment: "Label of the Sync heading and of the Sync rows in Settings.")
        l.stateOff = String(localized: "Off")
        l.stateSigningIn = String(localized: "Signing in", comment: "Settings: the state of sync while you are signing in.")
        l.stateWaiting = String(localized: "Waiting for approval", comment: "Settings: the state of sync while another device has to approve this one.")
        l.stateOn = String(localized: "On")
        l.stateLimit = String(localized: "Device limit reached", comment: "Settings: the state of sync when the plan's device limit is reached.")
        l.stateFailed = String(localized: "Sign-in failed", comment: "Settings: the state of sync after sign-in failed.")
        l.signingIn = String(localized: "Signing in…")
        l.enableSync = String(localized: "Enable Sync")
        l.tryAgain = String(localized: "Try Again")
        l.phoneHeading = String(localized: "Whimbrel on your phone")
        l.phoneBlurb = String(localized: "Read your meeting notes, decisions and action items on iPhone or Android. The apps are in beta.")
        l.joinBeta = String(localized: "Join the beta")
        l.scanWithPhone = String(localized: "Scan with your phone")
        l.qrLabel = String(localized: "QR code for the phone beta page")
        l.phoneConnected = String(localized: "Phone: connected")
        l.deviceName = String(localized: "This device's name")
        l.save = String(localized: "Save")
        l.loadingDevices = String(localized: "Loading devices…")
        l.approve = String(localized: "Approve", comment: "Button in Settings that approves a new device so it can sync.")
        l.resendKey = String(localized: "Re-send Key")
        l.revoke = String(localized: "Revoke")
        l.remove = String(localized: "Remove")
        l.cancel = String(localized: "Cancel")
        l.codeLine = { String(localized: "Check this matches the code on the new device: \($0)") }
        l.codeUnverifiable = String(localized: "Can't verify this device")
        l.collisionTitle = String(localized: "Another pending device shows the same code", comment: "Settings: warning on a device waiting for approval when another waiting device shows the same comparison code.")
        l.collisionText = String(localized: "Do not approve either one. Revoke the one you do not recognize.", comment: "Settings: what to do when two devices waiting for approval show the same comparison code.")
        l.revokeTitle = String(localized: "Revoke this device?")
        l.revokeMessage = String(localized: "This device will lose access immediately. Every other device on this account will also need to silently reconnect. This cannot be undone from here. The device would need to sign in again and be approved again.", comment: "Confirmation before a device is revoked from sync.")
        l.removeRunnerTitle = CloudRoutineCopy.removeRunnerTitle
        l.removeRunnerMessage = CloudRoutineCopy.removeRunnerMessage
        l.addRunner = CloudRoutineCopy.addRunner
        l.accountLabel = String(localized: "Sync account", comment: "Label of the Settings row with the button that deletes the sync account.")
        l.deleteAccount = String(localized: "Delete my sync account…")
        l.deleteAccountTitle = String(localized: "Delete your sync account?")
        l.delete = String(localized: "Delete")
        l.upgradePlus = String(localized: "Upgrade to Plus for unlimited devices.")
        l.upgradeCloud = String(localized: "Upgrade to Cloud for unlimited devices.")
        l.planReadFailed = String(localized: "Couldn't check your plan. Try again in a moment.")
        l.useThisComputer = String(localized: "Use this computer instead")
        l.waitlistJoined = String(localized: "You're on the waitlist. Thanks for telling us what you need.")
        l.waitlistPrompt = String(localized: "Want more? Tell us what you'd use:")
        l.joinWaitlist = String(localized: "Join the waitlist")
        l.syncHeading = String(localized: "Sync", comment: "Label of the Sync heading and of the Sync rows in Settings.")
        l.syncNow = String(localized: "Sync now")
        l.syncRowLabel = String(localized: "Sync", comment: "Label of the Sync heading and of the Sync rows in Settings.")
        l.syncing = String(localized: "Syncing…")
        l.removeThisDevice = ClientCopy.removeDevice
        l.removeDeviceTitle = ClientCopy.removeDeviceTitle
        l.removeDeviceMessage = ClientCopy.removeDeviceMessage
        l.removeDeviceConfirm = String(localized: "Remove")
        l.runnerSheetTitle = CloudRoutineCopy.runnerName
        l.runnerPublicKeyHelp = String(localized: "On the host that will run your routines, run whimbrel-cloud-runner init and paste the public key it prints here.")
        l.publicKey = String(localized: "Public key")
        l.add = String(localized: "Add")
        l.done = String(localized: "Done")
        l.copy = String(localized: "Copy")
        l.copied = String(localized: "Copied")
        l.provisionInstructions = CloudRoutineCopy.provisionInstructions
        l.runnerNotShownAgain = String(localized: "These values are not shown again.", comment: "Settings: said about the values Whimbrel Cloud's host needs, which are shown once.")
        l.runnerCloseQuestion = String(localized: "These values are not shown again. Close without copying the secret one?", comment: "Settings: asked when the sheet with Whimbrel Cloud's values is closed before the secret value was copied.")
        l.closeWithoutCopying = String(localized: "Close without copying", comment: "Settings: button that closes a sheet whose secret value was not copied.")
        l.keepOpen = String(localized: "Keep it open", comment: "Settings: button that keeps a sheet open instead of closing it.")
        return l
    }
}

// MARK: Runtime state

/// The pending-clients line: the classic `ClientPendingDevicesLine`'s model, with the same loads and the same removal.
@MainActor
final class SyncPendingRuntime: ObservableObject {
    private(set) var model: ClientPendingDevicesModel?
    private var cancellable: AnyCancellable?

    init(appState: AppState) {
        if let bridge = appState.clientControlsBridge() {
            let m = ClientPendingDevicesModel(bridge: bridge)
            model = m
            cancellable = m.objectWillChange.sink { [weak self] _ in self?.objectWillChange.send() }
        }
    }
}

// MARK: The view

struct SyncDevicesSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?

    var body: some View {
        SyncDevicesSettingsRedesignBody(appState: appState, highlighted: highlighted)
    }
}

private struct SyncDevicesSettingsRedesignBody: View {
    let appState: AppState
    @ObservedObject var syncState: SyncState
    @ObservedObject var content: ContentSyncCoordinator
    @ObservedObject var settingsSync: SettingsSyncCoordinator
    @ObservedObject var cloud: CloudRoutineSettings
    let highlighted: String?
    private let presentationContextProvider = WindowPresentationContextProvider()
    @StateObject private var syncNow: SyncNowModel
    @StateObject private var pending: SyncPendingRuntime
    @State private var working = false
    @State private var joinedWaitlist = false
    @State private var phoneWaitedOut = false

    init(appState: AppState, highlighted: String?) {
        self.appState = appState
        self.syncState = appState.syncState
        self.content = appState.contentSyncCoordinator
        self.settingsSync = appState.settingsSyncCoordinator
        self.cloud = appState.settingsSyncCoordinator.cloudRoutines
        self.highlighted = highlighted
        let sync = appState.contentSyncCoordinator
        _syncNow = StateObject(wrappedValue: SyncNowModel(
            canSync: { [weak sync] in sync?.canSyncNow ?? false },
            run: { [weak sync] in await sync?.syncNowManually() ?? .unavailable }))
        _pending = StateObject(wrappedValue: SyncPendingRuntime(appState: appState))
    }

    var body: some View {
        // The stale-sync line rechecks on a timer, because a stuck round publishes nothing.
        SwiftUI.TimelineView(.periodic(from: .now, by: 300)) { context in
            WSyncDevicesSettingsView(
                model: model(now: context.date), actions: actions(), highlighted: highlighted,
                waiting: AnyView(SyncWaitingRedesignView(syncState: syncState)))
        }
        .onAppear {
            syncState.startDevicesPolling()
            Task { await syncState.refreshPlan() }
            Task { await syncState.refreshAIBalance() }
        }
        .onChange(of: syncState.status) { old, status in
            // A token renewal is .ready to .ready: not a new arrival.
            if case .ready = old, case .ready = status { return }
            switch status {
            case .ready, .deviceLimit: Task { await syncState.refreshPlan() }
            default: break
            }
        }
        .onDisappear { syncState.stopDevicesPolling() }
        .task(id: isReady) { await loadPending() }
        // The phone card waits for the first device list; if it never comes, show the beta card after a few seconds.
        .task(id: isReady) {
            phoneWaitedOut = false
            guard isReady else { return }
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            if !Task.isCancelled { phoneWaitedOut = true }
        }
        .onChange(of: content.lastSyncedAt) { _, _ in Task { await loadPending() } }
        // The "Synced" line goes away by itself; an error stays until the next try.
        .task(id: syncNow.phase) {
            guard syncNow.phase == .synced else { return }
            try? await Task.sleep(nanoseconds: 5 * 1_000_000_000)
            if !Task.isCancelled { syncNow.clearOutcome() }
        }
    }

    private var isReady: Bool { if case .ready = syncState.status { return true } else { return false } }

    private func loadPending() async { if isReady { await pending.model?.load() } }

    private func model(now: Date) -> WSyncModel {
        let phase = SyncDevicesAdapter.phase(syncState.status)
        let devices = syncState.devices
        let rows = SyncDevicesAdapter.rows(
            devices: devices, thisDeviceKey: syncState.currentDevicePublicKeyBase64, runnerLastRanMs: cloud.runnerLastRunMs,
            code: { CoreMemoryStore.deviceComparisonCode(publicKeyBase64: $0) },
            lastRan: { String(localized: "Last ran \(Date(timeIntervalSince1970: Double($0) / 1000).formatted(date: .abbreviated, time: .shortened))") })
        let planModel = PlanSettingsAdapter.model(
            status: syncState.status, plan: syncState.plan, planReadFailed: syncState.planReadFailed, balance: syncState.aiBalance,
            billingError: syncState.billingError, isWorking: working)
        var limit: WLimitModel?
        if case let .deviceLimit(kind, holders) = syncState.status {
            limit = WLimitModel(
                message: SyncState.limitMessage(kind: kind, holders: holders), actionError: syncState.actionError, purchase: planModel.purchase,
                planReadFailed: planModel.state == .readFailed, billingError: planModel.billingError, canSwap: kind == "computer" && !holders.isEmpty,
                waitlist: SyncDevicesAdapter.waitlist(), joined: joinedWaitlist, isWorking: working)
        }
        let deleteMessage = [String(localized: "This deletes the encrypted copy on the sync server and signs out every device. Nothing on your devices is deleted."), syncState.aiCreditForfeitNote]
            .compactMap { $0 }.joined(separator: " ")
        let state = SyncNowModel.buttonState(status: content.syncStatus, roundRunning: content.roundInFlight, phase: syncNow.phase)
        var pendingModel: WPendingClientsModel?
        if isReady, let m = pending.model {
            pendingModel = WPendingClientsModel(
                line: m.line, devices: m.ids.map { WPendingClientsModel.Device(id: $0, name: appState.deviceLabel(for: $0)) }, isRemoving: m.isRemoving,
                error: m.errorText, confirmingName: m.showingConfirmation ? m.confirming.map { appState.deviceLabel(for: $0) } : nil)
        }
        return WSyncModel(
            labels: SyncDevicesAdapter.labels(), planLabels: PlanSettingsAdapter.labels(), phase: phase,
            phone: PhoneCardState.isLoading(ready: isReady, loaded: syncState.devicesLoaded, waitedOut: phoneWaitedOut)
                ? .loading
                : PhoneCardState.state(devices: devices) == .connected
                ? .connected : .beta(url: SyncDevicesAdapter.betaURL, qr: SyncDevicesAdapter.qrImage()),
            deviceName: syncState.deviceName, actionError: syncState.actionError, devices: rows,
            showsAddRunner: CloudRoutineCopy.showsAddRunner(flag: CloudRoutineCopy.uiFlag()), limit: limit, deleteMessage: deleteMessage,
            staleLine: staleLine(now: now),
            syncNow: WSyncNowModel(
                isEnabled: state == .enabled, isSyncing: state == .syncing, message: syncNow.message, messageIsError: syncNow.phase != .synced),
            notes: isReady ? notes() : [], pending: pendingModel, anchors: SettingsAnchorID.sync, tellAFriend: TellAFriend.labels())
    }

    private func staleLine(now: Date) -> String? {
        guard content.isStale(now: now), let last = content.lastSuccess else { return nil }
        let when = last.formatted(date: .abbreviated, time: .shortened)
        return content.retryPossible
            ? String(localized: "Sync hasn't worked since \(when). Whimbrel keeps trying.")
            : String(localized: "Sync hasn't worked since \(when).")
    }

    /// "Settings last synced", "Content last synced", the stalled-outbox line and the indexing line, as the classic list shows them.
    private func notes() -> [WSyncNote] {
        var out: [WSyncNote] = []
        if let error = settingsSync.lastSyncError {
            out.append(WSyncNote(id: "settings", text: error))
        } else if let at = settingsSync.lastSyncedAt {
            out.append(WSyncNote(id: "settings", text: String(localized: "Settings last synced \(at.formatted(.relative(presentation: .named)))")))
        }
        if let error = content.lastSyncError {
            out.append(WSyncNote(id: "content", text: error))
        } else if let at = content.lastSyncedAt {
            out.append(WSyncNote(id: "content", text: String(localized: "Content last synced \(at.formatted(.relative(presentation: .named)))")))
        }
        if let backlog = content.outboxBacklog, let oldest = backlog.oldest,
           SyncTiming.backlogIsStalled(pending: backlog.pending, oldest: oldest, now: Date()) {
            out.append(WSyncNote(id: "backlog", text: String(localized: "Changes waiting to send: \(backlog.pending), the oldest from \(oldest.formatted(.dateTime.month().day().hour().minute()))")))
        }
        if let percent = content.kindBackfillPercent {
            out.append(WSyncNote(id: "indexing", text: String(localized: "Indexing history: \(percent)%")))
        }
        return out
    }

    private func device(_ id: String) -> SyncDeviceRecord? { syncState.devices.first { $0.id == id } }

    private func actions() -> WSyncActions {
        let planPurchase = PlanSettingsAdapter.model(
            status: syncState.status, plan: syncState.plan, planReadFailed: syncState.planReadFailed, balance: nil, billingError: nil, isWorking: false).purchase
        return WSyncActions(
            enableSync: { Task { await syncState.enableSync(presentationContextProvider: presentationContextProvider) } },
            setDeviceName: { syncState.deviceName = $0 },
            saveDeviceName: { Task { await syncState.saveDeviceName() } },
            approve: { id in if let d = device(id) { Task { await syncState.approve(d) } } },
            revoke: { id in if let d = device(id) { Task { await syncState.revoke(d) } } },
            removeRunner: { id in if let d = device(id) { Task { await syncState.revoke(d) } } },
            // A fresh balance for the forfeit line in the confirmation.
            deleteAccountAsked: { Task { await syncState.refreshAIBalance() } },
            deleteAccount: { Task { await syncState.deleteSyncAccount() } },
            swapIn: {
                guard case let .deviceLimit(_, holders) = syncState.status, let holder = holders.first else { return }
                Task {
                    working = true
                    await syncState.swap(replacing: holder)
                    working = false
                }
            },
            joinWaitlist: { wants in
                Task {
                    working = true
                    joinedWaitlist = await syncState.joinWaitlist(wants: wants)
                    working = false
                }
            },
            buy: { interval in
                Task {
                    working = true
                    await syncState.upgrade(interval: interval, plan: planPurchase == .plus ? "plus" : nil)
                    working = false
                }
            },
            syncNow: { Task { await syncNow.start() } },
            openURL: { if let url = URL(string: $0) { NSWorkspace.shared.open(url) } },
            pendingAsk: { pending.model?.ask($0) },
            pendingCancel: { pending.model?.cancel() },
            pendingConfirm: { pending.model?.confirm() },
            registerRunner: { key in
                switch await syncState.registerRunner(publicKey: key) {
                case .success(let provision): return .success(SyncDevicesAdapter.provisionValues(provision, workerURL: syncState.workerURL))
                case .failure(let error): return .failure(error.text)
                }
            },
            copy: { SettingsPasteboard.copy($0) },
            tellAFriend: { TellAFriend.copy() })
    }
}
