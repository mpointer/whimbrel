import SwiftUI

// Models, actions and words for Sync & Devices, Connectors and Automations. The app builds the models from its state and
// passes the actions; nothing here reads the app. Values that are shown once (an MCP token, the runner's provision
// bundle) are never part of a long-lived model: they live in `WShownOnceState`, which forgets them when it closes.

// MARK: Anchors

public struct WSyncAnchors: Equatable, Sendable {
    public var overview: String
    public var syncNow: String
    public var name: String
    public var phone: String
    public var deleteAccount: String
    public var tellAFriend: String

    public init(overview: String, syncNow: String, name: String, phone: String, deleteAccount: String, tellAFriend: String) {
        self.tellAFriend = tellAFriend
        self.overview = overview
        self.syncNow = syncNow
        self.name = name
        self.phone = phone
        self.deleteAccount = deleteAccount
    }

    public var all: [String] { [overview, syncNow, name, phone, deleteAccount, tellAFriend] }
}

public struct WConnectorsAnchors: Equatable, Sendable {
    public var calendar: String
    public var gmail: String
    public var outlook: String
    public var ownClient: String
    public var mcp: String

    public init(calendar: String, gmail: String, outlook: String, ownClient: String, mcp: String) {
        self.calendar = calendar
        self.gmail = gmail
        self.outlook = outlook
        self.ownClient = ownClient
        self.mcp = mcp
    }

    public var all: [String] { [calendar, gmail, outlook, ownClient, mcp] }
}

// MARK: Values shown once

/// One value in a "shown once" sheet. `isSecret` says that closing before copying it must ask first.
public struct WShownValue: Identifiable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var value: String
    public var isSecret: Bool
    public var copied: Bool

    public init(id: String, name: String, value: String, isSecret: Bool = true, copied: Bool = false) {
        self.id = id
        self.name = name
        self.value = value
        self.isSecret = isSecret
        self.copied = copied
    }
}

/// What a shown-once sheet knows: the values, which of them were copied, and whether it is asking before it closes.
/// Closing empties it, so nothing it held outlives the sheet.
public struct WShownOnceState: Equatable, Sendable {
    public private(set) var values: [WShownValue]
    public private(set) var asking = false

    public init(values: [WShownValue]) { self.values = values }

    /// "Copied" shows only after the pasteboard write succeeded; a failed write leaves the value uncopied.
    public mutating func copyFinished(id: String, succeeded: Bool) {
        guard succeeded, let index = values.firstIndex(where: { $0.id == id }) else { return }
        values[index].copied = true
    }

    public var needsAskBeforeClosing: Bool { values.contains { $0.isSecret && !$0.copied } }

    /// Done or Escape: true when the sheet may close now; otherwise it starts asking.
    public mutating func requestClose() -> Bool {
        if needsAskBeforeClosing {
            asking = true
            return false
        }
        return true
    }

    public mutating func keepOpen() { asking = false }

    /// The sheet closed. Every value is dropped.
    public mutating func closed() {
        values = []
        asking = false
    }
}

// MARK: Sync & Devices

/// Where sign-in to sync stands. The waiting screen is the app's own view, put in a slot.
public enum WSyncPhase: Equatable, Sendable {
    case signedOut, signingIn, waiting, ready, deviceLimit
    case failed(String)
}

public struct WSyncStatusLine: Equatable, Sendable {
    public var text: String
    public var tone: WStatusTone
}

public enum WSyncStatusRule {
    /// The one-word state in the status row. VoiceOver announces it when it changes.
    public static func line(_ phase: WSyncPhase, labels l: WSyncLabels) -> WSyncStatusLine {
        switch phase {
        case .signedOut: return WSyncStatusLine(text: l.stateOff, tone: .neutral)
        case .signingIn: return WSyncStatusLine(text: l.stateSigningIn, tone: .neutral)
        case .waiting: return WSyncStatusLine(text: l.stateWaiting, tone: .warning)
        case .ready: return WSyncStatusLine(text: l.stateOn, tone: .good)
        case .deviceLimit: return WSyncStatusLine(text: l.stateLimit, tone: .warning)
        case .failed: return WSyncStatusLine(text: l.stateFailed, tone: .problem)
        }
    }

