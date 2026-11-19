import AppKit
import Foundation

// Client controls on a seed topic's page (X6, spec docs/superpowers/specs/2026-11-12-client-controls-design.md
// sections 4-7): delete with a review and a receipt, keep for N months, export, exclude from hosted models.
// The models hold no store: every core call goes through `ClientControlsBridge`, so tests run without one.

// MARK: - Seam

/// The core calls the client controls make. The real bridge runs reads on `CoreMemoryStore.accessQueue` and the
/// slow work (delete, expire, export write) on `CoreMemoryStore.slowLaneQueue` with the slow-lane store.
@MainActor
protocol ClientControlsBridge: AnyObject {
    func inventory(key: String) async -> Result<ClientInventory, ClientOpError>
    /// Waits up to two minutes for background work before it starts (`.busy` past that, nothing touched).
    /// Waits for running background work as long as it takes (the wait ends early with `.cancelled` after
    /// `cancelWait(token:)`), then deletes. `token` is new for each call. `reviewedLinked`: the linked items the
    /// user saw; core answers `.selectionChanged` if the linked set differs once the wait is over.
    func delete(key: String, includePossible: [ClientItemRef], reviewedLinked: [ClientItemRef], deleteExports: Bool, token: UInt64) async -> Result<ClientDeleteReceipt, ClientOpError>
    /// Background ticks still running, for "Waiting for background work to finish…". Cheap.
    func jobsRunning() -> Int
    /// False: too late, the delete has started.
    func cancelWait(token: UInt64) -> Bool
    func control(key: String) async -> Result<ClientControl, ClientOpError>
    /// Reads the client's current settings (as synced from other devices) and saves `change` applied to them,
    /// in one turn on the store's queue, so a value the page has not seen yet is never overwritten.
    func updateControl(key: String, _ change: @escaping (ClientControl) -> ClientControl) async -> Result<ClientControlUpdate, ClientOpError>
    func resummaryOffer(key: String) async -> Result<ClientResummaryOffer, ClientOpError>
    /// Summarises these meetings again; core keeps an excluded client's meeting on this Mac.
    func resummarize(meetings: [Int64])
    func reviewFlag() async -> Result<ClientReviewFlag?, ClientOpError>
    func dismissReviewFlag() async -> Result<Void, ClientOpError>
    func dueItems() async -> Result<[ClientDue], ClientOpError>
    func dueItems(key: String) async -> Result<[ClientItem], ClientOpError>
    func decide(key: String, item: ClientItemRef, decision: ClientRetentionDecision) async -> Result<Void, ClientOpError>
    /// One budgeted expiry call; the caller repeats while `more`.
    func expire(key: String, items: [ClientItemRef]) async -> Result<ClientExpiryReceipt, ClientOpError>
    func exportWrite(key: String, destParent: URL, includePossible: [ClientItemRef], includeMedia: Bool) async -> Result<ClientExportWritten, ClientOpError>
    func exportCopyMedia(token: UInt64) async -> Result<ClientExportDone, ClientOpError>
    /// Devices on the relay that hold back every client's settings from the others (`topic_rows_held_devices`);
    /// nil when not measured yet or unreadable (never shown as 0).
    func heldDevices() async -> Int?
}

// MARK: - Settings: exclude from hosted models, keep for N months

/// A settings write: the value it started from (read just before) and the value core saved.
struct ClientControlUpdate: Equatable {
    let before: ClientControl
    let after: ClientControl
}

@MainActor
final class ClientSettingsModel: ObservableObject {
    /// Shown when exclusion is turned on (core's `api::exclusion::ALREADY_LEFT_NOTE`).
    static let alreadyLeftSentence = String(localized: "Material that already left cannot be recalled.")
    /// The keep periods offered, in months; nil is Off. Spec 5 offers 12 by default.
    static let keepChoices: [Int?] = [nil, 3, 6, 12, 24, 36]

    /// What turning exclusion on shows: the already-left sentence and the meetings a hosted model summarised.
    struct ExclusionNotice: Equatable {
        let alreadyLeft: String
        let hostedSummaries: [Int64]
        var resummaryStarted = false
    }

    let key: String
    private let bridge: ClientControlsBridge

    /// The last value core saved; every write starts from it, so the toggle and the picker never undo each other.
    @Published private(set) var saved: ClientControl?
    /// False when core says this topic is not a live client: the page shows no Client section.
    @Published private(set) var isClient = true
    @Published private(set) var errorText: String?
    @Published private(set) var exclusionNotice: ExclusionNotice?
    @Published private(set) var isSaving = false
    /// What the user last chose, shown while it saves: the switch and the picker follow a click at
    /// once instead of waiting for core (0.9.80 device check, row 9). nil when nothing is pending.
    @Published private(set) var requested: ClientControl?
    /// Devices that keep this setting from reaching the others (`ClientCopy.heldDevices`); nil: not known.
    @Published private(set) var heldDevices: Int?

    private var writes: Task<Void, Never>?

    init(key: String, bridge: ClientControlsBridge) {
        self.key = key
        self.bridge = bridge
    }

    var excludeHosted: Bool { (requested ?? saved)?.excludeHosted ?? false }
    /// nil: Off (core's nil or 0).
    var keepMonths: Int? { (requested ?? saved)?.keepMonths.flatMap { $0 > 0 ? $0 : nil } }

    /// The picker's choices, with a saved period that is not one of them (set on another device) kept.
    var keepOptions: [Int?] {
        guard let current = keepMonths, !Self.keepChoices.contains(current) else { return Self.keepChoices }
        return (Self.keepChoices + [current]).sorted { ($0 ?? 0) < ($1 ?? 0) }
    }

    func load() async {
        heldDevices = await bridge.heldDevices()
        switch await bridge.control(key: key) {
        case .success(let c):
            saved = c
            isClient = true
            errorText = nil
        case .failure(.notFound):
            isClient = false
        case .failure(let e):
            errorText = ClientCopy.message(e)
        }
    }

    func setExcludeHosted(_ on: Bool) async {
        await write { current in
            var next = current
            next.excludeHosted = on
            return next
        }
    }

    func setKeepMonths(_ months: Int?) async {
        await write { current in
            var next = current
            next.keepMonths = months
            return next
        }
    }

    /// Summarises the listed meetings again (a local model, or not made).
    func resummarize() {
        guard var notice = exclusionNotice, !notice.hostedSummaries.isEmpty, !notice.resummaryStarted else { return }
        bridge.resummarize(meetings: notice.hostedSummaries)
        notice.resummaryStarted = true
        exclusionNotice = notice
    }

