import XCTest
@testable import Whimbrel

/// #590: sync stopped for four days after a sign-in problem, and only a relaunch fixed it.
final class SyncStallRecoveryTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func ago(_ minutes: Double) -> Date { now.addingTimeInterval(-minutes * 60) }

    // MARK: which statuses retry

    func testOnlyErrorAndDeviceLimitAreRetryable() {
        XCTAssertTrue(SyncStatus.error("Sign-in expired").isResumable)
        XCTAssertTrue(SyncStatus.deviceLimit(kind: "computer", holders: []).isResumable)
        XCTAssertFalse(SyncStatus.notSignedIn.isResumable)
        XCTAssertFalse(SyncStatus.signingIn.isResumable)
        XCTAssertFalse(SyncStatus.waitingForApproval.isResumable)
        XCTAssertFalse(SyncStatus.waitingForGroupKey.isResumable)
        XCTAssertFalse(SyncStatus.rateLimited.isResumable)
        XCTAssertFalse(SyncStatus.ready(tursoToken: "t", tursoDatabaseUrl: "u").isResumable)
    }

    // MARK: the 30-minute throttle

    func testAResumeIsAllowedWhenNoneRanYet() {
        XCTAssertTrue(SyncTiming.resumeAllowed(now: now, lastAttemptAt: nil))
    }

    func testAResumeWaitsThirtyMinutesFromTheLastAttempt() {
        XCTAssertFalse(SyncTiming.resumeAllowed(now: now, lastAttemptAt: ago(1)))
        XCTAssertFalse(SyncTiming.resumeAllowed(now: now, lastAttemptAt: ago(29.9)))
        XCTAssertTrue(SyncTiming.resumeAllowed(now: now, lastAttemptAt: ago(30)))
    }

    func testAClockSetBackDoesNotBlockResumesForever() {
        XCTAssertTrue(SyncTiming.resumeAllowed(now: now, lastAttemptAt: now.addingTimeInterval(3600)))
    }

    // MARK: the watchdog

    private func action(ready: Bool = true, resumable: Bool = false, key: Bool = true, busy: Bool = false,
                        failures: Int = 0, finished: Date?, started: Date? = nil, wake: Date? = nil, round: Date? = nil) -> SyncTiming.WatchdogAction {
        SyncTiming.watchdogAction(statusIsReady: ready, statusIsResumable: resumable, hasDeviceKey: key, interactiveFlowActive: busy,
                                  consecutiveFailures: failures, now: now, lastRoundFinishedAt: finished, loopStartedAt: started,
                                  lastWakeAt: wake, roundStartedAt: round)
    }

    func testARecentRoundIsHealthy() {
        XCTAssertEqual(action(finished: ago(1)), .healthy)
        XCTAssertEqual(action(finished: ago(9.9)), .healthy)
    }

    func testNoRoundForTenMinutesRestartsTheRound() {
        XCTAssertEqual(action(finished: ago(10)), .restartRound)
        XCTAssertEqual(action(finished: ago(60 * 24 * 4)), .restartRound)
    }

    func testAFreshlyStartedLoopGetsTimeToFinishItsFirstRound() {
        XCTAssertEqual(action(finished: ago(60 * 24 * 4), started: ago(2)), .healthy)
        XCTAssertEqual(action(finished: ago(60 * 24 * 4), started: ago(11)), .restartRound)
    }

    func testAReadyStatusWithNoRoundEverIsRestarted() {
        XCTAssertEqual(action(finished: nil, started: nil), .restartRound)
    }

    func testABackedOffLoopIsNotRestartedForSleepingItsBackoff() {
        // Six failures in a row sleep 15 minutes between rounds.
        XCTAssertEqual(action(failures: 6, finished: ago(16)), .healthy)
        XCTAssertEqual(action(failures: 6, finished: ago(20)), .restartRound)
    }

    func testWatchdogGraceAfterWake() {
        // Slept all night: the last round is hours old, but the loop has just been woken.
        XCTAssertEqual(action(finished: ago(60 * 8), wake: ago(2)), .healthy)
        XCTAssertEqual(action(finished: ago(60 * 8), wake: ago(11)), .restartRound)
    }

    func testARoundInFlightIsNeverReplaced() {
        XCTAssertEqual(action(finished: ago(60), round: ago(1)), .healthy)
        XCTAssertEqual(action(finished: ago(60), round: ago(6)), .roundStuck)
    }

    func testNoResumeDuringSwapOrSignIn() {
        XCTAssertEqual(action(ready: false, resumable: true, busy: true, finished: nil), .healthy)
        XCTAssertEqual(action(ready: false, resumable: true, busy: false, finished: nil), .resumeStatus)
    }

    @MainActor
    func testPendingFreshStartDoesNotBlockResume() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "SyncStallRecoveryTests.\(UUID().uuidString)"))
        defaults.set(true, forKey: "com.mpointer.kestrel.syncFreshStartPending")
        let state = SyncState(signIn: nil, worker: nil, defaults: defaults)
        XCTAssertFalse(state.interactiveFlowActive)
    }

    func testStaleHiddenWhenSignedOutOrWaiting() {
        XCTAssertTrue(SyncStatus.ready(tursoToken: "t", tursoDatabaseUrl: "u").canBeStale)
        XCTAssertTrue(SyncStatus.error("x").canBeStale)
        XCTAssertTrue(SyncStatus.deviceLimit(kind: "computer", holders: []).canBeStale)
        XCTAssertFalse(SyncStatus.notSignedIn.canBeStale)
        XCTAssertFalse(SyncStatus.signingIn.canBeStale)
        XCTAssertFalse(SyncStatus.waitingForApproval.canBeStale)
        XCTAssertFalse(SyncStatus.waitingForGroupKey.canBeStale)
        XCTAssertFalse(SyncStatus.rateLimited.canBeStale)
    }

    func testAnErrorStatusWithAKeyResumesTheStatus() {
        XCTAssertEqual(action(ready: false, resumable: true, finished: ago(60 * 96)), .resumeStatus)
    }

    func testAnErrorStatusWithoutAKeyIsLeftAlone() {
        XCTAssertEqual(action(ready: false, resumable: true, key: false, finished: nil), .healthy)
    }

    func testAStatusThatNeedsTheUserIsLeftAlone() {
        XCTAssertEqual(action(ready: false, resumable: false, finished: nil), .healthy)
    }

    // MARK: the stale line

    private func stale(lastSuccessHoursAgo: Double? = 3, finished: Date? = nil, activity: Date? = nil, roundMinutes: Double? = nil) -> Bool {
        SyncTiming.isStale(now: now, lastSuccess: lastSuccessHoursAgo.map { now.addingTimeInterval(-$0 * 3600) },
                           lastRoundFinishedAt: finished, lastActivityAt: activity,
                           roundStartedAt: roundMinutes.map { ago($0) })
    }

    func testNoRoundsRunningForHoursIsStale() {
        // Four days of nothing: no failures counted, no round started, none finished.
        XCTAssertTrue(stale(lastSuccessHoursAgo: 96, finished: nil))
        XCTAssertTrue(stale(lastSuccessHoursAgo: 96, finished: ago(60 * 96)))
    }

    func testAFailedRoundSinceTheLastSuccessIsStale() {
        XCTAssertTrue(stale(lastSuccessHoursAgo: 2.5, finished: ago(1)))
    }

    func testAMacThatJustWokeIsNotStaleUntilTenMinutesPass() {
        XCTAssertFalse(stale(lastSuccessHoursAgo: 12, finished: ago(60 * 12), activity: ago(1)))
        XCTAssertTrue(stale(lastSuccessHoursAgo: 12, finished: ago(60 * 12), activity: ago(11)))
    }

    func testAStuckRoundIsStale() {
        XCTAssertTrue(stale(finished: ago(180), activity: ago(1), roundMinutes: 6))
        XCTAssertFalse(stale(finished: ago(180), activity: ago(1), roundMinutes: 1))
    }

    func testARecentSuccessIsNeverStale() {
        XCTAssertFalse(stale(lastSuccessHoursAgo: 1.9, finished: ago(60 * 5)))
    }

    func testNeverSyncedIsNotStale() {
        XCTAssertFalse(stale(lastSuccessHoursAgo: nil, finished: nil))
    }
}