    /// What is spoken when the state changes.
    public static func announcement(_ phase: WSyncPhase, labels l: WSyncLabels) -> String {
        l.statusLabel + ": " + line(phase, labels: l).text
    }
}

/// One device in the account's list, with what each button may do. The rules are plain inputs, so they can be tested.
public struct WDeviceRow: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable { case person, runner }
    public enum Code: Equatable, Sendable {
        /// Not pending: no code to compare.
        case none
        case value(String)
        /// Pending, and no code could be worked out.
        case unverifiable
    }

    public var id: String
    public var name: String
    public var statusText: String
    public var lastRan: String?
    public var kind: Kind
    public var code: Code
    /// Another pending device shows the same code.
    public var collides: Bool
    public var canApprove: Bool
    public var canResend: Bool
    public var canRevoke: Bool
    public var canRemoveRunner: Bool

    /// Approve is a plain (secondary) button: when it is on it reads as on, and it is never the window's default action.
    /// With a code collision it is off, and the row's own collision notice below it says why.
    public var approveKind: WButtonKind { .secondary }
    public var approveIsEnabled: Bool { canApprove && !collides }

    public static func make(
        id: String, name: String, status: String, statusText: String, isRunner: Bool, keyDiffersFromThisDevice: Bool, code: String?,
        collides: Bool, canRemoveRunner: Bool, lastRan: String? = nil
    ) -> WDeviceRow {
        let pending = status == "pending"
        let codeValue: Code = pending ? (code.map(Code.value) ?? .unverifiable) : .none
        return WDeviceRow(
            id: id, name: name, statusText: statusText, lastRan: lastRan, kind: isRunner ? .runner : .person, code: codeValue,
            collides: pending && collides, canApprove: !isRunner && pending,
            canResend: !isRunner && status == "approved" && keyDiffersFromThisDevice,
            canRevoke: !isRunner && status != "revoked", canRemoveRunner: isRunner && canRemoveRunner)
    }
}

public enum WPhoneCardModel {
    /// The beta card: its words, the sign-up page and a QR image of it.
    case beta(url: String, qr: Image?)
    /// An approved phone is on the account: one line.
    case connected
    /// The first device list has not arrived: draw nothing rather than a card that may collapse a moment later.
    case loading
}

public struct WWaitlistOption: Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public init(id: String, label: String) {
        self.id = id
        self.label = label
    }
}

/// The free plan's device limit: swap this computer in, or join the waitlist.
public struct WLimitModel {
    public var message: String
    public var actionError: String?
    /// Which purchase the server's plan answer calls for (the app reads it). `.none` shows no purchase line.
    public var purchase: WPlanSettingsModel.Purchase
    public var planReadFailed: Bool
    public var billingError: String?
    public var canSwap: Bool
    public var waitlist: [WWaitlistOption]
    public var joined: Bool
    public var isWorking: Bool

    public init(
        message: String, actionError: String? = nil, purchase: WPlanSettingsModel.Purchase = .none, planReadFailed: Bool = false,
        billingError: String? = nil, canSwap: Bool = false, waitlist: [WWaitlistOption] = [], joined: Bool = false, isWorking: Bool = false
    ) {
        self.message = message
        self.actionError = actionError
        self.purchase = purchase
        self.planReadFailed = planReadFailed
        self.billingError = billingError
        self.canSwap = canSwap
        self.waitlist = waitlist
        self.joined = joined
        self.isWorking = isWorking
    }
}

public struct WSyncNowModel: Equatable, Sendable {
    public var isEnabled: Bool
    public var isSyncing: Bool
    public var message: String?
    public var messageIsError: Bool

    public init(isEnabled: Bool, isSyncing: Bool = false, message: String? = nil, messageIsError: Bool = false) {
        self.isEnabled = isEnabled
        self.isSyncing = isSyncing
        self.message = message
        self.messageIsError = messageIsError
    }
}

/// Devices that still need to update before they apply client deletes, each with "Remove this device".
public struct WPendingClientsModel: Equatable, Sendable {
    public struct Device: Identifiable, Equatable, Sendable {
        public var id: String
        public var name: String
        public init(id: String, name: String) {
            self.id = id
            self.name = name
        }
    }