    /// Writes one at a time; each applies its change to the value read at write time. The choice
    /// shows at once (`requested`) and stays until the last queued write has finished; a failed
    /// write then falls back to what core holds, with the error shown.
    private func write(_ change: @escaping (ClientControl) -> ClientControl) async {
        if let base = requested ?? saved { requested = change(base) }
        let previous = writes
        let task = Task { @MainActor [weak self] in
            await previous?.value
            await self?.apply(change)
        }
        writes = task
        await task.value
        if writes == task {
            requested = nil
        }
    }

    private func apply(_ change: @escaping (ClientControl) -> ClientControl) async {
        guard saved != nil else { return }
        isSaving = true
        // Applied to the stored value, not `saved`: another device may have changed it since the page loaded.
        let result = await bridge.updateControl(key: key, change)
        // Saved (or failed) here; the re-summary offer below is extra reading, not part of the save.
        isSaving = false
        switch result {
        case .success(let u):
            saved = u.after
            errorText = nil
            if u.after.excludeHosted && !u.before.excludeHosted {
                // Core's offer and sentence; the sentence shows even if the offer could not be read.
                let offer = try? await bridge.resummaryOffer(key: key).get()
                let note = offer.map(\.alreadyLeftNote).flatMap { $0.isEmpty ? nil : $0 } ?? Self.alreadyLeftSentence
                exclusionNotice = ExclusionNotice(alreadyLeft: note, hostedSummaries: offer?.meetings ?? [])
            } else if !u.after.excludeHosted {
                exclusionNotice = nil
            }
        case .failure(.notFound):
            isClient = false
        case .failure(let e):
            errorText = ClientCopy.message(e)
        }
    }
}

// MARK: - Delete and export review

@MainActor
final class ClientReviewModel: ObservableObject, Identifiable {
    enum Mode: Equatable { case delete, export }

    enum Phase: Equatable {
        case loading
        case review
        /// The delete or export is running: the button is off and a spinner shows.
        case running
        /// Confirmed, but background work is still running: nothing is deleted yet and Cancel is offered.
        case waiting
        case receipt(ClientDeleteReceipt)
        case exported(ExportOutcome)
        /// The client is not there any more (deleted elsewhere, or merged).
        case gone
        /// This Mac has no device identity: no delete is offered, and retrying cannot help.
        case noIdentity
        case failed(ClientOpError)
    }

    /// Why the review screen shows again after a Delete press.
    enum Notice: Equatable {
        /// The list changed under the review: it was read again, and ticks it no longer lists were dropped.
        case selectionChanged
        /// Background work did not finish in time; nothing was deleted.
        case busy
    }

    struct ExportOutcome: Equatable {
        let folder: String
        let media: ClientExportDone.Media?
        let partial: Bool
    }

    nonisolated let id = UUID()
    let key: String
    /// The client's name, typed to confirm. Never put in a receipt.
    let name: String
    let mode: Mode
    let hasDeviceIdentity: Bool
    private let bridge: ClientControlsBridge

    @Published private(set) var phase: Phase = .loading
    @Published private(set) var inventory: ClientInventory?
    /// Possible matches the user ticked. Empty to start: nothing that only names the client is deleted unless ticked.
    @Published private(set) var ticked: Set<ClientItemRef> = []
    @Published var confirmText = ""
    /// Also delete the export folders Whimbrel recorded for this client. Off unless the user ticks it.
    @Published var deleteExports = false
    /// Export only: audio and slide images too (off by default, because of size).
    @Published var includeMedia = false
    @Published private(set) var notice: Notice?
    /// A partial delete whose client record is already gone: nothing is left to run again.
    @Published private(set) var retryGone = false

    init(key: String, name: String, mode: Mode, hasDeviceIdentity: Bool, bridge: ClientControlsBridge) {
        self.key = key
        self.name = name
        self.mode = mode
        self.hasDeviceIdentity = hasDeviceIdentity
        self.bridge = bridge
    }

    var isRunning: Bool { phase == .running || phase == .waiting }
    /// Exact, as typed: a mis-tap or a near miss cannot delete a client.
    var nameMatches: Bool { !name.isEmpty && confirmText == name }
    var canDelete: Bool {
        mode == .delete && hasDeviceIdentity && phase == .review && inventory != nil && nameMatches
    }
    var canExport: Bool { mode == .export && phase == .review && inventory != nil }

    func isTicked(_ item: ClientItem) -> Bool { ticked.contains(item.ref) }

    /// A group's box: ticked when every item in it is.
    func isTicked(_ items: [ClientItem]) -> Bool { !items.isEmpty && items.allSatisfy(isTicked) }

    func tickedCount(_ items: [ClientItem]) -> Int { items.filter(isTicked).count }

    /// Ticks or unticks every item of a group; the delete still gets exactly the refs of `ticked`.
    func setTicked(_ items: [ClientItem], _ on: Bool) {
        for item in items { setTicked(item, on) }
    }

    func setTicked(_ item: ClientItem, _ on: Bool) {
        guard phase == .review, inventory?.possible.contains(item) == true else { return }
        if on { ticked.insert(item.ref) } else { ticked.remove(item.ref) }
    }

    func load() async {
        if mode == .delete && !hasDeviceIdentity {
            phase = .noIdentity
            return
        }
        phase = .loading
        if await refresh() { phase = .review }
    }

    /// Reads the inventory again and keeps only the ticks it still lists. False when it could not (phase set).
    private func refresh() async -> Bool {
        switch await bridge.inventory(key: key) {
        case .success(let inv):
            inventory = inv
            let listed = Set(inv.possible.map(\.ref))
            ticked = ticked.intersection(listed)
            return true
        case .failure(.notFound):
            phase = .gone
            return false
        case .failure(let e):
            phase = .failed(e)
            return false
        }
    }

    func delete() async {
        guard canDelete else { return }
        confirmed = true
        await runDelete()
    }

