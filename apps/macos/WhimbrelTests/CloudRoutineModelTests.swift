import XCTest
@testable import Whimbrel

final class CloudRoutineModelTests: XCTestCase {
    private let zone = TimeZone(identifier: "America/New_York")!
    private var calendar: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = zone
        return c
    }

    private func at(_ day: Int, _ hour: Int, _ minute: Int) -> Date {
        calendar.date(from: DateComponents(year: 2026, month: 11, day: day, hour: hour, minute: minute))!
    }

    private func run(_ date: Date, _ status: String = "success") -> CloudRunStamp {
        CloudRunStamp(ranAtUnixMs: Int64(date.timeIntervalSince1970 * 1000), status: status)
    }

    func testTheToggleIsHiddenForReplyPrioritiesAndFocusTime() {
        for template in ["reply_priorities", "focus_time", "away_follow_up", "someday"] {
            XCTAssertFalse(CloudRoutineCopy.showsToggle(CloudRoutineInfo.load(template)), template)
        }
        for template in ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"] {
            XCTAssertTrue(CloudRoutineCopy.showsToggle(CloudRoutineInfo.load(template)), template)
        }
        XCTAssertFalse(CloudRoutineCopy.showsToggle(nil))
    }

    func testTheToggleCopyContainsTheGrantSentenceAndTheKeyHoldingSentence() throws {
        let brief = try XCTUnwrap(CloudRoutineInfo.load("daily_brief"))
        let copy = CloudRoutineCopy.toggleCopy(brief)
        XCTAssertTrue(copy.hasPrefix("Whimbrel Cloud holds your sync key. While this runs it can read your \(brief.grantList)."), copy)
        XCTAssertFalse(copy.contains("While this runs, Whimbrel Cloud holds"), "the key is held at all times, not only during a run")
        // Core's words, literally, so a stale core library cannot pass this.
        XCTAssertEqual(brief.googleCalendarLimit,
                       "Events from your Google Calendar connection are not read by the cloud. Calendars added to your computer's Calendar app are.")
        XCTAssertTrue(copy.contains("It keeps nothing after the run."), copy)
        XCTAssertTrue(brief.grantList.contains("meetings"), "core's words: \(brief.grantList)")
        XCTAssertTrue(copy.contains(brief.googleCalendarLimit) && !brief.googleCalendarLimit.isEmpty, "the brief reads calendar events: \(copy)")
        let recap = try XCTUnwrap(CloudRoutineInfo.load("eod_recap"))
        XCTAssertFalse(CloudRoutineCopy.toggleCopy(recap).contains("Google Calendar"), "the recap reads no calendar events")
        XCTAssertTrue(CloudRoutineCopy.disclosure.contains("Results already made are not recalled if you later delete a client."))
        XCTAssertTrue(CloudRoutineCopy.disclosure.contains("items you filed by hand are not known to the cloud"))
        for promise in ["nothing leaves your computer", "never leaves", "we never receive", "rotat"] {
            for text in [copy, CloudRoutineCopy.disclosure, CloudRoutineCopy.readyByNote, CloudRoutineCopy.removeRunnerMessage,
                         CloudRoutineCopy.noRunner, CloudRoutineCopy.noRunnerPrep] {
                XCTAssertFalse(text.lowercased().contains(promise), "\(promise) in \(text)")
            }
        }
    }

    func testALateRunShowsTheLastGoodResultWithItsTime() {
        let now = at(16, 8, 15)
        let yesterday = at(15, 6, 58)
        // Today's run is missing past the 07:30 ready-by: the last good result shows, with its day and time.
        let late = CloudRoutineCopy.freshness(runs: [run(yesterday), run(at(16, 7, 40), "failed")], readyBy: "07:30", now: now, calendar: calendar)
        XCTAssertEqual(late, .late(lastGood: yesterday))
        let text = CloudRoutineCopy.label(late, now: now, calendar: calendar, locale: Locale(identifier: "en_US"))
        XCTAssertEqual(text, "Late: showing the last result from \(CloudRoutineCopy.when(yesterday, now: now, calendar: calendar, locale: Locale(identifier: "en_US")))")
        XCTAssertTrue(text?.contains("6:58") == true, text ?? "")

        // Before the ready-by time it is not late yet.
        XCTAssertEqual(CloudRoutineCopy.freshness(runs: [run(yesterday)], readyBy: "09:00", now: now, calendar: calendar), .updated(yesterday))
        // Today's run: "Updated 6:58 am" (the time only).
        let today = at(16, 6, 58)
        let fresh = CloudRoutineCopy.freshness(runs: [run(yesterday), run(today)], readyBy: "07:30", now: now, calendar: calendar)
        XCTAssertEqual(fresh, .updated(today))
        let updated = CloudRoutineCopy.label(fresh, now: now, calendar: calendar, locale: Locale(identifier: "en_US")) ?? ""
        XCTAssertTrue(updated.hasPrefix("Updated "), updated)
        XCTAssertTrue(updated.contains("6:58"), updated)
        XCTAssertFalse(updated.contains("Nov"), "today's run shows the time only: \(updated)")
        // A run left for the computer (a failed row) is never shown as a result.
        XCTAssertEqual(CloudRoutineCopy.freshness(runs: [run(at(16, 6, 0), "failed")], readyBy: "07:30", now: now, calendar: calendar), .late(lastGood: nil))
        XCTAssertEqual(CloudRoutineCopy.label(.late(lastGood: nil), now: now, calendar: calendar, locale: Locale(identifier: "en_US")),
                       "Late: no result yet")
        XCTAssertEqual(CloudRoutineCopy.freshness(runs: [], readyBy: "09:00", now: now, calendar: calendar), .none)
        XCTAssertNil(CloudRoutineCopy.label(.none, now: now, calendar: calendar, locale: Locale(identifier: "en_US")))
        // An unreadable ready-by is never late.
        XCTAssertEqual(CloudRoutineCopy.freshness(runs: [run(yesterday)], readyBy: "soon", now: now, calendar: calendar), .updated(yesterday))
    }

    /// Only the daily routines that are on can be late: the Weekly Digest runs on Sundays only and Meeting Prep
    /// before each meeting, and a routine that is off is never expected.
    func testOnlyEnabledDailyRoutinesCanBeLateAndTheDefaultReadyByIsAnHourAfterTheirTime() {
        XCTAssertTrue(CloudRoutineCopy.canBeLate(template: "daily_brief", enabled: true))
        XCTAssertTrue(CloudRoutineCopy.canBeLate(template: "eod_recap", enabled: true))
        XCTAssertFalse(CloudRoutineCopy.canBeLate(template: "daily_brief", enabled: false))
        XCTAssertFalse(CloudRoutineCopy.canBeLate(template: "weekly_digest", enabled: true))
        XCTAssertFalse(CloudRoutineCopy.canBeLate(template: "meeting_prep", enabled: true))
        XCTAssertEqual(CloudRoutineCopy.defaultReadyBy(schedule: "08:30"), "09:30")
        XCTAssertEqual(CloudRoutineCopy.defaultReadyBy(schedule: "23:30"), "23:59")
        XCTAssertNil(CloudRoutineCopy.defaultReadyBy(schedule: "soon"))
        // The pane passes no ready-by for a routine that cannot be late: yesterday's result is "Updated", never "Late".
        let yesterday = at(15, 6, 58)
        XCTAssertEqual(CloudRoutineCopy.freshness(runs: [run(yesterday)], readyBy: "", now: at(16, 20, 0), calendar: calendar), .updated(yesterday))
    }

    /// Handing a routine to the cloud stamps this Mac's on/off, time and model, so the runner reads them rather than
    /// whichever device's untouched value reached the relay first.
    @MainActor
    func testTurningCloudOnStampsTheRoutinesOwnFieldsForTheRunner() {
        let suite = "CloudRoutineModelTests.stamp"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        let cloud = CloudRoutineSettings(defaults: defaults)
        let keys = ["enabled", "schedule", "model_pref"].map { "routine.weekly_digest.\($0)" }
        for key in keys { SettingsFieldTimestamps.setTimestamp(0, for: key) }
        cloud.setCloud(true, for: "weekly_digest", routineEnabled: true)
        for key in keys { XCTAssertGreaterThan(SettingsFieldTimestamps.timestamp(for: key), 0, key) }
        XCTAssertEqual(cloud.cloudTemplates, ["weekly_digest"])
        cloud.setCloud(true, for: "reply_priorities", routineEnabled: true)
        XCTAssertFalse(cloud.isCloud("reply_priorities"), "never in the cloud")
    }

    func testTheToggleIsDisabledWhenSyncIsNotReady() {
        let reason = CloudRoutineCopy.blockedReason(syncReady: false, routineEnabled: true, localOnly: false, hasSyncedKey: true)
        XCTAssertNotNil(reason, "a synced key alone is not enough: sync must be on")
        XCTAssertTrue(reason?.contains("sync") == true, reason ?? "")
        XCTAssertNotNil(CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: true, hasSyncedKey: true))
        XCTAssertNotNil(CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: false, hasSyncedKey: false))
        XCTAssertNil(CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: false, hasSyncedKey: true))
    }

    func testTheNoRunnerNoteShowsWhenTheDeviceListIsEmpty() {
        func device(_ platform: String?, _ status: String) -> SyncDeviceRecord {
            SyncDeviceRecord(id: UUID().uuidString, accountId: "a", publicKey: UUID().uuidString, name: nil, platform: platform,
                             status: status, enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
        }
        XCTAssertTrue(CloudRoutineCopy.runnerMissing(devices: []))
        XCTAssertTrue(CloudRoutineCopy.runnerMissing(devices: [device("mac", "approved"), device("cloud", "revoked")]))
        XCTAssertFalse(CloudRoutineCopy.runnerMissing(devices: [device("mac", "approved"), device("cloud", "approved")]))
    }

    func testAlreadyOnWithSyncOffShowsTheSyncReasonNotTheNoRunnerNote() {
        let reason = CloudRoutineCopy.blockedReason(syncReady: false, routineEnabled: true, localOnly: false, hasSyncedKey: true)
        let note = CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: true)
        XCTAssertEqual(note, reason)
        XCTAssertNotEqual(note, CloudRoutineCopy.noRunner)
    }

    func testAlreadyOnWithSyncReadyAndNoRunnerShowsTheNoRunnerNote() {
        let reason = CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: false, hasSyncedKey: true)
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: true), CloudRoutineCopy.noRunner)
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: true, template: "daily_brief"), CloudRoutineCopy.noRunner)
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: true, template: "meeting_prep"), CloudRoutineCopy.noRunnerPrep)
        XCTAssertNil(CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: false))
        XCTAssertNil(CloudRoutineCopy.statusNote(on: false, blockedReason: reason, runnerMissing: true), "off and nothing blocks: no note")
    }

    func testAlreadyOnWithLocalOnlyShowsTheLocalOnlyReason() {
        let reason = CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: true, hasSyncedKey: true)
        XCTAssertTrue(reason?.contains("Local-only") == true)
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: true), reason)
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: false, blockedReason: reason, runnerMissing: false), reason)
    }

    // MARK: Task 6b: ready-by and Meeting Prep in the cloud reach core

    func testTheCloudPolicyJSONCarriesTheReadyByTimes() throws {
        let json = try XCTUnwrap(CoreMemoryStore.cloudPolicyJSON(templates: ["daily_brief"], readyBy: ["daily_brief": "07:00"]))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object["templates"] as? [String], ["daily_brief"])
        XCTAssertEqual(object["ready_by"] as? [String: String], ["daily_brief": "07:00"])
    }

    func testAFlaggedMeetingPrepWaitsOnThisMacUntilFiveMinutesBeforeItsStart() throws {
        let routing = CoreRoutingConfig(
            anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
            userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: "ollama", preferredModel: "no-such-model",
            customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""
        )
        let start = Date(timeIntervalSince1970: 1_900_000_000)
        let meeting = QualifyingMeeting(
            id: "EVT-P", title: "Planning", startDate: start, endDate: start.addingTimeInterval(1800), attendees: ["Ana"],
            conferenceLink: nil, source: .calendar, appBundleID: nil, organizerEmail: nil, organizerName: nil,
            selfIsOrganizer: false, selfResponse: nil
        )
        func ms(_ minutesBefore: Double) -> Int64 { Int64(start.addingTimeInterval(-minutesBefore * 60).timeIntervalSince1970 * 1000) }
        func prepStore() throws -> (CoreMemoryStore, URL) {
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let store = try XCTUnwrap(CoreMemoryStore(path: dir.appendingPathComponent("test.sqlite"), modelsDir: nil))
            // Seeds the built-in routines without running any.
            _ = store.routinesTick(nowUnixMs: ms(60), ollamaModel: "no-such-model", routing: routing, todayEvents: [],
                                   gmailConnected: false, recipientEmail: nil, gate: .off)
            let prep = try XCTUnwrap(store.listRoutines()?.first { $0.template == "meeting_prep" })
            XCTAssertTrue(store.setRoutineEnabled(id: prep.id, enabled: true))
            return (store, dir)
        }
        func prep(_ store: CoreMemoryStore, _ minutesBefore: Double, cloud: [String]) -> [CoreRoutineRun]? {
            store.meetingPrepRun(nowUnixMs: ms(minutesBefore), leadMinutes: 30, ollamaModel: "no-such-model", routing: routing,
                                 meetings: [meeting], gate: .full, cloudTemplates: cloud)
        }

        let (flagged, flaggedDir) = try prepStore()
        defer { try? FileManager.default.removeItem(at: flaggedDir) }
        XCTAssertEqual(prep(flagged, 20, cloud: ["meeting_prep"])?.count, 0, "left to the runner")
        XCTAssertEqual(prep(flagged, 5, cloud: ["meeting_prep"])?.count, 1, "nothing synced in: this Mac prepares it")

        let (unflagged, unflaggedDir) = try prepStore()
        defer { try? FileManager.default.removeItem(at: unflaggedDir) }
        XCTAssertEqual(prep(unflagged, 20, cloud: ["daily_brief"])?.count, 1, "not flagged: as before")

        // The runner never reads Google Calendar, so a Google meeting is prepared here at lead time.
        let google = QualifyingMeeting(
            id: "EVT-G", title: "Google planning", startDate: start, endDate: start.addingTimeInterval(1800), attendees: ["Ana"],
            conferenceLink: nil, source: .googleCalendar, appBundleID: nil, organizerEmail: nil, organizerName: nil,
            selfIsOrganizer: false, selfResponse: nil
        )
        let (googleStore, googleDir) = try prepStore()
        defer { try? FileManager.default.removeItem(at: googleDir) }
        let googleRuns = googleStore.meetingPrepRun(nowUnixMs: ms(20), leadMinutes: 30, ollamaModel: "no-such-model", routing: routing,
                                                    meetings: [google], gate: .full, cloudTemplates: ["meeting_prep"])
        XCTAssertEqual(googleRuns?.count, 1, "Google Calendar meeting: prepared here as before")
    }

    func testTheToggleIsBlockedWithTurnThisRoutineOnFirstWhenTheRoutineIsOff() {
        let reason = CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: false, localOnly: false, hasSyncedKey: true)
        XCTAssertEqual(reason, "Turn this routine on first.")
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: false, blockedReason: reason, runnerMissing: true), reason)
        XCTAssertEqual(CloudRoutineCopy.statusNote(on: true, blockedReason: reason, runnerMissing: true), reason)
    }

    func testTheOrderIsSyncOffThenRoutineOffThenLocalOnlyThenNoKey() {
        let syncOff = CloudRoutineCopy.blockedReason(syncReady: false, routineEnabled: false, localOnly: true, hasSyncedKey: false)
        XCTAssertTrue(syncOff?.hasPrefix("Turn on sync") == true, syncOff ?? "")
        XCTAssertEqual(CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: false, localOnly: true, hasSyncedKey: false), "Turn this routine on first.")
        let localOnly = CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: true, hasSyncedKey: false)
        XCTAssertTrue(localOnly?.hasPrefix("Local-only") == true, localOnly ?? "")
        let noKey = CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: false, hasSyncedKey: false)
        XCTAssertTrue(noKey?.contains("provider key you sync") == true, noKey ?? "")
        XCTAssertNil(CloudRoutineCopy.blockedReason(syncReady: true, routineEnabled: true, localOnly: false, hasSyncedKey: true))
    }

    @MainActor
    func testTurningItOffIsAlwaysAllowed() {
        let suite = "CloudRoutineModelTests.off"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        let cloud = CloudRoutineSettings(defaults: defaults)
        cloud.setCloud(true, for: "daily_brief", routineEnabled: true)
        XCTAssertTrue(cloud.isCloud("daily_brief"))
        // The routine was later turned off (here or on another device): the cloud toggle still turns off.
        cloud.setCloud(false, for: "daily_brief", routineEnabled: false)
        XCTAssertFalse(cloud.isCloud("daily_brief"))
    }

    /// Turning cloud on for a routine that is off is refused and stamps nothing, so this Mac never pushes
    /// enabled=false over another device's on.
    @MainActor
    func testTurningOnADisabledRoutineStampsNothing() {
        let suite = "CloudRoutineModelTests.disabled"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        let cloud = CloudRoutineSettings(defaults: defaults)
        let keys = ["enabled", "schedule", "model_pref", "cloud", "cloudGrant"].map { "routine.eod_recap.\($0)" }
        for key in keys { SettingsFieldTimestamps.setTimestamp(0, for: key) }
        cloud.setCloud(true, for: "eod_recap", routineEnabled: false)
        XCTAssertFalse(cloud.isCloud("eod_recap"))
        for key in keys { XCTAssertEqual(SettingsFieldTimestamps.timestamp(for: key), 0, key) }
    }

    // MARK: Final review C1: the hidden switch, and the grace in the words

    private func device(_ platform: String?, _ status: String) -> SyncDeviceRecord {
        SyncDeviceRecord(id: UUID().uuidString, accountId: "a", publicKey: UUID().uuidString, name: nil, platform: platform,
                         status: status, enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
    }

    func testWithTheSwitchOffAndNoRunnerNothingNewShows() {
        for devices in [[], [device("mac", "approved")], [device("mac", "approved"), device("cloud", "revoked")]] {
            XCTAssertFalse(CloudRoutineCopy.showsCloudSection(flag: false, devices: devices, on: false))
        }
        XCTAssertFalse(CloudRoutineCopy.showsAddRunner(flag: false))
    }

    func testWithTheSwitchOnTheToggleAndAddShow() {
        XCTAssertTrue(CloudRoutineCopy.showsCloudSection(flag: true, devices: [], on: false))
        XCTAssertTrue(CloudRoutineCopy.showsAddRunner(flag: true))
    }

    func testARunnerOnTheAccountShowsTheToggleButNotAdd() {
        let devices = [device("mac", "approved"), device("cloud", "approved")]
        XCTAssertTrue(CloudRoutineCopy.showsCloudSection(flag: false, devices: devices, on: false))
        XCTAssertFalse(CloudRoutineCopy.showsAddRunner(flag: false))
    }

    func testWithTheSwitchOffARunnerCanStillBeRemoved() {
        XCTAssertTrue(CloudRoutineCopy.canRemoveRunner(device("cloud", "approved")))
        XCTAssertFalse(CloudRoutineCopy.canRemoveRunner(device("cloud", "revoked")), "already removed")
        XCTAssertFalse(CloudRoutineCopy.canRemoveRunner(device("mac", "approved")), "a computer is revoked, not removed")
    }

    /// A routine already set to run in the cloud keeps its toggle, so it can be turned off after the runner is removed.
    func testARoutineAlreadyInTheCloudKeepsItsToggle() {
        XCTAssertTrue(CloudRoutineCopy.showsCloudSection(flag: false, devices: [], on: true))
    }

    func testTheSwitchIsReadFromItsDefaultsKeyAndIsOffUnlessSet() {
        let suite = "CloudRoutineModelTests.flag"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        XCTAssertFalse(CloudRoutineCopy.uiFlag(defaults))
        defaults.set(true, forKey: "com.mpointer.whimbrel.cloudRoutines.enabled")
        XCTAssertTrue(CloudRoutineCopy.uiFlag(defaults))
        defaults.removePersistentDomain(forName: suite)
    }

    func testTheNoRunnerWordsCountTheGraceFromTheReadyByTime() {
        XCTAssertTrue(CloudRoutineCopy.noRunner.contains("3 hours after its ready-by time or at 23:30, whichever is earlier"), CloudRoutineCopy.noRunner)
        XCTAssertTrue(CloudRoutineCopy.noRunner.contains("3 hours after its own time if it has no ready-by time"), CloudRoutineCopy.noRunner)
        XCTAssertTrue(CloudRoutineCopy.removeRunnerMessage.contains("3 hours after their ready-by time or at 23:30, whichever is earlier"))
        XCTAssertTrue(CloudRoutineCopy.removeRunnerMessage.contains("Meeting Prep 5 minutes before each meeting"))
        XCTAssertTrue(CloudRoutineCopy.noRunnerPrep.contains("5 minutes before it starts"))
        XCTAssertTrue(CloudRoutineCopy.readyByNote.hasPrefix("Whimbrel Cloud starts early enough to have the result ready by this time."))
    }
}