    public var line: String?
    public var devices: [Device]
    public var isRemoving: Bool
    public var error: String?
    /// The device the open confirmation is for.
    public var confirmingName: String?

    public init(line: String? = nil, devices: [Device] = [], isRemoving: Bool = false, error: String? = nil, confirmingName: String? = nil) {
        self.line = line
        self.devices = devices
        self.isRemoving = isRemoving
        self.error = error
        self.confirmingName = confirmingName
    }
}

public struct WSyncNote: Identifiable, Equatable, Sendable {
    public var id: String
    public var text: String
    public init(id: String, text: String) {
        self.id = id
        self.text = text
    }
}

public struct WSyncModel {
    public var labels: WSyncLabels
    public var planLabels: WPlanLabels
    public var phase: WSyncPhase
    public var phone: WPhoneCardModel
    public var deviceName: String
    public var actionError: String?
    public var devices: [WDeviceRow]
    public var showsAddRunner: Bool
    public var limit: WLimitModel?
    /// The words of the delete confirmation, with any AI credit note the app adds.
    public var deleteMessage: String
    public var staleLine: String?
    public var syncNow: WSyncNowModel
    public var notes: [WSyncNote]
    public var pending: WPendingClientsModel?
    public var anchors: WSyncAnchors
    /// The words of the Tell a friend card, or nil for no card.
    public var tellAFriend: WTellAFriendLabels?

    public init(
        labels: WSyncLabels = WSyncLabels(), planLabels: WPlanLabels = WPlanLabels(), phase: WSyncPhase, phone: WPhoneCardModel,
        deviceName: String = "", actionError: String? = nil, devices: [WDeviceRow] = [], showsAddRunner: Bool = false, limit: WLimitModel? = nil,
        deleteMessage: String = "", staleLine: String? = nil, syncNow: WSyncNowModel, notes: [WSyncNote] = [],
        pending: WPendingClientsModel? = nil, anchors: WSyncAnchors, tellAFriend: WTellAFriendLabels? = nil
    ) {
        self.labels = labels
        self.planLabels = planLabels
        self.phase = phase
        self.phone = phone
        self.deviceName = deviceName
        self.actionError = actionError
        self.devices = devices
        self.showsAddRunner = showsAddRunner
        self.limit = limit
        self.deleteMessage = deleteMessage
        self.staleLine = staleLine
        self.syncNow = syncNow
        self.notes = notes
        self.pending = pending
        self.anchors = anchors
        self.tellAFriend = tellAFriend
    }
}

/// What registering Whimbrel Cloud answered.
public enum WRunnerRegistration: Sendable {
    case success([WShownValue])
    case failure(String)
}

public struct WSyncActions {
    public var enableSync: () -> Void
    public var setDeviceName: (String) -> Void
    public var saveDeviceName: () -> Void
    public var approve: (String) -> Void
    public var revoke: (String) -> Void
    public var removeRunner: (String) -> Void
    public var deleteAccountAsked: () -> Void
    public var deleteAccount: () -> Void
    public var swapIn: () -> Void
    public var joinWaitlist: (Set<String>) -> Void
    public var buy: (_ interval: String) -> Void
    public var syncNow: () -> Void
    public var openURL: (String) -> Void
    public var pendingAsk: (String) -> Void
    public var pendingCancel: () -> Void
    public var pendingConfirm: () -> Void
    public var registerRunner: (String) async -> WRunnerRegistration
    /// Writes to the pasteboard; true when it worked.
    public var copy: (String) -> Bool
    /// Copies the Tell a friend message to the pasteboard; true when it worked.
    public var tellAFriend: () -> Bool

