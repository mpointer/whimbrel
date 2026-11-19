import XCTest
@testable import Whimbrel

/// Scripted client-control calls. `holdDelete` suspends a delete until the test releases it.
@MainActor
private final class FakeClientBridge: ClientControlsBridge {
    var inventories: [Result<ClientInventory, ClientOpError>] = []
    var deletes: [Result<ClientDeleteReceipt, ClientOpError>] = []
    var holdDelete = false
    private var heldDelete: CheckedContinuation<Void, Never>?
    private(set) var deleteCalls: [(picks: Set<ClientItemRef>, deleteExports: Bool)] = []
    private(set) var inventoryCalls = 0

    var control = ClientControl(excludeHosted: false, keepMonths: nil)
    var controlError: ClientOpError?
    private(set) var setCalls: [ClientControl] = []
    var offer: Result<ClientResummaryOffer, ClientOpError> = .success(
        ClientResummaryOffer(meetings: [11, 12], alreadyLeftNote: "Material that already left cannot be recalled."))
    private(set) var resummarized: [[Int64]] = []

    var due: [ClientDue] = []
    var expires: [Result<ClientExpiryReceipt, ClientOpError>] = []
    private(set) var expireCalls: [[ClientItemRef]] = []
    private(set) var decisions: [(ClientItemRef, ClientRetentionDecision)] = []
    private(set) var dismissed = 0

    var written: Result<ClientExportWritten, ClientOpError> = .success(ClientExportWritten(folder: "/x/Whimbrel client export", mediaToken: nil))
    var copied: Result<ClientExportDone, ClientOpError> = .failure(.notFound)
    private(set) var exportCalls: [(URL, Bool)] = []

    func inventory(key: String) async -> Result<ClientInventory, ClientOpError> {
        inventoryCalls += 1
        return inventories.count > 1 ? inventories.removeFirst() : (inventories.first ?? .failure(.failed))
    }

    /// Background ticks "still running"; the model polls it while a delete waits.
    var jobs = 0
    private(set) var cancelledTokens: [UInt64] = []
    private var cancelled = false

    func jobsRunning() -> Int { jobs }

    /// False once core has started the delete: a cancel is then too late and the delete runs on.
    var cancelAccepted = true
    private(set) var reviewedLinked: [ClientItemRef] = []

    func cancelWait(token: UInt64) -> Bool {
        cancelledTokens.append(token)
        guard cancelAccepted else { return false }
        cancelled = true
        releaseDelete()
        return true
    }

    func delete(key: String, includePossible: [ClientItemRef], reviewedLinked: [ClientItemRef], deleteExports: Bool, token: UInt64) async -> Result<ClientDeleteReceipt, ClientOpError> {
        self.reviewedLinked = reviewedLinked
        deleteCalls.append((Set(includePossible), deleteExports))
        if holdDelete { await withCheckedContinuation { heldDelete = $0 } }
        if cancelled { cancelled = false; return .failure(.cancelled) }
        return deletes.isEmpty ? .failure(.failed) : deletes.removeFirst()
    }

    func releaseDelete() {
        heldDelete?.resume()
        heldDelete = nil
    }

    func control(key: String) async -> Result<ClientControl, ClientOpError> {
        if let controlError { return .failure(controlError) }
        return .success(control)
    }

    /// Suspends the next update until `releaseUpdate`; `updateError` makes it fail.
    var holdUpdate = false
    var updateError: ClientOpError?
    private var heldUpdate: CheckedContinuation<Void, Never>?
    func releaseUpdate() {
        heldUpdate?.resume()
        heldUpdate = nil
    }

    func updateControl(key: String, _ change: @escaping (ClientControl) -> ClientControl) async -> Result<ClientControlUpdate, ClientOpError> {
        if holdUpdate { await withCheckedContinuation { heldUpdate = $0 } }
        if let updateError { return .failure(updateError) }
        let before = control
        let c = change(before)
        setCalls.append(c)
        control = c
        return .success(ClientControlUpdate(before: before, after: c))
    }

    func resummaryOffer(key: String) async -> Result<ClientResummaryOffer, ClientOpError> { offer }
    func resummarize(meetings: [Int64]) { resummarized.append(meetings) }
    func reviewFlag() async -> Result<ClientReviewFlag?, ClientOpError> { .success(nil) }
    func dismissReviewFlag() async -> Result<Void, ClientOpError> { dismissed += 1; return .success(()) }
    func dueItems() async -> Result<[ClientDue], ClientOpError> { .success(due) }
    var dueForKeyError: ClientOpError?
    func dueItems(key: String) async -> Result<[ClientItem], ClientOpError> {
        if let dueForKeyError { return .failure(dueForKeyError) }
        return .success(due.first { $0.key == key }?.items ?? [])
    }

    func decide(key: String, item: ClientItemRef, decision: ClientRetentionDecision) async -> Result<Void, ClientOpError> {
        decisions.append((item, decision))
        return .success(())
    }

    func expire(key: String, items: [ClientItemRef]) async -> Result<ClientExpiryReceipt, ClientOpError> {
        expireCalls.append(items)
        return expires.isEmpty ? .failure(.failed) : expires.removeFirst()
    }

    func exportWrite(key: String, destParent: URL, includePossible: [ClientItemRef], includeMedia: Bool) async -> Result<ClientExportWritten, ClientOpError> {
        exportCalls.append((destParent, includeMedia))
        return written
    }

    func exportCopyMedia(token: UInt64) async -> Result<ClientExportDone, ClientOpError> { copied }

    var held: Int?
    func heldDevices() async -> Int? { held }
}

@MainActor
private final class FakePendingBridge: ClientPendingDevicesBridge {
    var count: Int?
    var ids: [String]?
    var retireResult: Result<Int?, ClientOpError>?
    private(set) var retired: [String] = []
    func pendingDevices() async -> Int? { count }
    func pendingDeviceIds() async -> [String]? { ids }
    func retireDevice(_ id: String) async -> Result<Int?, ClientOpError> {
        retired.append(id)
        if let retireResult { return retireResult }
        ids?.removeAll { $0 == id }
        count = ids?.count
        return .success(count)
    }
}

private func item(_ kind: ClientItemKind, _ id: String) -> ClientItem {
    ClientItem(kind: kind, id: id, alsoClient: nil, dateUnixMs: 1_700_000_000_000, removed: false)
}

