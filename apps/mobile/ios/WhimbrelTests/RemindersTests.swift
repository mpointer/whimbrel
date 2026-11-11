import XCTest
@testable import Whimbrel

final class RemindersTests: XCTestCase {
    private var defaults: UserDefaults!

    override func setUp() {
        super.setUp()
        defaults = UserDefaults(suiteName: "RemindersTests-\(UUID().uuidString)")
    }

    private func plan(_ name: String) -> AccountPlan {
        AccountPlan(plan: name, until: nil, interval: nil, billing: true)
    }

    func testPlanCacheKeepsValueWhenFetchFails() {
        let cache = PlanCache(defaults: defaults)
        XCTAssertFalse(cache.isCloud)
        cache.update(plan("cloud"))
        cache.update(nil)
        XCTAssertTrue(cache.isCloud)
        XCTAssertTrue(PlanCache(defaults: defaults).isCloud, "survives a relaunch")
    }

    func testPlanCacheClearsWhenPlanIsFree() {
        let cache = PlanCache(defaults: defaults)
        cache.update(plan("cloud"))
        cache.update(plan("free"))
        XCTAssertFalse(cache.isCloud)
    }

    func testSettingsDefaultsAreOn() {
        let settings = ReminderSettingsStore(defaults: defaults).settings
        XCTAssertTrue(settings.meetingEnabled)
        XCTAssertEqual(settings.leadMinutes, 10)
        XCTAssertTrue(settings.digestEnabled)
        XCTAssertEqual(settings.digestMinuteOfDay, 480)
    }

    func testSettingsRoundTrip() {
        let store = ReminderSettingsStore(defaults: defaults)
        let changed = ReminderSettings(meetingEnabled: false, leadMinutes: 15, digestEnabled: false, digestMinuteOfDay: 555)
        store.settings = changed
        XCTAssertEqual(ReminderSettingsStore(defaults: defaults).settings, changed)
    }

    func testThrottleRemembersLastCheck() {
        let cache = PlanCache(defaults: defaults)
        XCTAssertNil(cache.lastCheckedAt)
        let when = Date(timeIntervalSince1970: 1_000_000)
        cache.lastCheckedAt = when
        XCTAssertEqual(PlanCache(defaults: defaults).lastCheckedAt, when)
    }
}

/// Records what the coordinator hands the system.
@MainActor
final class FakeScheduler: ReminderScheduling {
    private(set) var replaced: [[PlannedReminder]] = []
    private(set) var permissionAsks = 0
    func replaceAll(_ reminders: [PlannedReminder]) async { replaced.append(reminders) }
    func requestPermissionIfNeeded() async -> Bool { permissionAsks += 1; return true }
}

@MainActor
final class ReminderCoordinatorTests: XCTestCase {
    private func make(cloud: Bool, defaults: UserDefaults? = nil, scheduler: FakeScheduler? = nil,
                      fetchPlan: (() async -> AccountPlan?)? = nil,
                      readReplica: ((Int64, Int64) async -> ReminderCoordinator.ReplicaRead?)? = nil) -> (ReminderCoordinator, FakeScheduler, PlanCache) {
        let defaults = defaults ?? UserDefaults(suiteName: "ReminderCoordinatorTests-\(UUID().uuidString)")!
        let scheduler = scheduler ?? FakeScheduler()
        let cache = PlanCache(defaults: defaults)
        if cloud { cache.update(AccountPlan(plan: "cloud", until: nil, interval: nil, billing: true)) }
        let coordinator = ReminderCoordinator(
            sync: SyncModel(store: nil, credentials: FakeCredentials()),
            enrollment: EnrollmentModel(backend: FakeBackend()),
            scheduler: scheduler,
            settings: ReminderSettingsStore(defaults: defaults),
            plan: cache,
            fetchPlan: fetchPlan,
            readReplica: readReplica,
            now: { Date(timeIntervalSince1970: 1_800_000_000) })
        return (coordinator, scheduler, cache)
    }

