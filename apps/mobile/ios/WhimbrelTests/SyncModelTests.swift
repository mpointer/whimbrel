import XCTest
@testable import Whimbrel

/// Stands in for the local store: scripted pull results, a settable count.
final class FakeStore: MeetingSyncStore, PhoneReadStore {
    var pullResults: [Bool] = []
    var count = 0
    var countAfterPull: Int?
    /// Written on the store queue, read on the main actor: lock-protected.
    private let lock = NSLock()
    private var _pulls: [SyncCredentials] = []
    var pulls: [SyncCredentials] { lock.lock(); defer { lock.unlock() }; return _pulls }
    /// Called (on the store queue) after each pull is recorded.
    var onPull: (() -> Void)?

    func pullOnly(_ credentials: SyncCredentials, captureWindowDays: Int32) -> Bool {
        lock.lock(); _pulls.append(credentials); lock.unlock()
        defer { onPull?() }
        let ok = pullResults.isEmpty ? true : pullResults.removeFirst()
        if ok, let countAfterPull { count = countAfterPull }
        return ok
    }

    func groupedMeetingCount() -> Int? { count }
    func lastSyncError() -> String? { "connect failed: secret detail" }

    // PhoneReadStore: scripted, and records the thread each read ran on.
    var meetings: [PhoneMeeting] = []
    private(set) var readsOnMainThread: [Bool] = []
    func recentMeetings(limit: Int) -> [PhoneMeeting] {
        lock.lock(); readsOnMainThread.append(Thread.isMainThread); lock.unlock()
        return Array(meetings.prefix(limit))
    }
    func meeting(id: Int64) -> PhoneMeeting? { meetings.first { $0.id == id } }
    var transcripts: [Int64: [PhoneTranscriptSegment]] = [:]
    var items: [Int64: [PhoneActionItem]] = [:]
    func transcriptPage(meetingId: Int64, offset: Int, limit: Int) -> PhoneTranscriptPage? {
        let all = transcripts[meetingId] ?? []
        return PhoneTranscriptPage(segments: Array(all.dropFirst(offset).prefix(limit)), total: all.count)
    }
    func actionItems(meetingId: Int64) -> [PhoneActionItem] { items[meetingId] ?? [] }
    var runs: [PhoneRoutineRun] = []
    var events: [PhoneCalendarEvent] = []
    var preps: [String: PhoneMeetingPrep] = [:]
    func routineRuns(limit: Int) -> [PhoneRoutineRun] { Array(runs.prefix(limit)) }
    func prep(eventId: String) -> PhoneMeetingPrep? { preps[eventId] }
    func upcomingEvents(fromUnixMs: Int64, untilUnixMs: Int64, limit: Int) -> [PhoneCalendarEvent] {
        Array(events.filter { $0.startUnixMs >= fromUnixMs && $0.startUnixMs < untilUnixMs }.sorted { $0.startUnixMs < $1.startUnixMs }.prefix(limit))
    }
}

@MainActor
final class FakeCredentials: SyncCredentialSource {
    var credentials: SyncCredentials? = SyncCredentials(tursoToken: "t", tursoDatabaseUrl: "u", groupKeyBase64: "k", deviceId: "phone")
    private(set) var renewals: [(afterFailure: Bool, force: Bool)] = []
    func syncCredentials() -> SyncCredentials? { credentials }
    /// Runs inside every renewIfDue, like a revoke found while renewing.
    var onRenew: (() -> Void)?
    func renewIfDue(afterFailure: Bool, force: Bool) async {
        renewals.append((afterFailure, force))
        onRenew?()
    }
}

@MainActor
final class SyncModelTests: XCTestCase {
    var store: FakeStore!
    var credentials: FakeCredentials!
    var sleeper: FakeSleeper!

    override func setUp() async throws {
        store = FakeStore()
        credentials = FakeCredentials()
        sleeper = FakeSleeper()
    }

    private func makeModel(store: MeetingSyncStore?? = nil) -> SyncModel {
        let sleeper = sleeper!
        return SyncModel(store: store ?? self.store, credentials: credentials, sleep: { try await sleeper.sleep($0) })
    }