    /// "Try again" after a partial receipt, a failure or `busy`: the selection is rebuilt from a fresh
    /// inventory first (core answers `selection_changed` to a tick it no longer lists).
    func retry() async {
        guard mode == .delete, hasDeviceIdentity else { return }
        switch phase {
        case .receipt(let r):
            guard ClientCopy.retryHelps(r), !retryGone else { return }
        case .failed(let e):
            guard ClientCopy.isRetryable(e) else { return }
        case .review:
            guard notice == .busy else { return }
        default:
            return
        }
        phase = .running
        switch await bridge.inventory(key: key) {
        case .success(let inv):
            inventory = inv
            ticked = ticked.intersection(Set(inv.possible.map(\.ref)))
        case .failure(.notFound):
            // The client record went in the partial run: what is left is listed under Left for you.
            if let last = lastReceipt {
                retryGone = true
                phase = .receipt(last)
            } else {
                phase = .gone
            }
            return
        case .failure(let e):
            phase = .failed(e)
            return
        }
        // Never a delete the user did not confirm (a failed first load also offers Try again).
        guard confirmed, nameMatches else {
            phase = .review
            return
        }
        await runDelete()
    }

    /// An export folder was unreachable (its drive is gone): run again, leaving the export folders to the user.
    func retryWithoutDeletingExports() async {
        deleteExports = false
        await retry()
    }

    /// The last receipt has an export folder Whimbrel could not reach.
    var exportFolderUnreachable: Bool {
        guard case .receipt(let r) = phase else { return false }
        return r.failed.contains(.exportFolder)
    }

    private var lastReceipt: ClientDeleteReceipt?
    /// The user pressed Delete with the name typed. A retry deletes only after that; otherwise it reloads the review.
    private var confirmed = false

    private static var nextToken: UInt64 = 1
    private var waitToken: UInt64 = 0
    /// How often the wait looks for background work still running.
    var waitPoll: Duration = .milliseconds(300)

    /// The user pressed Cancel (or closed the sheet) after the delete had started: it finished anyway, and the
    /// receipt says so.
    @Published private(set) var cancelTooLate = false

    /// "Cancel" from the moment the delete is confirmed (waiting, or queued for the slow lane): nothing is deleted
    /// and the review shows again. After the delete has started core says so, and the receipt says it was too late.
    func cancelWait() {
        guard mode == .delete, phase == .waiting || phase == .running else { return }
        if !bridge.cancelWait(token: waitToken) { cancelTooLate = true }
    }

    private func runDelete() async {
        notice = nil
        cancelTooLate = false
        phase = bridge.jobsRunning() > 0 ? .waiting : .running
        let picks = Array(ticked)
        let seen = inventory?.linked.map(\.ref) ?? []
        waitToken = Self.nextToken
        Self.nextToken += 1
        let poll = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: self?.waitPoll ?? .milliseconds(300))
                guard let self, !Task.isCancelled, self.phase == .running || self.phase == .waiting else { return }
                self.phase = self.bridge.jobsRunning() > 0 ? .waiting : .running
            }
        }
        let result = await bridge.delete(key: key, includePossible: picks, reviewedLinked: seen, deleteExports: deleteExports, token: waitToken)
        poll.cancel()
        switch result {
        case .failure(.cancelled):
            // Nothing was touched; the ticks and the typed name stay, but a confirm is needed again.
            confirmed = false
            cancelTooLate = false
            phase = .review
        case .success(let receipt):
            lastReceipt = receipt
            phase = .receipt(receipt)
        case .failure(.selectionChanged):
            // Nothing was touched. Show the list as it is now and ask again: the user confirms the new list.
            confirmed = false
            if await refresh() {
                notice = .selectionChanged
                phase = .review
            }
        case .failure(.busy):
            notice = .busy
            phase = .review
        case .failure(.noDeviceId):
            phase = .noIdentity
        case .failure(.notFound):
            phase = .gone
        case .failure(let e):
            phase = .failed(e)
        }
    }

    // MARK: Export

    /// Writes the export into a new folder inside `parent` (chosen by the user in the open panel), then copies the
    /// media with the store released.
    func export(to parent: URL) async {
        guard canExport else { return }
        phase = .running
        let picks = Array(ticked)
        switch await bridge.exportWrite(key: key, destParent: parent, includePossible: picks, includeMedia: includeMedia) {
        case .failure(.notFound):
            phase = .gone
        case .failure(let e):
            phase = .failed(e)
        case .success(let written):
            guard let token = written.mediaToken else {
                phase = .exported(ExportOutcome(folder: written.folder, media: nil, partial: false))
                return
            }
            switch await bridge.exportCopyMedia(token: token) {
            case .success(let done):
                phase = .exported(ExportOutcome(folder: done.folder, media: done.media, partial: done.partial))
            case .failure:
                // The text is written; the media copy did not run.
                phase = .exported(ExportOutcome(folder: written.folder, media: nil, partial: true))
            }
        }
    }

    /// Back to the review after an export failure the user can fix (another folder).
    func backToReview() {
        guard case .failed = phase, inventory != nil else { return }
        phase = .review
    }
}

// MARK: - Keep for N months: the weekly review

@MainActor
final class ClientRetentionReviewModel: ObservableObject, Identifiable {
    struct Pick: Hashable {
        let key: String
        let ref: ClientItemRef
    }

    struct Group: Identifiable, Equatable {
        let key: String
        let name: String
        var items: [ClientItem]
        var id: String { key }
    }

    enum Phase: Equatable {
        case loading, review, running
        case done(ClientExpirySummary)
        case failed(ClientOpError)
    }

    /// Snooze puts an item off for 30 days (spec 5).
    static let snoozeDays = 30
    /// Rounds of a budgeted expiry before the screen stops and says so.
    static let maxRounds = 50

    nonisolated let id = UUID()
    private let bridge: ClientControlsBridge
    private let names: [String: String]
    private let now: () -> Date

    @Published private(set) var phase: Phase = .loading
    @Published private(set) var groups: [Group] = []
    /// Ticked items are deleted on confirm; Keep and Snooze take an item off the list.
    @Published private(set) var ticked: Set<Pick> = []
    @Published private(set) var errorText: String?

    /// `names`: client key to name, for the headings (a client not in it is shown without a name).
    init(names: [String: String], bridge: ClientControlsBridge, now: @escaping () -> Date = Date.init) {
        self.names = names
        self.bridge = bridge
        self.now = now
    }

    var tickedCount: Int { ticked.count }

    func load() async {
        phase = .loading
        switch await bridge.dueItems() {
        case .success(let due):
            groups = due.filter { !$0.items.isEmpty }.map {
                Group(key: $0.key, name: names[$0.key] ?? String(localized: "A client"), items: $0.items)
            }
            ticked = Set(groups.flatMap { g in g.items.map { Pick(key: g.key, ref: $0.ref) } })
            phase = .review
        case .failure(let e):
            phase = .failed(e)
        }
    }

    func isTicked(key: String, _ item: ClientItem) -> Bool { ticked.contains(Pick(key: key, ref: item.ref)) }