    func testCoordinatorClearsRemindersOffCloud() async {
        let (coordinator, scheduler, _) = make(cloud: false)
        await coordinator.reschedule()
        XCTAssertEqual(scheduler.replaced.count, 1)
        XCTAssertEqual(scheduler.replaced.first, [])
        XCTAssertEqual(scheduler.permissionAsks, 0)
    }

    func testCoordinatorLeavesRemindersAloneWhenStoreIsNotOpen() async {
        let (coordinator, scheduler, _) = make(cloud: true)
        await coordinator.reschedule()
        XCTAssertTrue(scheduler.replaced.isEmpty)
    }

    func testFailedPlanCheckKeepsCachedPlanAndSkipsThrottleStamp() async {
        // Not approved, so fetchAccountPlan is nil.
        let (coordinator, _, cache) = make(cloud: true)
        await coordinator.refreshPlanIfDue(force: true)
        XCTAssertTrue(coordinator.isCloud)
        XCTAssertTrue(cache.isCloud)
        XCTAssertNil(cache.lastCheckedAt)
    }

    func testRecentCheckThrottlesNonForcedRefresh() async {
        let (coordinator, scheduler, cache) = make(cloud: false)
        cache.lastCheckedAt = Date(timeIntervalSince1970: 1_800_000_000 - 60)
        await coordinator.refreshPlanIfDue(force: false)
        XCTAssertEqual(cache.lastCheckedAt, Date(timeIntervalSince1970: 1_800_000_000 - 60))
        XCTAssertTrue(scheduler.replaced.isEmpty)
    }

    private func cloudPlan() -> AccountPlan { AccountPlan(plan: "cloud", until: nil, interval: nil, billing: true) }
    private func freePlan() -> AccountPlan { AccountPlan(plan: "free", until: nil, interval: nil, billing: true) }

    func testPermissionAskedOnceWhenCloudInForeground() async {
        let defaults = UserDefaults(suiteName: "perm-\(UUID().uuidString)")!
        let (coordinator, scheduler, _) = make(cloud: false, defaults: defaults, fetchPlan: { self.cloudPlan() }, readReplica: { _, _ in ([], []) })
        coordinator.isForeground = true
        await coordinator.refreshPlanIfDue(force: true)
        await coordinator.refreshPlanIfDue(force: true)
        XCTAssertEqual(scheduler.permissionAsks, 1)
        let (again, scheduler2, _) = make(cloud: true, defaults: defaults, fetchPlan: { self.cloudPlan() }, readReplica: { _, _ in ([], []) })
        again.isForeground = true
        await again.permissionTask?.value
        await again.refreshPlanIfDue(force: true)
        XCTAssertEqual(scheduler2.permissionAsks, 0)
    }

    func testDemoCoordinatorNeverSchedulesOrAsks() async {
        var fetches = 0
        let (coordinator, scheduler, _) = make(cloud: true, fetchPlan: { fetches += 1; return self.cloudPlan() }, readReplica: { _, _ in ([], []) })
        coordinator.isEnabled = false
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        await coordinator.refreshPlanIfDue(force: true)
        await coordinator.reschedule()
        XCTAssertTrue(scheduler.replaced.isEmpty)
        XCTAssertEqual(scheduler.permissionAsks, 0)
        XCTAssertEqual(fetches, 0)
    }

    func testPermissionDeferredUntilForeground() async {
        let (coordinator, scheduler, _) = make(cloud: false, fetchPlan: { self.cloudPlan() }, readReplica: { _, _ in ([], []) })
        await coordinator.refreshPlanIfDue(force: true)
        XCTAssertEqual(scheduler.permissionAsks, 0)
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        XCTAssertEqual(scheduler.permissionAsks, 1)
    }

    func testCloudToFreeClearsReminders() async {
        let (coordinator, scheduler, cache) = make(cloud: true, fetchPlan: { self.freePlan() }, readReplica: { _, _ in ([], []) })
        await coordinator.refreshPlanIfDue(force: true)
        XCTAssertFalse(cache.isCloud)
        XCTAssertEqual(scheduler.replaced.last, [])
    }