    public init(
        enableSync: @escaping () -> Void = {}, setDeviceName: @escaping (String) -> Void = { _ in }, saveDeviceName: @escaping () -> Void = {},
        approve: @escaping (String) -> Void = { _ in }, revoke: @escaping (String) -> Void = { _ in }, removeRunner: @escaping (String) -> Void = { _ in },
        deleteAccountAsked: @escaping () -> Void = {}, deleteAccount: @escaping () -> Void = {}, swapIn: @escaping () -> Void = {},
        joinWaitlist: @escaping (Set<String>) -> Void = { _ in }, buy: @escaping (String) -> Void = { _ in }, syncNow: @escaping () -> Void = {},
        openURL: @escaping (String) -> Void = { _ in }, pendingAsk: @escaping (String) -> Void = { _ in }, pendingCancel: @escaping () -> Void = {},
        pendingConfirm: @escaping () -> Void = {}, registerRunner: @escaping (String) async -> WRunnerRegistration = { _ in .failure("") },
        copy: @escaping (String) -> Bool = { _ in true }, tellAFriend: @escaping () -> Bool = { true }
    ) {
        self.tellAFriend = tellAFriend
        self.enableSync = enableSync
        self.setDeviceName = setDeviceName
        self.saveDeviceName = saveDeviceName
        self.approve = approve
        self.revoke = revoke
        self.removeRunner = removeRunner
        self.deleteAccountAsked = deleteAccountAsked
        self.deleteAccount = deleteAccount
        self.swapIn = swapIn
        self.joinWaitlist = joinWaitlist
        self.buy = buy
        self.syncNow = syncNow
        self.openURL = openURL
        self.pendingAsk = pendingAsk
        self.pendingCancel = pendingCancel
        self.pendingConfirm = pendingConfirm
        self.registerRunner = registerRunner
        self.copy = copy
    }
}

// MARK: Connectors

public enum WConnectorService: String, Equatable, Sendable { case calendar, gmail, outlook }

public enum WAddAccountState: Equatable, Sendable {
    case idle, connecting
    case error(String)
}

public struct WAccountRow: Identifiable, Equatable, Sendable {
    public var id: String { email }
    public var email: String
    public var isDefault: Bool
    public var detail: String
    /// Shown instead of the detail: the account needs to sign in again, or Google refused access.
    public var reconnect: String?
    public var showsMakeDefault: Bool
    /// What Remove asks first. Nil: Remove acts at once (Outlook, as today).
    public var removeQuestion: WAskModel?

    public init(email: String, isDefault: Bool, detail: String, reconnect: String? = nil, showsMakeDefault: Bool, removeQuestion: WAskModel?) {
        self.email = email
        self.isDefault = isDefault
        self.detail = detail
        self.reconnect = reconnect
        self.showsMakeDefault = showsMakeDefault
        self.removeQuestion = removeQuestion
    }
}

/// A confirmation's words.
public struct WAskModel: Equatable, Sendable {
    public var title: String
    public var message: String
    public init(title: String, message: String) {
        self.title = title
        self.message = message
    }
}

public struct WConnectorModel: Equatable, Sendable {
    public var heading: String
    public var accounts: [WAccountRow]
    public var addTitle: String
    public var addState: WAddAccountState
    public var anchor: String

    public init(heading: String, accounts: [WAccountRow], addTitle: String, addState: WAddAccountState, anchor: String) {
        self.heading = heading
        self.accounts = accounts
        self.addTitle = addTitle
        self.addState = addState
        self.anchor = anchor
    }
}

/// The own Google client. The secret is never part of the model: only whether one is saved, and the client id (not secret).
public struct WOwnClientModel: Equatable, Sendable {
    public var isSaved: Bool
    public var clientId: String
    public var error: String?

    public init(isSaved: Bool, clientId: String, error: String? = nil) {
        self.isSaved = isSaved
        self.clientId = clientId
        self.error = error
    }
}

public struct WMcpClientRow: Identifiable, Equatable, Sendable {
    public var id: String
    public var name: String
    /// "Memory, Meetings, Write · created Nov 3, 2026".
    public var detail: String
    /// "Revoked Nov 5, 2026" once revoked; the Revoke button is gone then.
    public var revokedText: String?

    public init(id: String, name: String, detail: String, revokedText: String? = nil) {
        self.id = id
        self.name = name
        self.detail = detail
        self.revokedText = revokedText
    }
}

public struct WMcpScopeRow: Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var detail: String?
    public var isOn: Bool
    public init(id: String, label: String, detail: String? = nil, isOn: Bool) {
        self.id = id
        self.label = label
        self.detail = detail
        self.isOn = isOn
    }
}