    func setTicked(key: String, _ item: ClientItem, _ on: Bool) {
        let p = Pick(key: key, ref: item.ref)
        if on { ticked.insert(p) } else { ticked.remove(p) }
    }

    /// "Keep these": never asked again for this item, on this Mac.
    func keep(key: String, _ item: ClientItem) async {
        await decide(key: key, item, .keep)
    }

    func snooze(key: String, _ item: ClientItem) async {
        let until = now().addingTimeInterval(TimeInterval(Self.snoozeDays) * 86_400)
        await decide(key: key, item, .snooze(untilMs: Int64(until.timeIntervalSince1970 * 1000)))
    }

    private func decide(key: String, _ item: ClientItem, _ d: ClientRetentionDecision) async {
        switch await bridge.decide(key: key, item: item.ref, decision: d) {
        case .success:
            errorText = nil
            ticked.remove(Pick(key: key, ref: item.ref))
            if let i = groups.firstIndex(where: { $0.key == key }) {
                groups[i].items.removeAll { $0 == item }
                if groups[i].items.isEmpty { groups.remove(at: i) }
            }
        case .failure(let e):
            errorText = ClientCopy.message(e)
        }
    }

    /// Deletes the ticked items on this Mac, client by client, repeating each budgeted call while core says
    /// `more` (each round sends only the ticked items still due).
    func deleteTicked() async {
        guard phase == .review, !ticked.isEmpty else { return }
        phase = .running
        var summary = ClientExpirySummary()
        for group in groups {
            let wanted = Set(group.items.map(\.ref).filter { ticked.contains(Pick(key: group.key, ref: $0)) })
            guard !wanted.isEmpty else { continue }
            var items = group.items.map(\.ref).filter(wanted.contains)
            var rounds = 0
            while !items.isEmpty {
                rounds += 1
                switch await bridge.expire(key: group.key, items: items) {
                case .failure(let e):
                    summary.stopped = e
                    phase = .done(summary)
                    return
                case .success(let r):
                    summary.add(r)
                    guard r.more else {
                        summary.waitingToSync = ClientExpirySummary.merge(summary.waitingToSync, r.waitingToSync)
                        items = []
                        continue
                    }
                    guard rounds < Self.maxRounds else {
                        summary.unfinished = true
                        items = []
                        continue
                    }
                    // Only the ones still due go again; done ones would count as "no longer due".
                    switch await bridge.dueItems(key: group.key) {
                    case .success(let still):
                        items = still.map(\.ref).filter(wanted.contains)
                    case .failure(let e):
                        // Never "deleted" while ticked items may remain.
                        summary.stopped = e
                        phase = .done(summary)
                        return
                    }
                }
            }
        }
        phase = .done(summary)
    }

    /// "Later": the weekly prompt comes back in a week.
    func later() async {
        _ = await bridge.dismissReviewFlag()
    }
}

/// What a keep-for-N-months run did, summed over its rounds and clients.
struct ClientExpirySummary: Equatable {
    var counts: [ClientKindCount] = []
    var notDue: [ClientKindCount] = []
    var waitingToSync: [ClientKindCount] = []
    var failed: [ClientItemKind] = []
    var leftForYou: [ClientLeftForYou] = []
    /// It stopped after `maxRounds`: run it again.
    var unfinished = false
    var stopped: ClientOpError?

    mutating func add(_ r: ClientExpiryReceipt) {
        counts = Self.merge(counts, r.counts)
        notDue = Self.merge(notDue, r.notDue)
        for k in r.failed where !failed.contains(k) { failed.append(k) }
        for l in r.leftForYou where !leftForYou.contains(l) { leftForYou.append(l) }
    }

    static func merge(_ a: [ClientKindCount], _ b: [ClientKindCount]) -> [ClientKindCount] {
        var out = a
        for c in b {
            if let i = out.firstIndex(where: { $0.kind == c.kind }) {
                out[i] = ClientKindCount(kind: c.kind, count: out[i].count + c.count)
            } else {
                out.append(c)
            }
        }
        return out
    }

    var complete: Bool { failed.isEmpty && stopped == nil && !unfinished }
}

// MARK: - Settings → Devices: devices still to update

/// The core calls behind the Devices pane's update line.
@MainActor
protocol ClientPendingDevicesBridge: AnyObject {
    /// Devices that still need to update before they apply client deletes; nil when not known yet.
    func pendingDevices() async -> Int?
    /// Those devices' sync ids (public keys); nil when not known yet.
    func pendingDeviceIds() async -> [String]?
    /// "Remove this device": stops counting one of them, on every device after this Mac's next sync round. The
    /// count after it.
    func retireDevice(_ id: String) async -> Result<Int?, ClientOpError>
}

/// "N devices still need to update" under the sync status (spec 13.3), the count each sync round stores, with
/// each such device named and a "Remove this device" for one that is retired and will never update. Removing
/// one only stops counting it; it releases nothing (client deletes are never held for a device).
@MainActor
final class ClientPendingDevicesModel: ObservableObject {
    @Published private(set) var count: Int?
    /// The devices counted, by sync id. Empty when none or not known.
    @Published private(set) var ids: [String] = []
    /// The device the confirmation was opened for (kept while the alert closes, so its Remove button still has it).
    @Published private(set) var confirming: String?
    /// The confirmation alert is up.
    @Published var showingConfirmation = false
    @Published private(set) var errorText: String?
    @Published private(set) var isRemoving = false
    private let bridge: ClientPendingDevicesBridge

    init(bridge: ClientPendingDevicesBridge) {
        self.bridge = bridge
    }

    /// Nil for none and for not known yet.
    var line: String? { ClientCopy.syncPendingLine(count) }

    func load() async {
        count = await bridge.pendingDevices()
        ids = (count ?? 0) > 0 ? (await bridge.pendingDeviceIds() ?? []) : []
    }

    /// Opens the confirmation for a listed device.
    func ask(_ id: String) {
        guard ids.contains(id), !isRemoving else { return }
        errorText = nil
        confirming = id
        showingConfirmation = true
    }

    /// Cancel in the confirmation.
    func cancel() {
        confirming = nil
        showingConfirmation = false
    }

    /// The user pressed Remove in the confirmation: removes the device it was opened for, and only that one.
    @discardableResult
    func confirm() -> Task<Void, Never>? {
        guard let id = confirming, ids.contains(id), !isRemoving else { return nil }
        confirming = nil
        showingConfirmation = false
        isRemoving = true
        return Task { await self.retire(id) }
    }