private func inventory(linked: [ClientItem] = [item(.meeting, "1")], possible: [ClientItem] = [item(.email, "e1"), item(.conversation, "c1")]) -> ClientInventory {
    ClientInventory(linked: linked, possible: possible, possibleTruncated: false,
                    counts: [ClientKindCount(kind: .meeting, count: linked.count)], leftForYou: [.mailboxOriginals, .backupsAndCloud])
}

private func receipt(client: ClientRecordState = .removed, pending: Int? = 2, failed: [ClientItemKind] = [], complete: Bool = true, retryHelps: Bool = false) -> ClientDeleteReceipt {
    ClientDeleteReceipt(dateUnixMs: 1, counts: [ClientKindCount(kind: .meeting, count: 1)], notSent: [], localOnly: [],
                        client: client, leftForYou: [.mailboxOriginals], pendingDevices: pending, failed: failed,
                        failedStages: [], complete: complete, retryHelps: retryHelps)
}

private func expiry(deleted: Int, more: Bool) -> ClientExpiryReceipt {
    ClientExpiryReceipt(dateUnixMs: 1, counts: [ClientKindCount(kind: .email, count: deleted)], notDue: [], waitingToSync: [],
                        more: more, failed: [], failedStages: [], leftForYou: [], complete: !more)
}

@MainActor
final class ClientControlsModelTests: XCTestCase {
    private func review(_ bridge: FakeClientBridge, identity: Bool = true, mode: ClientReviewModel.Mode = .delete) async -> ClientReviewModel {
        let m = ClientReviewModel(key: "acme", name: "Acme Ltd", mode: mode, hasDeviceIdentity: identity, bridge: bridge)
        await m.load()
        return m
    }

    // MARK: Brief tests