public struct WMcpModel: Equatable, Sendable {
    public var serverOn: Bool
    public var serverStatus: String
    public var serverError: String?
    public var name: String
    public var scopes: [WMcpScopeRow]
    public var canAdd: Bool
    public var error: String?
    public var clients: [WMcpClientRow]

    public init(
        serverOn: Bool, serverStatus: String, serverError: String? = nil, name: String, scopes: [WMcpScopeRow], canAdd: Bool, error: String? = nil,
        clients: [WMcpClientRow]
    ) {
        self.serverOn = serverOn
        self.serverStatus = serverStatus
        self.serverError = serverError
        self.name = name
        self.scopes = scopes
        self.canAdd = canAdd
        self.error = error
        self.clients = clients
    }
}

public struct WConnectorsModel {
    public var labels: WConnectorsLabels
    public var calendar: WConnectorModel
    public var gmail: WConnectorModel
    public var outlook: WConnectorModel
    public var ownClient: WOwnClientModel
    public var mcp: WMcpModel
    /// The token while its sheet is open. Nil before and after.
    public var shownToken: [WShownValue]?
    public var anchors: WConnectorsAnchors

    public init(
        labels: WConnectorsLabels = WConnectorsLabels(), calendar: WConnectorModel, gmail: WConnectorModel, outlook: WConnectorModel,
        ownClient: WOwnClientModel, mcp: WMcpModel, shownToken: [WShownValue]? = nil, anchors: WConnectorsAnchors
    ) {
        self.labels = labels
        self.calendar = calendar
        self.gmail = gmail
        self.outlook = outlook
        self.ownClient = ownClient
        self.mcp = mcp
        self.shownToken = shownToken
        self.anchors = anchors
    }
}

public struct WConnectorsActions {
    public var makeDefault: (WConnectorService, String) -> Void
    public var removeAccount: (WConnectorService, String) -> Void
    public var addAccount: (WConnectorService) -> Void
    /// The client id and the secret just typed. The secret goes straight to the keychain call and is not kept.
    /// Answers whether the client was saved; the row keeps what was typed when it was not.
    public var saveOwnClient: (_ clientId: String, _ secret: String) -> Bool
    public var removeOwnClient: () -> Void
    public var setServerOn: (Bool) -> Void
    public var setClientName: (String) -> Void
    public var setScope: (String, Bool) -> Void
    public var addClient: () -> Void
    public var revokeClient: (String) -> Void
    public var closeToken: () -> Void
    public var copy: (String) -> Bool

    public init(
        makeDefault: @escaping (WConnectorService, String) -> Void = { _, _ in }, removeAccount: @escaping (WConnectorService, String) -> Void = { _, _ in },
        addAccount: @escaping (WConnectorService) -> Void = { _ in }, saveOwnClient: @escaping (String, String) -> Bool = { _, _ in true },
        removeOwnClient: @escaping () -> Void = {}, setServerOn: @escaping (Bool) -> Void = { _ in }, setClientName: @escaping (String) -> Void = { _ in },
        setScope: @escaping (String, Bool) -> Void = { _, _ in }, addClient: @escaping () -> Void = {}, revokeClient: @escaping (String) -> Void = { _ in },
        closeToken: @escaping () -> Void = {}, copy: @escaping (String) -> Bool = { _ in true }
    ) {
        self.makeDefault = makeDefault
        self.removeAccount = removeAccount
        self.addAccount = addAccount
        self.saveOwnClient = saveOwnClient
        self.removeOwnClient = removeOwnClient
        self.setServerOn = setServerOn
        self.setClientName = setClientName
        self.setScope = setScope
        self.addClient = addClient
        self.revokeClient = revokeClient
        self.closeToken = closeToken
        self.copy = copy
    }
}

// MARK: Automations

public struct WRoutineCloud: Equatable, Sendable {
    public var isOn: Bool
    /// Why it cannot be turned on. Turning it off is always allowed.
    public var blockedReason: String?
    public var toggleCopy: String
    public var disclosure: String
    public var statusNote: String?
    /// The ready-by time as a date (only its hour and minute count). Nil: this routine has no ready-by time.
    public var readyBy: Date?
    public var readyByNote: String
    public var freshness: String?