    private func retire(_ id: String) async {
        defer { isRemoving = false }
        switch await bridge.retireDevice(id) {
        case .success(let n):
            count = n
            ids.removeAll { $0 == id }
            errorText = nil
        case .failure(.notFound):
            // Updated since, or already removed: read the list again.
            errorText = ClientCopy.deviceNoLongerListed
            await load()
        case .failure(let e):
            errorText = ClientCopy.message(e)
        }
    }
}

// MARK: - Row text

/// What a review row shows: the item's own label (client text, for this screen only) over its kind and date.
/// Kept apart from `ClientCopy`, whose lines go into receipts and must never hold client text.
/// Screen captures of one app (or one window title) on one day, shown as one row.
struct ClientCaptureGroup: Identifiable, Equatable {
    let id: String
    let items: [ClientItem]
    /// The app's name, else the window title of the first capture, else nil.
    let source: String?
}

/// A row of the review list: one item, or a group of captures.
enum ClientReviewRow: Identifiable, Equatable {
    case item(ClientItem)
    case group(ClientCaptureGroup)

    var id: String {
        switch self {
        case .item(let i): "\(i.kind.rawValue):\(i.id)"
        case .group(let g): "group:\(g.id)"
        }
    }
}

enum ClientRowText {
    /// The app's display name for a bundle id (the installed app's name), nil when it is not found.
    static func systemAppName(_ bundleId: String) -> String? {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) else { return nil }
        return FileManager.default.displayName(atPath: url.path).replacingOccurrences(of: ".app", with: "")
    }

    /// Captures with the same app (or, with no app, the same window title) on the same day, two or more, fold
    /// into one group at the place of the first of them; everything else stays a row. Order is kept.
    static func rows(_ items: [ClientItem], appName: (String) -> String? = systemAppName, calendar: Calendar = .current) -> [ClientReviewRow] {
        func key(_ i: ClientItem) -> String? {
            guard i.kind == .capture else { return nil }
            let who = i.app ?? i.label?.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let who, !who.isEmpty else { return nil }
            let day = calendar.startOfDay(for: TopicDates.date(ms: i.dateUnixMs)).timeIntervalSince1970
            return "\(i.app == nil ? "t" : "a"):\(who)|\(Int(day))"
        }
        var members: [String: [ClientItem]] = [:]
        for i in items { if let k = key(i) { members[k, default: []].append(i) } }
        var out: [ClientReviewRow] = []
        var placed: Set<String> = []
        for i in items {
            guard let k = key(i), let group = members[k], group.count > 1 else { out.append(.item(i)); continue }
            if placed.insert(k).inserted {
                let source = i.app.flatMap(appName) ?? group.compactMap { $0.label?.trimmingCharacters(in: .whitespacesAndNewlines) }.first { !$0.isEmpty }
                out.append(.group(ClientCaptureGroup(id: k, items: group, source: source)))
            }
        }
        return out
    }

    /// "Screen captures from Terminal, Wed, Nov 18: 14". The count is last so every language reads it.
    static func groupTitle(_ g: ClientCaptureGroup) -> String {
        let day = TopicDates.day(TopicDates.date(ms: g.items[0].dateUnixMs))
        let n = g.items.count
        guard let source = g.source else { return String(localized: "Screen captures on \(day): \(n)") }
        return String(localized: "Screen captures from \(source), \(day): \(n)")
    }

    /// "3 of 14 ticked" for a group that is partly ticked.
    static func groupProgress(ticked: Int, of n: Int) -> String {
        String(localized: "\(ticked) of \(n) ticked")
    }

    /// The label, or the kind and date when the item has none.
    static func title(_ item: ClientItem) -> String {
        guard let label = item.label?.trimmingCharacters(in: .whitespacesAndNewlines), !label.isEmpty else {
            return ClientCopy.item(item)
        }
        return label
    }

    /// The kind and date under a label; nil when the title already is the kind and date.
    static func detail(_ item: ClientItem) -> String? {
        title(item) == ClientCopy.item(item) ? nil : ClientCopy.item(item)
    }
}

// MARK: - Words

/// Every user-facing line of the client controls, built from counts and categories only (never client text).
enum ClientCopy {
    static func kind(_ k: ClientItemKind) -> String {
        switch k {
        case .meeting: String(localized: "Meetings")
        case .email: String(localized: "Emails")
        case .thread: String(localized: "Email threads")
        case .calendarEvent: String(localized: "Calendar events")
        case .capture: String(localized: "Screen captures")
        case .episode: String(localized: "Screen sessions")
        case .conversation: String(localized: "Ask conversations")
        case .step: String(localized: "Steps")
        case .vaultNote: String(localized: "Vault notes")
        case .exportFolder: String(localized: "Export folders")
        }
    }

    /// "Meetings: 3"
    static func count(_ c: ClientKindCount) -> String {
        String(localized: "\(kind(c.kind)): \(c.count)")
    }

    static func counts(_ list: [ClientKindCount]) -> [String] {
        list.filter { $0.count > 0 }.map(count)
    }

    static func item(_ item: ClientItem) -> String {
        let day = TopicDates.day(TopicDates.date(ms: item.dateUnixMs))
        return String(localized: "\(singular(item.kind)), \(day)")
    }

    static func singular(_ k: ClientItemKind) -> String {
        switch k {
        case .meeting: String(localized: "Meeting")
        case .email: String(localized: "Email")
        case .thread: String(localized: "Email thread")
        case .calendarEvent: String(localized: "Calendar event")
        case .capture: String(localized: "Screen capture")
        case .episode: String(localized: "Screen session")
        case .conversation: String(localized: "Ask conversation")
        case .step: String(localized: "Step")
        case .vaultNote: String(localized: "Vault note")
        case .exportFolder: String(localized: "Export folder")
        }
    }

    /// The receipt's devices line. Nil from core means not measured yet: never shown as 0.
    static func pendingDevices(_ n: Int?) -> String {
        switch n {
        case nil: String(localized: "Not known yet how many of your other devices still need to update to apply this delete. Sync works it out on a later round.")
        // Never "none": a device on an older version writes nothing the count can see (spec 13.3).
        case 0?: String(localized: "No device that has updated is still waiting to apply this delete. A device on an older version, or one you removed from the count in Settings, may not have applied it.")
        case 1?: String(localized: "1 device still needs to update before it applies this delete.")
        case let n?: String(localized: "\(n) devices still need to update before they apply this delete.")
        }
    }