    func testReadRunsOnTheStoreQueueAndReturnsNilWithoutAStore() async {
        let model = makeModel()
        await model.syncNow() // opens nothing new; the store was injected
        let count = await model.read { $0.recentMeetings(limit: 5).count }
        XCTAssertEqual(count, 0)
        XCTAssertEqual(store.readsOnMainThread, [false], "reads never run on the main thread")
        let none = await makeModel(store: .some(nil)).read { $0.recentMeetings(limit: 5).count }
        XCTAssertNil(none, "no store, no read")
    }

    func testReadRevisionIncrementsAfterASuccessfulPullOnly() async {
        let model = makeModel()
        XCTAssertEqual(model.readRevision, 0)
        store.pullResults = [true, false]
        await model.syncNow()
        XCTAssertEqual(model.readRevision, 1)
        await model.syncNow()
        XCTAssertEqual(model.readRevision, 1, "a failed pull changes nothing to reload")
    }

    func testReadRevisionIncrementsWhenTheStoreOpensEvenIfThePullFails() async {
        store.pullResults = [false]
        let model = SyncModel(openStore: { [store] in store }, credentials: credentials, sleep: { try await self.sleeper.sleep($0) })
        XCTAssertEqual(model.readRevision, 0)
        await model.syncNow()
        XCTAssertGreaterThan(model.readRevision, 0, "screens that read before the store opened must read again, offline too")
    }

    func testResetClosesTheStoreAndReopensLazily() async {
        var opened = 0
        weak var weakStore: FakeStore?
        let model = SyncModel(openStore: {
            opened += 1
            let fresh = FakeStore()
            weakStore = fresh
            return fresh
        }, credentials: credentials, sleep: { try await self.sleeper.sleep($0) })
        await model.syncNow()
        XCTAssertNotNil(weakStore)
        XCTAssertNotNil(model.meetingCount)
        model.reset {}
        XCTAssertNil(weakStore, "the store handle is released")
        XCTAssertNil(model.meetingCount)
        XCTAssertNil(model.loopTask)
        await model.syncNow()
        XCTAssertEqual(opened, 2, "re-enrolling opens a fresh store")
    }

    /// A revoke is usually found while a round is in flight (renewIfDue inside syncNow): the
    /// reset it triggers must not wait on that round. It completes when the round ends.
    func testResetRequestedDuringARoundCompletesAfterTheRound() async {
        weak var weakStore: FakeStore?
        let model = SyncModel(openStore: {
            let fresh = FakeStore()
            weakStore = fresh
            return fresh
        }, credentials: credentials, sleep: { try await self.sleeper.sleep($0) })
        var cleaned = 0
        credentials.onRenew = { model.reset { cleaned += 1 } }
        await model.syncNow()
        XCTAssertEqual(cleaned, 1, "cleanup ran once the round ended")
        XCTAssertNil(weakStore, "the store is released")
        XCTAssertNil(model.meetingCount)
        XCTAssertFalse(model.isSyncing)
    }