    public init(
        isOn: Bool, blockedReason: String? = nil, toggleCopy: String, disclosure: String, statusNote: String? = nil, readyBy: Date? = nil,
        readyByNote: String = "", freshness: String? = nil
    ) {
        self.isOn = isOn
        self.blockedReason = blockedReason
        self.toggleCopy = toggleCopy
        self.disclosure = disclosure
        self.statusNote = statusNote
        self.readyBy = readyBy
        self.readyByNote = readyByNote
        self.freshness = freshness
    }
}

public struct WRoutineRow: Identifiable, Equatable, Sendable {
    public enum Timing: Equatable, Sendable {
        /// Meeting prep: how long before each meeting.
        case leadMinutes(Int, range: ClosedRange<Int>)
        /// Everything else: a time of day, "HH:MM".
        case dailyAt(String)
    }

    public var id: String
    public var name: String
    public var enabled: Bool
    /// Nil: this routine has no write step, so no switch is offered.
    public var autoApprove: Bool?
    public var timing: Timing
    public var modelOptions: [WChoice]?
    public var modelSelection: String
    public var cloud: WRoutineCloud?

    public init(
        id: String, name: String, enabled: Bool, autoApprove: Bool?, timing: Timing, modelOptions: [WChoice]? = nil, modelSelection: String = "",
        cloud: WRoutineCloud? = nil
    ) {
        self.id = id
        self.name = name
        self.enabled = enabled
        self.autoApprove = autoApprove
        self.timing = timing
        self.modelOptions = modelOptions
        self.modelSelection = modelSelection
        self.cloud = cloud
    }
}

public struct WRunsOnModel: Equatable, Sendable {
    public var options: [WChoice]
    public var selection: String
    public var chosenLine: String?
    public var note: String?

    public init(options: [WChoice], selection: String, chosenLine: String? = nil, note: String? = nil) {
        self.options = options
        self.selection = selection
        self.chosenLine = chosenLine
        self.note = note
    }
}

public struct WAutomationsModel {
    public var labels: WAutomationsLabels
    public var runsOn: WRunsOnModel
    public var routines: [WRoutineRow]

    public init(labels: WAutomationsLabels = WAutomationsLabels(), runsOn: WRunsOnModel, routines: [WRoutineRow]) {
        self.labels = labels
        self.runsOn = runsOn
        self.routines = routines
    }
}

public struct WAutomationsActions {
    public var setRunsOn: (String) -> Void
    public var setEnabled: (String, Bool) -> Void
    public var setAutoApprove: (String, Bool) -> Void
    public var setLeadMinutes: (Int) -> Void
    public var setSchedule: (String, String) -> Void
    public var setModel: (String, String) -> Void
    public var setCloud: (String, Bool) -> Void
    public var setReadyBy: (String, Date) -> Void

    public init(
        setRunsOn: @escaping (String) -> Void = { _ in }, setEnabled: @escaping (String, Bool) -> Void = { _, _ in },
        setAutoApprove: @escaping (String, Bool) -> Void = { _, _ in }, setLeadMinutes: @escaping (Int) -> Void = { _ in },
        setSchedule: @escaping (String, String) -> Void = { _, _ in }, setModel: @escaping (String, String) -> Void = { _, _ in },
        setCloud: @escaping (String, Bool) -> Void = { _, _ in }, setReadyBy: @escaping (String, Date) -> Void = { _, _ in }
    ) {
        self.setRunsOn = setRunsOn
        self.setEnabled = setEnabled
        self.setAutoApprove = setAutoApprove
        self.setLeadMinutes = setLeadMinutes
        self.setSchedule = setSchedule
        self.setModel = setModel
        self.setCloud = setCloud
        self.setReadyBy = setReadyBy
    }
}

/// A time of day typed as "HH:MM".
public enum WTimeRule {
    /// The text as "HH:MM" when it is a time of day (hours 00 to 23, minutes 00 to 59), else nil.
    public static func parse(_ text: String) -> String? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let parts = t.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 2, parts[0].count == 2, parts[1].count == 2,
              let h = Int(parts[0]), let m = Int(parts[1]), (0...23).contains(h), (0...59).contains(m) else { return nil }
        return t
    }
}