    /// Settings → Devices: nil for none, and for not known yet (never said as 0).
    static func syncPendingLine(_ n: Int?) -> String? {
        switch n {
        case nil, 0?: nil
        case 1?: String(localized: "1 device still needs to update before it applies client deletes.")
        case let n?: n < 0 ? nil : String(localized: "\(n) devices still need to update before they apply client deletes.")
        }
    }

    static let removeDevice = String(localized: "Remove this device…")
    static let removeDeviceTitle = String(localized: "Remove this device from the count?")
    static let removeDeviceMessage = String(localized: "Only do this for a device you no longer use. Your devices stop counting it as needing an update. It keeps its access to sync: to end that too, revoke it in the device list. If it is used again before it updates, it will not apply client deletes, and nothing here will say so.")
    static let deviceNoLongerListed = String(localized: "That device is no longer on the list. The list was read again.")

    static let reappears = String(localized: "A client can reappear as a new topic if new mail or a new meeting matches it.")
    /// True on this Mac only: other devices keep their derived copies until their own retention (Left for you says so).
    static let derived = String(localized: "On this Mac, summaries, suggestions, people and organisations that Whimbrel made and that name this client are deleted too.")
    /// The matcher's limit (final review, Major 5; ruled: stated, matcher unchanged).
    static let matchingLimit = String(localized: "Matching is on whole words of the client's name, aliases and known contact addresses. A name inside another word or in text written without spaces (some inflected forms, some scripts) may not match, and that material can still go to a hosted model.")
    /// A clean-up that failed after the client record went: it can't be run again from the receipt.
    static let cleanupNotFinished = String(localized: "The client is deleted, but the last clean-up step did not finish, so words from the deleted material can stay in Whimbrel's search index on this Mac. The client record is already gone, so this delete can't be run again. Whimbrel runs the clean-up again the next time you delete a client.")
    static let expiryNoIdentity = String(localized: "Not deleted: sync is on, but this Mac has no device identity, so Whimbrel can't tell which of these items your other devices already have. Quit and reopen Whimbrel to try again.")

    /// Whether "Try again" can do anything for this receipt: only while the client record is still here (core's
    /// `retry_helps` can't know that a retry then finds no client, after `removed` or `removedHereOnly`).
    static func retryHelps(_ r: ClientDeleteReceipt) -> Bool { r.retryHelps && r.client == .stillPresent }

    /// Why a keep-for-N-months run stopped. `noDeviceId` has its own words: it is about which items synced.
    static func expiryStopped(_ e: ClientOpError) -> String {
        e == .noDeviceId ? expiryNoIdentity : message(e)
    }

    /// The Exclude toggle's warning (final review, Major 22): while a device without the topics marker is on the
    /// relay, every client's settings row is held on this Mac, so the others never learn of the exclusion. Removing
    /// a device from the count does not release it. Nil for none and for not known (never said as 0).
    static func heldDevices(_ n: Int?) -> String? {
        switch n {
        case nil: nil
        case let n? where n <= 0: nil
        case 1?: String(localized: "1 device on your sync hasn't updated yet. Until it does, this setting stays on this Mac: your other devices don't receive it, and they could still send this client's material to a hosted model. Update that device. Until then, set it on each other device where this client appears; a client added here may not appear there yet. Removing it from the count in Settings doesn't change this.")
        case let n?: String(localized: "\(n) devices on your sync haven't updated yet. Until they do, this setting stays on this Mac: your other devices don't receive it, and they could still send this client's material to a hosted model. Update those devices. Until then, set it on each other device where this client appears; a client added here may not appear there yet. Removing them from the count in Settings doesn't change this.")
        }
    }

    /// Under an answer a hosted model gave without some retrieved items (core's `filtered_hosted`; items, not clients).
    static func leftOut(_ n: Int) -> String? {
        switch n {
        case ...0: nil
        case 1: String(localized: "Left out 1 item that involves an excluded client.")
        default: String(localized: "Left out \(n) items that involve an excluded client.")
        }
    }
    /// The hotkey panel's stream was held back by the gate (the line had said hosted): nothing was sent.
    static let hotkeyGateRefused = String(localized: "Not sent: this would have included material about a client you excluded from hosted models. Nothing went to the hosted model.")
    static let gmailThreads = String(localized: "A Gmail thread summary may keep text from mail in that thread that already expired from Whimbrel.")
    /// X4 design 3.5 step 8: the receipt's line when the delete also removed the encryption upgrade's copy.
    static let rollbackCopyRemoved = String(localized: "The unencrypted copy of the memory database kept from the encryption upgrade was removed too.")

    static func receiptHeadline(_ r: ClientDeleteReceipt) -> String {
        switch r.client {
        case .removed where r.complete:
            String(localized: "Deleted this client and the items you chose. Check the places listed under Left for you.")
        case .removedHereOnly:
            String(localized: "Deleted on this Mac only. Your other devices were not told, because this Mac has no sync identity.")
        // The client record is gone, so a retry finds nothing to run.
        case .removed where r.failedStages.contains(.cleanup):
            cleanupNotFinished
        case .removed:
            String(localized: "The client is deleted, but some items could not be removed. They are listed below.")
        case .stillPresent where r.retryHelps:
            String(localized: "Not finished: some of this client's material is still here. Try again to finish.")
        case .stillPresent:
            String(localized: "Not finished: some of this client's material is still here. It is listed below.")
        }
    }

    /// Items or steps that failed, as kind names; nil when nothing failed.
    static func failedLine(_ r: ClientDeleteReceipt) -> String? {
        let kinds = r.failed.map(kind)
        let stages = r.failedStages.filter { $0 != .noDeviceId }.map(stage)
        let all = kinds + stages
        guard !all.isEmpty else { return nil }
        return String(localized: "Not removed: \(all.joined(separator: ", ")).")
    }

    static func stage(_ s: ClientDeleteStage) -> String {
        switch s {
        case .derivedText: String(localized: "text Whimbrel made from this material")
        case .clientRecord: String(localized: "the client record")
        case .cleanup: String(localized: "clean-up")
        case .files: String(localized: "files")
        case .noDeviceId: String(localized: "telling your other devices")
        }
    }

    /// One "Left for you" entry: a title and the paths it names (shown as is).
    struct LeftLine: Equatable, Hashable {
        let text: String
        let paths: [String]
    }