    func testConfirmRequiresExactName() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        let m = await review(bridge)
        XCTAssertEqual(m.phase, .review)
        XCTAssertFalse(m.canDelete)
        for wrong in ["acme ltd", "Acme", "Acme Ltd ", " Acme Ltd", "ACME LTD"] {
            m.confirmText = wrong
            XCTAssertFalse(m.canDelete, wrong)
        }
        m.confirmText = "Acme Ltd"
        XCTAssertTrue(m.canDelete)
        // A press with the wrong name deletes nothing.
        m.confirmText = "Acme"
        await m.delete()
        XCTAssertTrue(bridge.deleteCalls.isEmpty)
    }

    func testPossibleMatchesStartUnticked() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(receipt())]
        let m = await review(bridge)
        XCTAssertTrue(m.ticked.isEmpty)
        XCTAssertFalse(m.inventory!.possible.contains { m.isTicked($0) })
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(bridge.deleteCalls.first?.picks, [], "an unticked possible match is never sent")
        XCTAssertEqual(bridge.deleteCalls.first?.deleteExports, false, "export folders stay unless ticked")
    }

    func testReceiptShowsPendingDevices() {
        XCTAssertEqual(ClientCopy.pendingDevices(2), "2 devices still need to update before they apply this delete.")
        XCTAssertEqual(ClientCopy.pendingDevices(1), "1 device still needs to update before it applies this delete.")
        // A device on an older version writes nothing the count sees: 0 is never "none of your devices".
        XCTAssertTrue(ClientCopy.pendingDevices(0).hasPrefix("No device that has updated is still waiting"))
        XCTAssertTrue(ClientCopy.pendingDevices(0).contains("older version"))
        // Unknown is said as unknown, never as 0.
        let unknown = ClientCopy.pendingDevices(nil)
        XCTAssertTrue(unknown.hasPrefix("Not known yet"), unknown)
        XCTAssertFalse(unknown.contains("0"))
    }

    func testEnablingExclusionShowsAlreadyLeftSentence() async {
        let bridge = FakeClientBridge()
        let m = ClientSettingsModel(key: "acme", bridge: bridge)
        await m.load()
        XCTAssertNil(m.exclusionNotice)
        await m.setExcludeHosted(true)
        XCTAssertTrue(m.excludeHosted)
        XCTAssertEqual(m.exclusionNotice?.alreadyLeft, "Material that already left cannot be recalled.")
        XCTAssertEqual(m.exclusionNotice?.hostedSummaries, [11, 12])
        m.resummarize()
        XCTAssertEqual(bridge.resummarized, [[11, 12]])
        m.resummarize()
        XCTAssertEqual(bridge.resummarized.count, 1, "offered once")
        await m.setExcludeHosted(false)
        XCTAssertNil(m.exclusionNotice)
    }

    /// Spins until `ok` holds (the model polls on a short timer in these tests).
    private func until(_ what: String, _ ok: () -> Bool) async {
        for _ in 0..<2_000 where !ok() { try? await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(ok(), what)
    }

    func testDeleteWaitsForBackgroundWorkThenRunsAndShowsTheReceipt() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(receipt())]
        bridge.holdDelete = true
        bridge.jobs = 2
        let m = await review(bridge)
        m.waitPoll = .milliseconds(10)
        m.confirmText = "Acme Ltd"
        let run = Task { await m.delete() }
        await until("waiting") { m.phase == .waiting }
        XCTAssertTrue(m.isRunning, "the sheet cannot be dismissed while it waits")
        XCTAssertFalse(m.canDelete)
        bridge.jobs = 0
        await until("running") { m.phase == .running }
        bridge.releaseDelete()
        await run.value
        XCTAssertEqual(m.phase, .receipt(receipt()))
    }

    func testCancelWhileWaitingDeletesNothingAndReturnsToTheReviewWithTheTicks() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.holdDelete = true
        bridge.jobs = 1
        let m = await review(bridge)
        m.waitPoll = .milliseconds(10)
        m.setTicked(item(.email, "e1"), true)
        m.confirmText = "Acme Ltd"
        let run = Task { await m.delete() }
        await until("waiting") { m.phase == .waiting }
        m.cancelWait()
        await run.value
        XCTAssertEqual(bridge.cancelledTokens.count, 1)
        XCTAssertEqual(m.phase, .review)
        XCTAssertNil(m.notice)
        XCTAssertTrue(m.isTicked(item(.email, "e1")), "selections are kept")
        XCTAssertEqual(m.confirmText, "Acme Ltd")
        XCTAssertTrue(m.canDelete, "it can be confirmed again")
        m.cancelWait()
        XCTAssertEqual(bridge.cancelledTokens.count, 1, "cancel does nothing outside a wait")
    }

    func testCancelIsAvailableWhileQueuedAndRunningNotOnlyWhileWaiting() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.holdDelete = true
        bridge.jobs = 0
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        let run = Task { await m.delete() }
        await until("running") { m.phase == .running }
        m.cancelWait()
        await run.value
        XCTAssertEqual(bridge.cancelledTokens.count, 1)
        XCTAssertEqual(m.phase, .review)
        XCTAssertFalse(m.cancelTooLate)
    }

    func testACancelAfterTheDeleteStartedSaysTooLateAndShowsTheReceipt() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(receipt())]
        bridge.holdDelete = true
        bridge.cancelAccepted = false
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        let run = Task { await m.delete() }
        await until("running") { m.phase == .running }
        m.cancelWait()
        XCTAssertTrue(m.cancelTooLate)
        bridge.releaseDelete()
        await run.value
        XCTAssertEqual(m.phase, .receipt(receipt()))
        XCTAssertTrue(m.cancelTooLate, "the receipt screen says so")
    }

    func testTheDeleteSendsTheLinkedSetTheUserReviewed() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory(linked: [item(.meeting, "1"), item(.email, "e9")]))]
        bridge.deletes = [.success(receipt())]
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(Set(bridge.reviewedLinked), [ClientItemRef(kind: .meeting, id: "1"), ClientItemRef(kind: .email, id: "e9")])
    }

    func testTheWaitForRunningTicksHoldsNoStoreAndComesBeforeTheSlowLane() throws {
        // The real bridge cannot be built in the test host: hold its shape in the source. Step 1 runs on a plain
        // queue and needs no store; only then does the delete take the slow lane.
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/ClientControlsModel.swift")
        let src = try String(contentsOf: url, encoding: .utf8)
        let start = try XCTUnwrap(src.range(of: "reviewedLinked: [ClientItemRef], deleteExports: Bool, token: UInt64) async -> Result<ClientDeleteReceipt, ClientOpError> {\n        let f = Self.folders()"))
        let body = String(src[start.lowerBound...].prefix(1800))
        let pause = try XCTUnwrap(body.range(of: "clientPauseWait"))
        let lane = try XCTUnwrap(body.range(of: "onSlowLane"))
        XCTAssertLessThan(pause.lowerBound, lane.lowerBound, "wait first, slow lane second")
        let waitBlock = String(body[..<lane.lowerBound])
        XCTAssertTrue(waitBlock.contains("waitQueue"))
        XCTAssertFalse(waitBlock.contains("slowLaneQueue") || waitBlock.contains("onSlowLane") || waitBlock.contains("onAccess"))
    }

    func testDeleteButtonDisabledWhileRunning() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(receipt())]
        bridge.holdDelete = true
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        XCTAssertTrue(m.canDelete)
        let run = Task { await m.delete() }
        var spins = 0
        while bridge.deleteCalls.isEmpty {
            spins += 1
            guard spins < 10_000 else { return XCTFail("the delete never started") }
            await Task.yield()
        }
        XCTAssertTrue(m.isRunning)
        XCTAssertFalse(m.canDelete)
        await m.delete()
        XCTAssertEqual(bridge.deleteCalls.count, 1, "a second press while running does nothing")
        bridge.releaseDelete()
        await run.value
        XCTAssertEqual(m.phase, .receipt(receipt()))
    }

    // MARK: Row text and grouping

    private func capture(_ id: String, app: String?, title: String?, ms: Int64 = 1_700_000_000_000, excerpt: String? = nil) -> ClientItem {
        ClientItem(kind: .capture, id: id, alsoClient: nil, dateUnixMs: ms, removed: false, label: title, excerpt: excerpt, app: app)
    }

    func testCapturesFromOneAppOnOneDayFoldIntoOneGroup() {
        let day: Int64 = 1_700_000_000_000
        let items = [
            capture("1", app: "com.apple.Terminal", title: "zsh", ms: day),
            item(.email, "e1"),
            capture("2", app: "com.apple.Terminal", title: "claude --resume", ms: day + 60_000),
            capture("3", app: "com.apple.Terminal", title: "vim", ms: day + 120_000),
            capture("4", app: "com.apple.Terminal", title: "zsh", ms: day + 3 * 86_400_000),
            capture("5", app: "com.apple.Safari", title: "Docs", ms: day),
        ]
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        let rows = ClientRowText.rows(items, appName: { $0 == "com.apple.Terminal" ? "Terminal" : nil }, calendar: cal)
        XCTAssertEqual(rows.count, 4, "three Terminal captures fold; the lone ones and the email stay")
        guard case .group(let g) = rows[0] else { return XCTFail("the group sits where its first capture was") }
        XCTAssertEqual(g.items.map(\.id), ["1", "2", "3"])
        XCTAssertEqual(g.source, "Terminal")
        XCTAssertTrue(ClientRowText.groupTitle(g).contains("Terminal") && ClientRowText.groupTitle(g).contains("3"))
        XCTAssertEqual(rows[1], .item(items[1]))
        XCTAssertEqual(rows.compactMap { if case .item(let i) = $0 { i.id } else { nil } }, ["e1", "4", "5"])
        // Without an app name the window title names the group; with no app, same title and day group.
        let bare = ClientRowText.rows([capture("a", app: nil, title: "Notes", ms: day), capture("b", app: nil, title: "Notes", ms: day)], appName: { _ in nil }, calendar: cal)
        guard case .group(let b) = bare[0] else { return XCTFail("same window title groups") }
        XCTAssertEqual(b.source, "Notes")
        XCTAssertEqual(ClientRowText.rows([capture("a", app: nil, title: nil), capture("b", app: nil, title: nil)], calendar: cal).count, 2, "no app and no title: no group")
    }

    func testTickingAGroupTicksEveryCaptureAndDeletesExactlyThosePicks() async {
        let caps = (1...3).map { capture("\($0)", app: "com.apple.Terminal", title: "zsh") }
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory(possible: caps + [item(.email, "e1")]))]
        bridge.deletes = [.success(receipt())]
        let m = await review(bridge)
        XCTAssertFalse(m.isTicked(caps))
        m.setTicked(caps, true)
        XCTAssertTrue(m.isTicked(caps))
        m.setTicked(caps[1], false)
        XCTAssertFalse(m.isTicked(caps), "one unticked: the group box is off")
        XCTAssertEqual(m.tickedCount(caps), 2)
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(bridge.deleteCalls.first?.picks, [ClientItemRef(kind: .capture, id: "1"), ClientItemRef(kind: .capture, id: "3")])
    }

    func testEveryRowSaysWhatItIsAndExcerptsDecode() throws {
        for kind in [ClientItemKind.meeting, .email, .calendarEvent, .capture, .episode, .conversation, .step] {
            let labelled = ClientItem(kind: kind, id: "1", alsoClient: nil, dateUnixMs: 1_700_000_000_000, removed: false, label: "Meeting recap")
            XCTAssertEqual(ClientRowText.detail(labelled), ClientCopy.item(labelled))
            XCTAssertTrue(ClientCopy.item(labelled).hasPrefix(ClientCopy.singular(kind)), "kind first, then the day")
        }
        let json = #"{"kind":"capture","id":"7","also_client":null,"date_unix_ms":1,"removed":false,"label":"zsh","excerpt":"…lunch with Acme…","app":"com.apple.Terminal"}"#
        let c = try JSONDecoder().decode(ClientItem.self, from: Data(json.utf8))
        XCTAssertEqual(c.excerpt, "…lunch with Acme…")
        XCTAssertEqual(c.app, "com.apple.Terminal")
    }

    // MARK: Error states

    func testBusyKeepsTheReviewAndTryAgainRuns() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.failure(.busy), .success(receipt())]
        let m = await review(bridge)
        m.setTicked(item(.email, "e1"), true)
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(m.phase, .review)
        XCTAssertEqual(m.notice, .busy)
        await m.retry()
        XCTAssertEqual(bridge.deleteCalls.count, 2)
        XCTAssertEqual(bridge.deleteCalls[1].picks, [ClientItemRef(kind: .email, id: "e1")])
        XCTAssertEqual(m.phase, .receipt(receipt()))
    }

    func testSelectionChangedRebuildsTheTicksFromAFreshInventory() async {
        let bridge = FakeClientBridge()
        // The fresh inventory no longer lists e1.
        bridge.inventories = [.success(inventory()), .success(inventory(possible: [item(.conversation, "c1")]))]
        bridge.deletes = [.failure(.selectionChanged), .success(receipt())]
        let m = await review(bridge)
        m.setTicked(item(.email, "e1"), true)
        m.setTicked(item(.conversation, "c1"), true)
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(m.phase, .review)
        XCTAssertEqual(m.notice, .selectionChanged)
        XCTAssertEqual(m.ticked, [ClientItemRef(kind: .conversation, id: "c1")])
        await m.delete()
        XCTAssertEqual(bridge.deleteCalls[1].picks, [ClientItemRef(kind: .conversation, id: "c1")])
    }

    func testAPartialReceiptRetriesWithASelectionRebuiltFromAFreshInventory() async {
        let bridge = FakeClientBridge()
        let partial = receipt(client: .stillPresent, failed: [.email], complete: false, retryHelps: true)
        bridge.inventories = [.success(inventory()), .success(inventory(possible: [item(.conversation, "c1")]))]
        bridge.deletes = [.success(partial), .success(receipt())]
        let m = await review(bridge)
        m.setTicked(item(.email, "e1"), true)
        m.setTicked(item(.conversation, "c1"), true)
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(m.phase, .receipt(partial))
        await m.retry()
        XCTAssertEqual(bridge.inventoryCalls, 2, "a fresh inventory before the retry")
        XCTAssertEqual(bridge.deleteCalls[1].picks, [ClientItemRef(kind: .conversation, id: "c1")])
        XCTAssertEqual(m.phase, .receipt(receipt()))
    }

    func testARetryAfterTheClientRecordWentKeepsTheReceiptAndOffersNoMoreRetries() async {
        let bridge = FakeClientBridge()
        let partial = receipt(client: .stillPresent, failed: [.capture], complete: false, retryHelps: true)
        bridge.inventories = [.success(inventory()), .failure(.notFound)]
        bridge.deletes = [.success(partial)]
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        await m.delete()
        await m.retry()
        XCTAssertEqual(m.phase, .receipt(partial))
        XCTAssertTrue(m.retryGone)
        XCTAssertEqual(bridge.deleteCalls.count, 1)
    }

    func testRetryHelpsFalseOffersNoRetry() async {
        let bridge = FakeClientBridge()
        let final = receipt(client: .removedHereOnly, complete: false, retryHelps: false)
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(final)]
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        await m.delete()
        await m.retry()
        XCTAssertEqual(bridge.deleteCalls.count, 1)
        XCTAssertEqual(m.phase, .receipt(final))
    }

    func testNoIdentityOffersNoDeleteAndNoRetry() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        let m = await review(bridge, identity: false)
        XCTAssertEqual(m.phase, .noIdentity)
        m.confirmText = "Acme Ltd"
        XCTAssertFalse(m.canDelete)
        await m.delete()
        await m.retry()
        XCTAssertTrue(bridge.deleteCalls.isEmpty)
        XCTAssertEqual(bridge.inventoryCalls, 0)
    }

    func testTryAgainAfterAFailedLoadNeverDeletesUnconfirmed() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.failure(.failed), .success(inventory())]
        bridge.deletes = [.success(receipt())]
        let m = await review(bridge)
        XCTAssertEqual(m.phase, .failed(.failed))
        await m.retry()
        XCTAssertEqual(m.phase, .review)
        XCTAssertTrue(bridge.deleteCalls.isEmpty, "Try again reloads the review; it never deletes without the name")
    }

    func testBusyTryAgainWithTheNameClearedDeletesNothing() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.failure(.busy), .success(receipt())]
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        await m.delete()
        m.confirmText = ""
        await m.retry()
        XCTAssertEqual(bridge.deleteCalls.count, 1)
        XCTAssertEqual(m.phase, .review)
    }

    func testCoreNoDeviceIdIsTheNoIdentityState() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.failure(.noDeviceId)]
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertEqual(m.phase, .noIdentity)
        await m.retry()
        XCTAssertEqual(bridge.deleteCalls.count, 1)
    }

    func testAnUnreachableExportFolderOffersARunWithoutDeletingExports() async {
        let bridge = FakeClientBridge()
        let partial = receipt(client: .stillPresent, failed: [.exportFolder], complete: false, retryHelps: true)
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(partial), .success(receipt())]
        let m = await review(bridge)
        m.deleteExports = true
        m.confirmText = "Acme Ltd"
        await m.delete()
        XCTAssertTrue(m.exportFolderUnreachable)
        await m.retryWithoutDeletingExports()
        XCTAssertEqual(bridge.deleteCalls.map(\.deleteExports), [true, false])
        XCTAssertEqual(m.phase, .receipt(receipt()))
    }

    func testReceiptLinesHoldNoClientName() {
        let r = ClientDeleteReceipt(dateUnixMs: 1, counts: [ClientKindCount(kind: .meeting, count: 3)], notSent: [], localOnly: [],
                                    client: .removed, leftForYou: [.mailboxOriginals, .backupsAndCloud, .expiredHereOnly(2)],
                                    pendingDevices: nil, failed: [], failedStages: [], complete: true, retryHelps: false)
        var lines = [ClientCopy.receiptHeadline(r), ClientCopy.pendingDevices(r.pendingDevices), ClientCopy.reappears]
        lines += ClientCopy.counts(r.counts) + r.leftForYou.compactMap { ClientCopy.leftForYou($0)?.text }
        XCTAssertTrue(lines.contains("Meetings: 3"))
        XCTAssertFalse(lines.joined().contains("Acme"))
        XCTAssertTrue(ClientCopy.leftForYou(.backupsAndCloud)!.text.contains("Time Machine"))
    }

    /// A row shows the item's label (client text, this screen only); the receipt and its saved file never hold it.
    /// A row shows the item's label (client text, this screen only). That no receipt holds a label is guarded in core
    /// (`labels_reach_the_review_rows_and_never_a_receipt`): receipts carry no items, and this shell sends back
    /// `ClientItemRef`s only, so a shell-side receipt check here could not fail.
    func testRowsShowLabels() throws {
        let json = #"{"kind":"email","id":"e1","also_client":null,"date_unix_ms":1700000000000,"removed":false,"label":"Ostrander quarterly figures"}"#
        let labelled = try JSONDecoder().decode(ClientItem.self, from: Data(json.utf8))
        XCTAssertEqual(labelled.label, "Ostrander quarterly figures")
        XCTAssertEqual(ClientRowText.title(labelled), "Ostrander quarterly figures")
        XCTAssertEqual(ClientRowText.detail(labelled), ClientCopy.item(labelled), "kind and date under the label")
        let bare = item(.meeting, "1")
        XCTAssertEqual(ClientRowText.title(bare), ClientCopy.item(bare), "no label: kind and date")
        XCTAssertNil(ClientRowText.detail(bare))
        let old = try JSONDecoder().decode(ClientItem.self, from: Data(#"{"kind":"email","id":"e1","also_client":null,"date_unix_ms":1,"removed":false}"#.utf8))
        XCTAssertNil(old.label, "a core without labels still decodes")
    }

    func testExpiredHereOnlySaysOneItemInTheSingular() {
        XCTAssertEqual(ClientCopy.leftForYou(.expiredHereOnly(1))?.text,
                       "1 item was removed earlier on this Mac only by Keep for N months. Your other devices may still hold it.")
        XCTAssertEqual(ClientCopy.leftForYou(.expiredHereOnly(3))?.text,
                       "3 items were removed earlier on this Mac only by Keep for N months. Your other devices may still hold them.")
    }

    // MARK: Vault rule

    /// Mike: vault notes are deleted only while vault sync is on. Off, no vault goes to core and "Left for you"
    /// says the vault was not touched.
    func testTheVaultIsPassedOnlyWhileVaultSyncIsOn() throws {
        let suite = "ClientControlsVaultRule-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        var f = CoreClientControlsBridge.folders(defaults: defaults)
        XCTAssertNil(f.vault)
        XCTAssertFalse(f.vaultSyncOff, "no vault was ever chosen: nothing to say")

        defaults.set("/Users/me/Vault", forKey: VaultSettings.pathKey)
        defaults.set(false, forKey: VaultSettings.enabledKey)
        f = CoreClientControlsBridge.folders(defaults: defaults)
        XCTAssertNil(f.vault, "vault sync off: the path is not passed")
        XCTAssertTrue(f.vaultSyncOff)

        defaults.set(true, forKey: VaultSettings.enabledKey)
        f = CoreClientControlsBridge.folders(defaults: defaults)
        XCTAssertEqual(f.vault?.path, "/Users/me/Vault")
        XCTAssertFalse(f.vaultSyncOff)

        let decoded = try JSONDecoder().decode([ClientLeftForYou].self, from: Data(#"["vault_sync_off",{"vault_folder":"/v"}]"#.utf8))
        XCTAssertEqual(decoded, [.vaultSyncOff, .vaultFolder("/v")])
        XCTAssertEqual(ClientCopy.leftForYou(.vaultSyncOff)?.text,
                       "Notes in your vault are not touched while vault sync is off. Delete them yourself if they name this client.")
        XCTAssertEqual(ClientCopy.leftForYou(.vaultSyncOff)?.paths, [])
    }

    // MARK: Settings → Devices

    func testSyncLineSaysHowManyDevicesStillNeedToUpdate() async {
        XCTAssertNil(ClientCopy.syncPendingLine(nil), "not known yet: no line")
        XCTAssertNil(ClientCopy.syncPendingLine(0), "none: no line")
        XCTAssertEqual(ClientCopy.syncPendingLine(1), "1 device still needs to update before it applies client deletes.")
        XCTAssertEqual(ClientCopy.syncPendingLine(3), "3 devices still need to update before they apply client deletes.")
        let bridge = FakePendingBridge()
        bridge.count = 2
        let m = ClientPendingDevicesModel(bridge: bridge)
        XCTAssertNil(m.line)
        await m.load()
        XCTAssertEqual(m.line, "2 devices still need to update before they apply client deletes.")
        bridge.count = 0
        await m.load()
        XCTAssertNil(m.line)
    }

    func testRemovingADeviceNeedsTheConfirmationAndOnlyRemovesThatOne() async {
        let bridge = FakePendingBridge()
        bridge.count = 2
        bridge.ids = ["pk-old", "pk-attic"]
        let m = ClientPendingDevicesModel(bridge: bridge)
        await m.load()
        XCTAssertEqual(m.ids, ["pk-old", "pk-attic"])
        XCTAssertNil(m.confirm(), "nothing is removed without the confirmation")
        m.ask("pk-unknown")
        XCTAssertFalse(m.showingConfirmation, "only a listed device")
        m.ask("pk-old")
        XCTAssertTrue(m.showingConfirmation)
        m.cancel()
        XCTAssertNil(m.confirm(), "Cancel removes nothing")
        m.ask("pk-old")
        m.showingConfirmation = false  // the alert closing before its button's action runs
        await m.confirm()?.value
        XCTAssertEqual(bridge.retired, ["pk-old"])
        XCTAssertEqual(m.ids, ["pk-attic"])
        XCTAssertEqual(m.line, "1 device still needs to update before it applies client deletes.")
        XCTAssertNil(m.confirm(), "one confirmation, one removal")

        // Core says it is no longer listed (it updated meanwhile): the list is read again.
        bridge.retireResult = .failure(.notFound)
        bridge.count = 0
        m.ask("pk-attic")
        bridge.ids = []
        await m.confirm()?.value
        XCTAssertEqual(m.errorText, ClientCopy.deviceNoLongerListed)
        XCTAssertNil(m.line)
        XCTAssertTrue(m.ids.isEmpty)
    }

    // MARK: Settings

    /// 0.9.80 device check, row 9: the switch went on, then gray, and its state was unclear while it saved.
    func testTheSwitchShowsTheChoiceAtOnceWhileItSaves() async {
        let bridge = FakeClientBridge()
        bridge.holdUpdate = true
        let m = ClientSettingsModel(key: "acme", bridge: bridge)
        await m.load()
        let write = Task { await m.setExcludeHosted(true) }
        while !m.isSaving { await Task.yield() }
        XCTAssertTrue(m.excludeHosted, "the choice shows before core has saved it")
        XCTAssertTrue(m.isSaving, "Saving… shows")
        bridge.releaseUpdate()
        await write.value
        XCTAssertFalse(m.isSaving)
        XCTAssertTrue(m.excludeHosted)
        XCTAssertNil(m.requested)
        XCTAssertEqual(bridge.control.excludeHosted, true)
    }

    func testAFailedSaveFallsBackToWhatCoreHoldsAndSaysSo() async {
        let bridge = FakeClientBridge()
        bridge.updateError = .failed
        let m = ClientSettingsModel(key: "acme", bridge: bridge)
        await m.load()
        await m.setExcludeHosted(true)
        XCTAssertFalse(m.excludeHosted, "back to the saved value")
        XCTAssertFalse(m.isSaving)
        XCTAssertNotNil(m.errorText)
    }

    func testKeepAndExcludeWritesDoNotUndoEachOther() async {
        let bridge = FakeClientBridge()
        let m = ClientSettingsModel(key: "acme", bridge: bridge)
        await m.load()
        async let a: Void = m.setExcludeHosted(true)
        async let b: Void = m.setKeepMonths(12)
        _ = await (a, b)
        XCTAssertEqual(bridge.control, ClientControl(excludeHosted: true, keepMonths: 12))
        XCTAssertEqual(m.keepMonths, 12)
        XCTAssertTrue(m.excludeHosted)
    }

    func testAWriteAppliesToTheValueSyncedSinceTheLoadNotTheStaleOne() async {
        let bridge = FakeClientBridge()
        let m = ClientSettingsModel(key: "acme", bridge: bridge)
        await m.load()
        XCTAssertFalse(m.excludeHosted)
        // Another device turned exclusion on; a sync round applied it after the page loaded.
        bridge.control = ClientControl(excludeHosted: true, keepMonths: nil)
        await m.setKeepMonths(12)
        XCTAssertEqual(bridge.control, ClientControl(excludeHosted: true, keepMonths: 12), "exclusion stays on")
        XCTAssertTrue(m.excludeHosted, "the page shows the synced value")
        XCTAssertNil(m.exclusionNotice, "it was not turned on here")
    }

    func testANonClientTopicHidesTheSection() async {
        let bridge = FakeClientBridge()
        bridge.controlError = .notFound
        let m = ClientSettingsModel(key: "x", bridge: bridge)
        await m.load()
        XCTAssertFalse(m.isClient)
    }

    // MARK: Keep for N months review

    func testTheRetentionReviewKeepsSnoozesAndExpiresWhileMore() async {
        let bridge = FakeClientBridge()
        let a = item(.email, "a"), b = item(.email, "b"), c = item(.email, "c")
        bridge.due = [ClientDue(key: "acme", items: [a, b, c])]
        let now = Date(timeIntervalSince1970: 1_000)
        let m = ClientRetentionReviewModel(names: ["acme": "Acme Ltd"], bridge: bridge, now: { now })
        await m.load()
        XCTAssertEqual(m.groups.first?.name, "Acme Ltd")
        XCTAssertEqual(m.tickedCount, 3)
        await m.keep(key: "acme", a)
        await m.snooze(key: "acme", b)
        XCTAssertEqual(bridge.decisions.map(\.0), [a.ref, b.ref])
        XCTAssertEqual(bridge.decisions[1].1, .snooze(untilMs: (1_000 + 30 * 86_400) * 1000))
        XCTAssertEqual(m.groups.first?.items, [c])
        bridge.expires = [.success(expiry(deleted: 0, more: true)), .success(expiry(deleted: 1, more: false))]
        await m.deleteTicked()
        XCTAssertEqual(bridge.expireCalls, [[c.ref], [c.ref]])
        guard case .done(let s) = m.phase else { return XCTFail("\(m.phase)") }
        XCTAssertEqual(s.counts, [ClientKindCount(kind: .email, count: 1)])
        XCTAssertTrue(s.complete)
        await m.later()
        XCTAssertEqual(bridge.dismissed, 1)
    }

    func testAFailedReReadInTheExpiryLoopNeverReportsDone() async {
        let bridge = FakeClientBridge()
        bridge.due = [ClientDue(key: "acme", items: [item(.email, "a"), item(.email, "b")])]
        let m = ClientRetentionReviewModel(names: [:], bridge: bridge)
        await m.load()
        bridge.expires = [.success(expiry(deleted: 1, more: true))]
        bridge.dueForKeyError = .failed
        await m.deleteTicked()
        guard case .done(let s) = m.phase else { return XCTFail("\(m.phase)") }
        XCTAssertFalse(s.complete)
        XCTAssertEqual(s.stopped, .failed)
    }

    func testHeadlinesOnlySayTryAgainWhenItCanHelp() {
        let stuck = receipt(client: .removed, failed: [.capture], complete: false, retryHelps: false)
        XCTAssertFalse(ClientCopy.receiptHeadline(stuck).contains("Try again"))
        let helps = receipt(client: .stillPresent, failed: [.capture], complete: false, retryHelps: true)
        XCTAssertTrue(ClientCopy.receiptHeadline(helps).contains("Try again"))
    }

    // MARK: Final review (14b)

    func testLeftForYouSaysDerivedCopiesStayOnOtherDevicesUntilTheirOwnRetention() {
        let line = ClientCopy.leftForYou(.copiesOnOtherDevices)!.text
        XCTAssertTrue(line.contains("Routine results, profile suggestions, people's names and meeting briefs"), line)
        XCTAssertTrue(line.contains("the delete does not remove them there"), line)
        XCTAssertFalse(line.contains("retention"), "no retention removes routine results or people")
        let expiry = ClientCopy.expiryLeftForYou(.copiesOnOtherDevices)!.text
        XCTAssertTrue(expiry.contains("on this Mac only"), expiry)
        XCTAssertFalse(expiry.contains("applies the delete"), expiry)
        XCTAssertEqual(ClientCopy.expiryLeftForYou(.mailboxOriginals), ClientCopy.leftForYou(.mailboxOriginals))
        XCTAssertTrue(ClientCopy.derived.hasPrefix("On this Mac, "))
        XCTAssertTrue(ClientCopy.reappears.contains("new mail or a new meeting"))
    }

    func testACleanUpThatFailedAfterTheRecordWentOffersNoTryAgainAndSaysWhatToDo() async {
        let r = ClientDeleteReceipt(dateUnixMs: 1, counts: [], notSent: [], localOnly: [], client: .removed, leftForYou: [],
                                    pendingDevices: 0, failed: [], failedStages: [.cleanup], complete: false, retryHelps: true)
        let headline = ClientCopy.receiptHeadline(r)
        XCTAssertFalse(headline.contains("Try again"), headline)
        XCTAssertTrue(headline.contains("can't be run again"), headline)
        XCTAssertTrue(headline.contains("next time you delete a client"), headline)
        XCTAssertFalse(ClientCopy.retryHelps(r))
        let hereOnly = ClientDeleteReceipt(dateUnixMs: 1, counts: [], notSent: [], localOnly: [], client: .removedHereOnly, leftForYou: [],
                                           pendingDevices: nil, failed: [], failedStages: [.cleanup], complete: false, retryHelps: true)
        XCTAssertFalse(ClientCopy.retryHelps(hereOnly), "no client left to run again here either")
        XCTAssertTrue(ClientCopy.retryHelps(receipt(client: .stillPresent, failed: [.capture], complete: false, retryHelps: true)))
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.deletes = [.success(r)]
        let m = await review(bridge)
        m.confirmText = "Acme Ltd"
        await m.delete()
        let reads = bridge.inventoryCalls
        await m.retry()
        XCTAssertEqual(bridge.inventoryCalls, reads, "nothing to run again")
        XCTAssertEqual(bridge.deleteCalls.count, 1)
        XCTAssertEqual(m.phase, .receipt(r))
    }

    func testTheExcludeToggleWarnsWhileDevicesHoldTheSettingBackAndNeverSaysZero() async {
        XCTAssertNil(ClientCopy.heldDevices(nil))
        XCTAssertNil(ClientCopy.heldDevices(0))
        XCTAssertTrue(ClientCopy.heldDevices(1)!.hasPrefix("1 device on your sync hasn't updated yet."))
        XCTAssertTrue(ClientCopy.heldDevices(3)!.hasPrefix("3 devices on your sync haven't updated yet."))
        XCTAssertTrue(ClientCopy.heldDevices(3)!.contains("could still send this client's material to a hosted model"))
        XCTAssertTrue(ClientCopy.heldDevices(3)!.contains("Update those devices."))
        XCTAssertFalse(ClientCopy.heldDevices(1)!.contains("Set it on each device you use"))
        let bridge = FakeClientBridge()
        bridge.held = 2
        let m = ClientSettingsModel(key: "acme", bridge: bridge)
        await m.load()
        XCTAssertEqual(m.heldDevices, 2)
        bridge.held = nil
        await m.load()
        XCTAssertNil(m.heldDevices)
    }

    func testTheMatcherLimitIsStatedAndTheLeftOutNoteCountsItems() {
        XCTAssertTrue(ClientCopy.matchingLimit.contains("whole words"))
        XCTAssertNil(ClientCopy.leftOut(0))
        XCTAssertEqual(ClientCopy.leftOut(1), "Left out 1 item that involves an excluded client.")
        XCTAssertEqual(ClientCopy.leftOut(4), "Left out 4 items that involve an excluded client.")
    }

    func testCoresExclusionOutcomeDecodes() throws {
        let d = JSONDecoder()
        let filtered = try d.decode(CorePrepareAskData.self, from: Data(#"{"prepare_id":"id","prompt":"p","exclusion":{"kind":"filtered_hosted","left_out":2}}"#.utf8))
        XCTAssertEqual(filtered.exclusion, .filteredHosted(leftOut: 2))
        XCTAssertEqual(filtered.exclusion?.leftOut, 2)
        let allowed = try d.decode(CorePrepareAskData.self, from: Data(#"{"prepare_id":"id","prompt":"p","exclusion":{"kind":"allowed"}}"#.utf8))
        XCTAssertEqual(allowed.exclusion?.leftOut, 0)
        let older = try d.decode(CorePrepareAskData.self, from: Data(#"{"prepare_id":"id","prompt":"p"}"#.utf8))
        XCTAssertNil(older.exclusion)
        let newer = try d.decode(CorePrepareAskData.self, from: Data(#"{"prepare_id":"id","prompt":"p","exclusion":{"kind":"something_new"}}"#.utf8))
        XCTAssertEqual(newer.exclusion, .other)
    }

    func testAnExpiryRefusedForNoDeviceIdSaysSoInItsOwnWords() async {
        let bridge = FakeClientBridge()
        bridge.due = [ClientDue(key: "acme", items: [item(.email, "a")])]
        bridge.expires = [.failure(.noDeviceId)]
        let m = ClientRetentionReviewModel(names: ["acme": "Acme Ltd"], bridge: bridge)
        await m.load()
        await m.deleteTicked()
        guard case .done(let s) = m.phase else { return XCTFail("\(m.phase)") }
        XCTAssertEqual(s.stopped, .noDeviceId)
        let line = ClientCopy.expiryStopped(.noDeviceId)
        XCTAssertTrue(line.contains("no device identity"), line)
        XCTAssertFalse(line.contains("can't delete a client"), line)
        XCTAssertEqual(ClientCopy.expiryStopped(.busy), ClientCopy.message(.busy))
        XCTAssertTrue(CoreClientControlsBridge.isRealDeviceId("abc"))
        XCTAssertFalse(CoreClientControlsBridge.isRealDeviceId("spike-device"))
        XCTAssertFalse(CoreClientControlsBridge.isRealDeviceId(" "))
    }

    func testTheSavedReceiptHoldsNoPathsAndNoDerivedClaimWhenThatStageFailed() {
        let r = ClientDeleteReceipt(dateUnixMs: 1, counts: [], notSent: [], localOnly: [], client: .stillPresent,
                                    leftForYou: [.filesNotRemoved(["/Users/m/Pictures/Whimbrel/Snapshots/2026-11-01 Acme kickoff"])],
                                    pendingDevices: nil, failed: [], failedStages: [.derivedText], complete: false, retryHelps: true)
        let text = ClientReceiptFile.text(r)
        XCTAssertFalse(text.contains("Acme"))
        XCTAssertFalse(text.contains(ClientCopy.derived))
    }

    // MARK: Export

    func testExportCopiesMediaAfterTheWriteAndShowsPartial() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.written = .success(ClientExportWritten(folder: "/x/E", mediaToken: 7))
        bridge.copied = .success(ClientExportDone(folder: "/x/E", media: .init(files: 2, skipped: 0, failed: 1), partial: true))
        let m = await review(bridge, mode: .export)
        m.includeMedia = true
        await m.export(to: URL(fileURLWithPath: "/x"))
        XCTAssertEqual(bridge.exportCalls.first?.1, true)
        XCTAssertEqual(m.phase, .exported(.init(folder: "/x/E", media: .init(files: 2, skipped: 0, failed: 1), partial: true)))
    }

    func testANetworkShareExportSaysWhy() async {
        let bridge = FakeClientBridge()
        bridge.inventories = [.success(inventory())]
        bridge.written = .failure(.folderNotAllowed)
        let m = await review(bridge, mode: .export)
        await m.export(to: URL(fileURLWithPath: "/Volumes/share"))
        XCTAssertEqual(m.phase, .failed(.folderNotAllowed))
        XCTAssertTrue(ClientCopy.message(.folderNotAllowed).contains("network share"))
        m.backToReview()
        XCTAssertEqual(m.phase, .review)
    }
}

/// Exclusion refusals say what happened (nothing was sent), never "couldn't reach the provider".
@MainActor
final class ExclusionRefusalMessageTests: XCTestCase {
    func testRefusalsHaveTheirOwnWords() {
        let excluded = CoreLLMError.providerFailureBecause(.excludedClient)
        let unreadable = CoreLLMError.providerFailureBecause(.exclusionUnreadable)
        XCTAssertNotEqual(excluded.userFacingMessage, CoreLLMError.providerFailure.userFacingMessage)
        XCTAssertNotEqual(unreadable.userFacingMessage, CoreLLMError.providerFailure.userFacingMessage)
        XCTAssertTrue(excluded.userFacingMessage.hasPrefix("Not sent"))
        XCTAssertTrue(excluded.opensAIProvidersSettings)
        for text in [excluded.userFacingMessage, unreadable.userFacingMessage] {
            XCTAssertFalse(text.lowercased().contains("never leaves"))
            XCTAssertFalse(text.lowercased().contains("nothing leaves"))
        }
    }

    func testAStreamTheGateRefusedCarriesItsClass() {
        let vm = StreamingAskViewModel()
        vm.apply(.terminal(runId: "r", outcome: .failed(classification: "excluded_client", message: "x"), atUnixMs: 0, finalText: "", usage: .init()))
        XCTAssertEqual(vm.exclusionRefusal, .excludedClient)
        XCTAssertEqual(vm.phase, .failed(message: CoreLLMError.excludedClientMessage))
        vm.apply(.terminal(runId: "r", outcome: .failed(classification: "exclusion_unreadable", message: "x"), atUnixMs: 0, finalText: "", usage: .init()))
        XCTAssertEqual(vm.exclusionRefusal, .unreadable)
        XCTAssertEqual(vm.phase, .failed(message: CoreLLMError.exclusionUnreadableMessage))
    }
}

final class WindowAskDestinationDecodingTests: XCTestCase {
    func testCoresBareDestinationsDecode() throws {
        let d = JSONDecoder()
        XCTAssertEqual(try d.decode(WindowAskDestination.self, from: Data(#""local_only""#.utf8)), .localOnly)
        XCTAssertEqual(try d.decode(WindowAskDestination.self, from: Data(#""local""#.utf8)), .local)
        XCTAssertEqual(try d.decode(WindowAskDestination.self, from: Data(#"{"hosted":{"provider":"ollama"}}"#.utf8)), .hosted(provider: "ollama"))
    }
}

/// The topic page column (TopicsView.pane): a slow load shows "Loading…", never the previous topic's page.
final class TopicsPaneTests: XCTestCase {
    func testShownPageForTheSelectionIsThePage() {
        XCTAssertEqual(TopicsView.pane(selected: 7, shown: 7, answered: .some(7)), .page)
    }

    func testNewSelectionLoadsInsteadOfShowingThePreviousTopic() {
        XCTAssertEqual(TopicsView.pane(selected: 8, shown: 7, answered: .some(7)), .loading)
    }

    func testBeforeAnyLoadFinishesIsLoading() {
        XCTAssertEqual(TopicsView.pane(selected: nil, shown: nil, answered: .none), .loading)
        XCTAssertEqual(TopicsView.pane(selected: 8, shown: nil, answered: .none), .loading)
    }

    func testFirstTopicPageShowsBeforeTheSelectionIsSet() {
        XCTAssertEqual(TopicsView.pane(selected: nil, shown: 7, answered: .some(7)), .page)
    }

    func testALoadThatReturnedNoPageStopsTheSpinner() {
        XCTAssertEqual(TopicsView.pane(selected: 8, shown: nil, answered: .some(8)), .choose)
        XCTAssertEqual(TopicsView.pane(selected: nil, shown: nil, answered: .some(nil)), .choose)
    }
}