    func testStoreRemovalMarkerIsSetFirstAndClearedOnlyAfterSuccess() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "whimbrel.test.\(UUID().uuidString)"))
        StoreRemoval.markPending(defaults)
        XCTAssertTrue(defaults.bool(forKey: StoreRemoval.pendingKey))
        StoreRemoval.finish(defaults) { throw CocoaError(.fileWriteNoPermission) }
        XCTAssertTrue(defaults.bool(forKey: StoreRemoval.pendingKey), "failed removal stays pending")
        StoreRemoval.finishIfPending(defaults) { throw CocoaError(.fileNoSuchFile) }
        XCTAssertFalse(defaults.bool(forKey: StoreRemoval.pendingKey), "already gone counts as done")
        var removals = 0
        StoreRemoval.finishIfPending(defaults) { removals += 1 }
        XCTAssertEqual(removals, 0, "nothing pending, nothing removed")
    }

    func testCadenceIs60SecondsDoublingTo15MinutesOnFailures() {
        XCTAssertEqual((0...6).map { SyncTiming.nextDelaySeconds(consecutiveFailures: $0) }, [60, 120, 240, 480, 900, 900, 900])
    }

    func testStartPullsAtOnceThenEveryMinuteAndShowsTheGroupedCount() async {
        sleeper.limit = 2
        store.countAfterPull = 7
        let model = makeModel()
        model.start()
        await model.loopTask?.value
        XCTAssertEqual(store.pulls.count, 3)
        XCTAssertEqual(store.pulls.first?.deviceId, "phone")
        XCTAssertEqual(sleeper.intervals, [60, 60, 60])
        XCTAssertEqual(model.meetingCount, 7)
        XCTAssertNotNil(model.lastSyncedAt)
        XCTAssertNil(model.lastError)
        XCTAssertFalse(model.isSyncing)
        XCTAssertEqual(credentials.renewals.first?.afterFailure, false)
    }

    func testFailuresBackOffShowAGenericMessageAndRenewTheToken() async {
        sleeper.limit = 2
        store.pullResults = [false, false, true]
        let model = makeModel()
        model.start()
        await model.loopTask?.value
        XCTAssertEqual(sleeper.intervals, [120, 240, 60])
        XCTAssertNil(model.lastError, "cleared by the good round")
        XCTAssertTrue(credentials.renewals.contains { $0.afterFailure }, "a failed round re-authenticates (token may be refused)")
    }

    func testFailureMessageIsGenericAndKeepsTheLastCount() async {
        store.count = 3
        store.pullResults = [false]
        let model = makeModel()
        await model.syncNow()
        XCTAssertEqual(model.lastError, "Couldn't sync. Will retry automatically.")
        XCTAssertFalse(model.lastError?.contains("secret") ?? true, "the core's cause goes to the log only")
        XCTAssertEqual(model.meetingCount, 3, "what's already on the phone still shows")
        XCTAssertNil(model.lastSyncedAt)
    }

    func testNoTokenYetIsAFailureWithoutPulling() async {
        credentials.credentials = nil
        let model = makeModel()
        await model.syncNow()
        XCTAssertEqual(store.pulls.count, 0)
        XCTAssertEqual(model.lastError, "Couldn't reach the server. Will retry automatically.")
    }

    func testManualSyncForcesARenewal() async {
        let model = makeModel()
        await model.syncNow(manual: true)
        XCTAssertEqual(credentials.renewals.first?.force, true)
    }

    func testNoLocalStoreSaysSo() async {
        let model = makeModel(store: .some(nil))
        await model.syncNow()
        XCTAssertEqual(model.lastError, "This iPhone's local storage is unavailable.")
        XCTAssertNil(model.meetingCount)
    }

    func testStopEndsTheLoopAndNoMorePulls() async throws {
        let firstPull = expectation(description: "first pull")
        firstPull.assertForOverFulfill = false
        store.onPull = { firstPull.fulfill() }
        let model = SyncModel(store: store, credentials: credentials, sleep: { _ in try await Task.sleep(nanoseconds: 3_600 * 1_000_000_000) })
        model.start()
        let loop = try XCTUnwrap(model.loopTask)
        await fulfillment(of: [firstPull], timeout: 10)
        model.stop()
        XCTAssertNil(model.loopTask)
        guard loop.isCancelled else { return XCTFail("the loop must be cancelled") }  // else the await below hangs
        await loop.value
        XCTAssertEqual(store.pulls.count, 1)
    }

    func testStartTwiceRunsOneLoop() {
        let model = SyncModel(store: store, credentials: credentials, sleep: { _ in try await Task.sleep(nanoseconds: 3_600 * 1_000_000_000) })
        model.start()
        let first = model.loopTask
        model.start()
        XCTAssertTrue(first == model.loopTask)
        model.stop()
    }

    /// Opening the store (migrations) happens on the store queue, not the main thread, and
    /// the model shows syncing while it opens.
    func testStoreOpensOffTheMainThreadOnFirstSync() async {
        let opened = NSLock()
        var openedOnMain: Bool?
        let fake = store!
        let model = SyncModel(openStore: {
            opened.lock(); openedOnMain = Thread.isMainThread; opened.unlock()
            return fake
        }, credentials: credentials, sleep: { _ in })
        await model.syncNow()
        opened.lock(); defer { opened.unlock() }
        XCTAssertEqual(openedOnMain, false)
        XCTAssertEqual(store.pulls.count, 1)
        XCTAssertNil(model.lastError)
    }

    func testStoreThatFailsToOpenSaysSo() async {
        let model = SyncModel(openStore: { nil }, credentials: credentials, sleep: { _ in })
        await model.syncNow()
        XCTAssertEqual(model.lastError, "This iPhone's local storage is unavailable.")
        XCTAssertFalse(model.isSyncing)
    }
}