    static func leftForYou(_ l: ClientLeftForYou) -> LeftLine? {
        switch l {
        case .mediaFolder(let p):
            LeftLine(text: String(localized: "The media folder. Whimbrel removes this client's audio and slide images from it; copies that a backup or cloud sync made of it stay."), paths: [p])
        case .vaultFolder(let p):
            LeftLine(text: String(localized: "Your Obsidian vault. Whimbrel removes this client's notes from it; copies that a backup or sync made stay."), paths: [p])
        case .exportFolders(let ps):
            LeftLine(text: String(localized: "Earlier exports of this client. They are plain copies, yours to keep or delete. An export you moved or renamed is not listed here: delete it yourself."), paths: ps)
        case .mailboxOriginals:
            LeftLine(text: String(localized: "The original emails stay in your mailbox. Delete them there if you need to."), paths: [])
        case .backupsAndCloud:
            LeftLine(text: String(localized: "Time Machine and cloud backups (iCloud, Dropbox and others) can hold copies of files, the vault and exports. Whimbrel cannot reach them; they are yours to handle. Files are often the hardest part, because backups keep copies."), paths: [])
        case .relayCopiesBeforeThisRelease:
            LeftLine(text: String(localized: "Copies your devices sent to the sync relay before this version stay there, encrypted. Current devices ignore them."), paths: [])
        case .filesNotRemoved(let ps):
            LeftLine(text: String(localized: "Files Whimbrel could not remove:"), paths: ps)
        case .copiesOnOtherDevices:
            LeftLine(text: String(localized: "Your other devices: most copies go when each device syncs and applies the delete. Routine results, profile suggestions, people's names and meeting briefs that named this client can stay on each device: the delete does not remove them there."), paths: [])
        case .expiredHereOnly(1):
            LeftLine(text: String(localized: "1 item was removed earlier on this Mac only by Keep for N months. Your other devices may still hold it."), paths: [])
        case .expiredHereOnly(let n):
            LeftLine(text: String(localized: "\(n) items were removed earlier on this Mac only by Keep for N months. Your other devices may still hold them."), paths: [])
        case .vaultSyncOff:
            LeftLine(text: String(localized: "Notes in your vault are not touched while vault sync is off. Delete them yourself if they name this client."), paths: [])
        case .other:
            nil
        }
    }

    /// One "Left for you" entry on a keep-for-N-months result. An expiry never reaches other devices, so the
    /// delete's other-devices line would be false there.
    static func expiryLeftForYou(_ l: ClientLeftForYou) -> LeftLine? {
        if case .copiesOnOtherDevices = l { return LeftLine(text: expiryOtherDevices, paths: []) }
        return leftForYou(l)
    }

    static let expiryOtherDevices = String(localized: "This ran on this Mac only. Your other devices keep their copies until each one expires them or you delete them there.")

    static func message(_ e: ClientOpError) -> String {
        switch e {
        case .notFound: String(localized: "This client is no longer here.")
        case .noDeviceId: noIdentity
        case .invalidInput: String(localized: "Whimbrel couldn't read the request. Nothing was changed.")
        case .selectionChanged: String(localized: "The list changed while you were reviewing it. Nothing was deleted. Check it and confirm again.")
        case .busy: String(localized: "Background work didn't finish in time. Nothing was deleted. Try again in a moment.")
        case .cancelled: String(localized: "Cancelled. Nothing was deleted.")
        case .folderNotAllowed: String(localized: "Choose a folder on this Mac, not a network share. Nothing was written.")
        case .failed: String(localized: "Whimbrel couldn't read or write its data or a folder. Try again.")
        }
    }

    static let noIdentity = String(localized: "Whimbrel can't delete a client on this Mac: it couldn't read this Mac's device identity from the Keychain, so your other devices would never hear of the delete. Quit and reopen Whimbrel to try again.")

    static func isRetryable(_ e: ClientOpError) -> Bool {
        switch e {
        case .busy, .failed: true
        case .notFound, .noDeviceId, .invalidInput, .selectionChanged, .cancelled, .folderNotAllowed: false
        }
    }

    static func keepLabel(_ months: Int?) -> String {
        guard let months else { return String(localized: "Off") }
        if months == 1 { return String(localized: "1 month") }
        return String(localized: "\(months) months")
    }
}

// MARK: - The real bridge

/// Core calls for the client controls. Reads on `accessQueue` with the main store; delete, expire and the
/// export's text on `slowLaneQueue` with the slow-lane store (core requires it); the export's media copy on a
/// background queue with no store.
@MainActor
final class CoreClientControlsBridge: ClientControlsBridge, ClientPendingDevicesBridge {
    /// Every item in a keep-for-N-months expiry call: core stops starting new items after this.
    nonisolated static let expireBudgetMs: UInt64 = 5_000

    private let store: CoreMemoryStore
    private let slow: CoreMemoryStore?
    private let deviceId: String
    private let resummarizeAction: ([Int64]) -> Void

    init(store: CoreMemoryStore, slow: CoreMemoryStore?, deviceId: String, resummarize: @escaping ([Int64]) -> Void) {
        self.store = store
        self.slow = slow
        self.deviceId = deviceId
        self.resummarizeAction = resummarize
    }

    /// The folders core may remove this client's files from: the media folder first (named under "Left for
    /// you"), then the data folder (recordings not moved yet); and the Obsidian vault, only while "Keep an
    /// Obsidian vault up to date" is on (Mike: vault notes are deleted only while vault sync is on). With it off and
    /// a vault chosen before, no vault is passed and `vaultSyncOff` makes "Left for you" say the vault was not touched.
    nonisolated static func folders(defaults: UserDefaults = .standard) -> (media: [URL], vault: URL?, vaultSyncOff: Bool) {
        let media = [MediaSettings.currentFolderURL(), SpikePaths.root]
        let path = defaults.string(forKey: VaultSettings.pathKey)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !path.isEmpty else { return (media, nil, false) }
        guard defaults.bool(forKey: VaultSettings.enabledKey) else { return (media, nil, true) }
        return (media, URL(fileURLWithPath: path, isDirectory: true), false)
    }

    /// False for none and for the placeholder `AppState` uses when this Mac has no sync identity (core treats it
    /// as no id too).
    nonisolated static func isRealDeviceId(_ id: String) -> Bool {
        let id = id.trimmingCharacters(in: .whitespaces)
        return !id.isEmpty && id != "spike-device"
    }

    /// The sync group key when sync is set up here, else nil (core then cleans up only this Mac's own rows).
    /// A Keychain error is not "no sync": the call is refused rather than run as if sync were off.
    nonisolated private static func groupKey() -> Result<String?, ClientOpError> {
        do {
            let key = try SyncKeychain.loadGroupKey()
            return .success(key.flatMap { $0.isEmpty ? nil : $0 })
        } catch {
            return .failure(.failed)
        }
    }

