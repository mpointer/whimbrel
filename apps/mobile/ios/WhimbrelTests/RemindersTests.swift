import XCTest
@testable import Whimbrel

final class RemindersTests: XCTestCase {
    private var defaults: UserDefaults!

    override func setUp() {
        super.setUp()
        defaults = UserDefaults(suiteName: "RemindersTests-\(UUID().uuidString)")
    }

    func testPermissionAskedSurvivesARelaunch() {
        let state = ReminderStateStore(defaults: defaults)
        XCTAssertFalse(state.permissionAsked)
        state.permissionAsked = true
        XCTAssertTrue(ReminderStateStore(defaults: defaults).permissionAsked)
        XCTAssertTrue(defaults.bool(forKey: "whimbrel.reminders.permissionAsked"), "the key a phone already has is kept")
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
    private func make(approved: Bool = true, defaults: UserDefaults? = nil, scheduler: FakeScheduler? = nil,
                      isApproved: (() -> Bool)? = nil,
                      readReplica: ((Int64, Int64) async -> ReminderCoordinator.ReplicaRead?)? = nil) -> (ReminderCoordinator, FakeScheduler) {
        let defaults = defaults ?? UserDefaults(suiteName: "ReminderCoordinatorTests-\(UUID().uuidString)")!
        let scheduler = scheduler ?? FakeScheduler()
        let coordinator = ReminderCoordinator(
            sync: SyncModel(store: nil, credentials: FakeCredentials()),
            enrollment: EnrollmentModel(backend: FakeBackend()),
            scheduler: scheduler,
            settings: ReminderSettingsStore(defaults: defaults),
            state: ReminderStateStore(defaults: defaults),
            isApproved: isApproved ?? { approved },
            readReplica: readReplica,
            now: { Date(timeIntervalSince1970: 1_800_000_000) })
        return (coordinator, scheduler)
    }

    private func soonEvent() -> PhoneCalendarEvent {
        PhoneCalendarEvent(externalId: "e1", source: "google_calendar", title: "Soon", startUnixMs: 1_800_003_600_000, attendees: [], rowId: 1)
    }

    func testApprovedPhoneGetsRemindersWithNoPlanCheck() async {
        // No plan is fetched or cached: a Free account schedules the same reminders as Cloud.
        let event = soonEvent()
        let (coordinator, scheduler) = make(readReplica: { _, _ in ([event], []) })
        await coordinator.reschedule()
        XCTAssertEqual(scheduler.replaced.count, 1)
        XCTAssertEqual(scheduler.replaced.first?.map(\.id), ["meeting:e1"])
    }

    func testPhoneNotApprovedSchedulesNothingAndIsNotAsked() async {
        var reads = 0
        let (coordinator, scheduler) = make(approved: false, readReplica: { _, _ in reads += 1; return ([], []) })
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        await coordinator.reschedule()
        await coordinator.askPermissionIfNeeded()
        XCTAssertTrue(scheduler.replaced.isEmpty)
        XCTAssertEqual(scheduler.permissionAsks, 0)
        XCTAssertEqual(reads, 0)
    }

    func testCoordinatorLeavesRemindersAloneWhenStoreIsNotOpen() async {
        let (coordinator, scheduler) = make()
        await coordinator.reschedule()
        XCTAssertTrue(scheduler.replaced.isEmpty)
    }

    func testPermissionAskedOnceInForeground() async {
        let defaults = UserDefaults(suiteName: "perm-\(UUID().uuidString)")!
        let (coordinator, scheduler) = make(defaults: defaults, readReplica: { _, _ in ([], []) })
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        await coordinator.askPermissionIfNeeded()
        XCTAssertEqual(scheduler.permissionAsks, 1)
        let (again, scheduler2) = make(defaults: defaults, readReplica: { _, _ in ([], []) })
        again.isForeground = true
        await again.permissionTask?.value
        await again.askPermissionIfNeeded()
        XCTAssertEqual(scheduler2.permissionAsks, 0)
    }

    func testDemoCoordinatorNeverSchedulesOrAsks() async {
        let (coordinator, scheduler) = make(readReplica: { _, _ in ([], []) })
        coordinator.isEnabled = false
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        await coordinator.askPermissionIfNeeded()
        await coordinator.reschedule()
        XCTAssertTrue(scheduler.replaced.isEmpty)
        XCTAssertEqual(scheduler.permissionAsks, 0)
    }

    func testPermissionDeferredUntilForeground() async {
        let (coordinator, scheduler) = make(readReplica: { _, _ in ([], []) })
        await coordinator.askPermissionIfNeeded()
        XCTAssertEqual(scheduler.permissionAsks, 0)
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        XCTAssertEqual(scheduler.permissionAsks, 1)
    }

    func testPhoneApprovedWhileOpenIsAskedAfterSync() async {
        // Foreground before approval: nothing then. The per-sync ask catches it once approved.
        var approved = false
        let (coordinator, scheduler) = make(isApproved: { approved }, readReplica: { _, _ in ([], []) })
        coordinator.isForeground = true
        await coordinator.permissionTask?.value
        XCTAssertEqual(scheduler.permissionAsks, 0)
        approved = true
        await coordinator.askPermissionIfNeeded()
        XCTAssertEqual(scheduler.permissionAsks, 1)
    }

    func testRunThatOutlivesApprovalDoesNotLand() async {
        var approved = true
        var gate: CheckedContinuation<Void, Never>?
        let (coordinator, scheduler) = make(isApproved: { approved }, readReplica: { _, _ in
            await withCheckedContinuation { gate = $0 }
            return ([], [])
        })
        let run = Task { await coordinator.reschedule() }
        while gate == nil { await Task.yield() }
        approved = false
        gate?.resume()
        await run.value
        XCTAssertTrue(scheduler.replaced.isEmpty)
    }

    func testStaleRescheduleDoesNotOverwriteNewer() async {
        var calls = 0
        var gate: CheckedContinuation<Void, Never>?
        let (coordinator, scheduler) = make(readReplica: { _, _ in
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
    func testEveryPlanShowsControlsAndFooter() {
        let rows = ReminderSettingsRows.make(settings: ReminderSettings(), denied: false)
        XCTAssertTrue(rows.showsMeetingToggle)
        XCTAssertTrue(rows.showsLeadPicker)
        XCTAssertTrue(rows.showsDigestToggle)
        XCTAssertTrue(rows.showsDigestTime)
        XCTAssertFalse(rows.showsDeniedNotice)
        XCTAssertTrue(rows.showsFooter)
    }

    func testLeadPickerHiddenWhenMeetingRemindersOff() {
        var settings = ReminderSettings()
        settings.meetingEnabled = false
        let rows = ReminderSettingsRows.make(settings: settings, denied: false)
        XCTAssertTrue(rows.showsMeetingToggle)
        XCTAssertFalse(rows.showsLeadPicker)
    }

    func testDigestTimeHiddenWhenStepsDueTodayOff() {
        var settings = ReminderSettings()
        settings.digestEnabled = false
        let rows = ReminderSettingsRows.make(settings: settings, denied: false)
        XCTAssertTrue(rows.showsDigestToggle)
        XCTAssertFalse(rows.showsDigestTime)
    }

    func testDeniedNoticeOnlyWhenDenied() {
        XCTAssertTrue(ReminderSettingsRows.make(settings: ReminderSettings(), denied: true).showsDeniedNotice)
        XCTAssertFalse(ReminderSettingsRows.make(settings: ReminderSettings(), denied: false).showsDeniedNotice)
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