    func testOverlappingNonForcedChecksMakeOneRequest() async {
        var fetches = 0
        var release: CheckedContinuation<Void, Never>?
        let (coordinator, _, _) = make(cloud: false, fetchPlan: {
            fetches += 1
            await withCheckedContinuation { release = $0 }
            return self.freePlan()
        }, readReplica: { _, _ in ([], []) })
        let first = Task { await coordinator.refreshPlanIfDue(force: false) }
        while release == nil { await Task.yield() }
        await coordinator.refreshPlanIfDue(force: false)
        release?.resume()
        await first.value
        XCTAssertEqual(fetches, 1)
    }

    func testStaleRescheduleDoesNotOverwriteNewer() async {
        var calls = 0
        var gate: CheckedContinuation<Void, Never>?
        let (coordinator, scheduler, _) = make(cloud: true, readReplica: { _, _ in
            calls += 1
            if calls == 1 { await withCheckedContinuation { gate = $0 } }
            return ([], [])
        })
        let older = Task { await coordinator.reschedule() }
        while gate == nil { await Task.yield() }
        await coordinator.reschedule()
        XCTAssertEqual(scheduler.replaced.count, 1)
        gate?.resume()
        await older.value
        XCTAssertEqual(scheduler.replaced.count, 1, "the older run must not land after the newer one")
    }
}

final class ReminderSettingsRowsTests: XCTestCase {
    func testOffCloudShowsUpgradeLinesOnly() {
        let rows = ReminderSettingsRows.make(isCloud: false, settings: ReminderSettings(), denied: true)
        XCTAssertTrue(rows.showsUpgradeLines)
        XCTAssertFalse(rows.showsMeetingToggle)
        XCTAssertFalse(rows.showsLeadPicker)
        XCTAssertFalse(rows.showsDigestToggle)
        XCTAssertFalse(rows.showsDigestTime)
        XCTAssertFalse(rows.showsDeniedNotice)
        XCTAssertFalse(rows.showsFooter)
    }

    func testCloudShowsControlsAndFooter() {
        let rows = ReminderSettingsRows.make(isCloud: true, settings: ReminderSettings(), denied: false)
        XCTAssertFalse(rows.showsUpgradeLines)
        XCTAssertTrue(rows.showsMeetingToggle)
        XCTAssertTrue(rows.showsLeadPicker)
        XCTAssertTrue(rows.showsDigestToggle)
        XCTAssertTrue(rows.showsDigestTime)
        XCTAssertTrue(rows.showsFooter)
    }

    func testLeadPickerHiddenWhenMeetingRemindersOff() {
        var settings = ReminderSettings()
        settings.meetingEnabled = false
        let rows = ReminderSettingsRows.make(isCloud: true, settings: settings, denied: false)
        XCTAssertTrue(rows.showsMeetingToggle)
        XCTAssertFalse(rows.showsLeadPicker)
    }

    func testDigestTimeHiddenWhenStepsDueTodayOff() {
        var settings = ReminderSettings()
        settings.digestEnabled = false
        let rows = ReminderSettingsRows.make(isCloud: true, settings: settings, denied: false)
        XCTAssertTrue(rows.showsDigestToggle)
        XCTAssertFalse(rows.showsDigestTime)
    }

    func testDeniedNoticeOnlyWhenDenied() {
        XCTAssertTrue(ReminderSettingsRows.make(isCloud: true, settings: ReminderSettings(), denied: true).showsDeniedNotice)
        XCTAssertFalse(ReminderSettingsRows.make(isCloud: true, settings: ReminderSettings(), denied: false).showsDeniedNotice)
    }

    func testDigestTimeRoundTripsMinuteOfDay() {
        let calendar = Calendar.current
        let day = Date(timeIntervalSince1970: 1_800_000_000)
        for minute in [0, 480, 725, 1439] {
            let date = ReminderSettingsRows.date(forMinuteOfDay: minute, on: day, calendar: calendar)
            XCTAssertEqual(ReminderSettingsRows.minuteOfDay(of: date, calendar: calendar), minute)
        }
    }
}