    private func onAccess<T>(_ body: @escaping (CoreMemoryStore) -> T) async -> T {
        let store = self.store
        return await withCheckedContinuation { c in CoreMemoryStore.accessQueue.async { c.resume(returning: body(store)) } }
    }

    private func onSlowLane<T>(_ unavailable: T, _ body: @escaping (CoreMemoryStore) -> T) async -> T {
        guard let slow else { return unavailable }
        return await withCheckedContinuation { c in CoreMemoryStore.slowLaneQueue.async { c.resume(returning: body(slow)) } }
    }

    func inventory(key: String) async -> Result<ClientInventory, ClientOpError> {
        let f = Self.folders()
        return await onAccess { $0.clientInventory(key: key, mediaRoots: f.media, vault: f.vault, vaultSyncOff: f.vaultSyncOff) }
    }

    func jobsRunning() -> Int { CoreMemoryStore.clientJobsRunning() }

    func cancelWait(token: UInt64) -> Bool { CoreMemoryStore.clientCancelWait(token: token) }

    /// A plain queue that holds no store: the wait for running ticks happens here, so the slow lane (Ask, the
    /// post-meeting summary, compaction) is not held up behind it.
    nonisolated private static let waitQueue = DispatchQueue(label: "com.whimbrel.client-delete-wait", qos: .userInitiated)

    func delete(key: String, includePossible: [ClientItemRef], reviewedLinked: [ClientItemRef], deleteExports: Bool, token: UInt64) async -> Result<ClientDeleteReceipt, ClientOpError> {
        let f = Self.folders()
        let deviceId = self.deviceId
        // Step 1, off the slow lane: wait for running ticks (the pause is held under the token).
        let held = await withCheckedContinuation { c in
            Self.waitQueue.async { c.resume(returning: CoreMemoryStore.clientPauseWait(token: token)) }
        }
        guard held else { return .failure(.cancelled) }
        guard slow != nil, case .success(let groupKey) = Self.groupKey() else {
            CoreMemoryStore.clientReleasePause(token: token)
            return .failure(.failed)
        }
        // Step 2, on the slow lane, only once nothing is running; the pause is released when it ends.
        return await onSlowLane(.failure(.failed)) { slow in
            slow.clientDeleteHeld(key: key, includePossible: includePossible, reviewedLinked: reviewedLinked, deleteExports: deleteExports,
                                  mediaRoots: f.media, vault: f.vault, vaultSyncOff: f.vaultSyncOff, deviceId: deviceId,
                                  groupKeyBase64: groupKey, token: token)
        }
    }

    func control(key: String) async -> Result<ClientControl, ClientOpError> {
        await onAccess { $0.clientControl(key: key) }
    }

    func updateControl(key: String, _ change: @escaping (ClientControl) -> ClientControl) async -> Result<ClientControlUpdate, ClientOpError> {
        await onAccess { store in
            switch store.clientControl(key: key) {
            case .failure(let e):
                return .failure(e)
            case .success(let current):
                let next = change(current)
                guard next != current else { return .success(ClientControlUpdate(before: current, after: current)) }
                return store.setClientControl(key: key, next).map { ClientControlUpdate(before: current, after: $0) }
            }
        }
    }

    func resummaryOffer(key: String) async -> Result<ClientResummaryOffer, ClientOpError> {
        await onAccess { $0.clientResummaryOffer(key: key) }
    }

    func resummarize(meetings: [Int64]) { resummarizeAction(meetings) }

    func pendingDevices() async -> Int? {
        await onAccess { $0.clientPendingDevices() }
    }

    func pendingDeviceIds() async -> [String]? {
        await onAccess { $0.clientPendingDeviceIds() }
    }

    func retireDevice(_ id: String) async -> Result<Int?, ClientOpError> {
        await onAccess { $0.clientRetireDevice(id) }
    }

    func reviewFlag() async -> Result<ClientReviewFlag?, ClientOpError> {
        await onAccess { $0.clientReviewFlag() }
    }

    func dismissReviewFlag() async -> Result<Void, ClientOpError> {
        await onAccess { $0.clientReviewFlagDismiss() }
    }

    func dueItems() async -> Result<[ClientDue], ClientOpError> {
        await onAccess { $0.clientDueItems() }
    }

    func dueItems(key: String) async -> Result<[ClientItem], ClientOpError> {
        await onAccess { $0.clientDueItems(key: key) }
    }

    func decide(key: String, item: ClientItemRef, decision: ClientRetentionDecision) async -> Result<Void, ClientOpError> {
        await onAccess { $0.clientRetentionDecide(key: key, item: item, decision: decision) }
    }

    func expire(key: String, items: [ClientItemRef]) async -> Result<ClientExpiryReceipt, ClientOpError> {
        let f = Self.folders()
        let deviceId = self.deviceId
        return await onSlowLane(.failure(.failed)) { slow in
            guard case .success(let groupKey) = Self.groupKey() else { return .failure(.failed) }
            // The same handle the expiry runs on, as the delete does: with sync on and no id core refuses the
            // expiry (`no_device_id`), and with one it can tell this Mac's unpushed items from the rest.
            if Self.isRealDeviceId(deviceId) { _ = slow.setDeviceId(deviceId) }
            return slow.clientExpire(key: key, items: items, mediaRoots: f.media, vault: f.vault, vaultSyncOff: f.vaultSyncOff,
                                     syncOn: groupKey != nil, budgetMs: Self.expireBudgetMs)
        }
    }

    func exportWrite(key: String, destParent: URL, includePossible: [ClientItemRef], includeMedia: Bool) async -> Result<ClientExportWritten, ClientOpError> {
        let f = Self.folders()
        return await onSlowLane(.failure(.failed)) { slow in
            slow.clientExportWrite(key: key, destParent: destParent, includePossible: includePossible,
                                   includeMedia: includeMedia, mediaRoots: f.media)
        }
    }

    func heldDevices() async -> Int? {
        await onAccess { $0.clientTopicRowsHeldDevices() }
    }

    func exportCopyMedia(token: UInt64) async -> Result<ClientExportDone, ClientOpError> {
        await withCheckedContinuation { c in
            DispatchQueue.global(qos: .userInitiated).async { c.resume(returning: CoreMemoryStore.clientExportCopyMedia(token: token)) }
        }
    }
}
