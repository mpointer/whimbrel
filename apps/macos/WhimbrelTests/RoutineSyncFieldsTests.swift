import XCTest
@testable import Whimbrel

final class RoutineSyncFieldsTests: XCTestCase {
    private func routine(_ id: Int64, _ template: String, enabled: Bool, at hhmm: String, model: String = "default") -> CoreRoutine {
        CoreRoutine(id: id, name: template, template: template, scheduleHhmm: hhmm, enabled: enabled, autoApproveWriteActions: false, modelPref: model, createdAtUnixMs: 0)
    }

    func testKeysAreStable() {
        XCTAssertEqual(RoutineSyncFields.key(template: "daily_brief", field: .enabled), "routine.daily_brief.enabled")
        XCTAssertEqual(RoutineSyncFields.key(template: "reply_priorities", field: .schedule), "routine.reply_priorities.schedule")
        XCTAssertEqual(RoutineSyncFields.key(template: "daily_brief", field: .modelPref), "routine.daily_brief.model_pref")
    }

    func testModelPrefSyncsInTheThreeShapesCoreAccepts() {
        let routines = [routine(7, "daily_brief", enabled: true, at: "08:30", model: "cheapest")]
        XCTAssertEqual(RoutineSyncFields.values(from: routines)["routine.daily_brief.model_pref"], "cheapest")
        for good in ["default", "cheapest", "anthropic/claude-sonnet-5", "ollama/gemma4:12b", "openrouter/vendor/model:free"] {
            XCTAssertEqual(RoutineSyncFields.change(key: "routine.daily_brief.model_pref", value: good, routines: routines), .modelPref(id: 7, good), good)
        }
        for bad in ["", "cheap", "anthropic", "anthropic/", "venice/llama"] {
            XCTAssertNil(RoutineSyncFields.change(key: "routine.daily_brief.model_pref", value: bad, routines: routines), bad)
        }
    }

    /// The four routines that may run in the cloud (the runner needs their on/off, time and model) and Reply
    /// Priorities (it fails over). Focus Time writes to the calendar: its settings stay on this device.
    func testTheCloudTemplatesAndReplyPrioritiesAreSyncedButNotFocusTime() {
        let routines = [
            routine(1, "daily_brief", enabled: true, at: "08:30"),
            routine(2, "focus_time", enabled: true, at: "09:00"),
            routine(3, "eod_recap", enabled: true, at: "17:00"),
            routine(4, "reply_priorities", enabled: false, at: "10:00"),
            routine(5, "weekly_digest", enabled: false, at: "16:00"),
            routine(6, "meeting_prep", enabled: true, at: "08:00"),
        ]
        let values = RoutineSyncFields.values(from: routines)
        XCTAssertEqual(values, [
            "routine.daily_brief.enabled": "true", "routine.daily_brief.schedule": "08:30", "routine.daily_brief.model_pref": "default",
            "routine.reply_priorities.enabled": "false", "routine.reply_priorities.schedule": "10:00", "routine.reply_priorities.model_pref": "default",
            "routine.eod_recap.enabled": "true", "routine.eod_recap.schedule": "17:00", "routine.eod_recap.model_pref": "default",
            "routine.weekly_digest.enabled": "false", "routine.weekly_digest.schedule": "16:00", "routine.weekly_digest.model_pref": "default",
            "routine.meeting_prep.enabled": "true", "routine.meeting_prep.schedule": "08:00", "routine.meeting_prep.model_pref": "default",
        ])
        XCTAssertEqual(RoutineSyncFields.change(key: "routine.eod_recap.enabled", value: "false", routines: routines), .enabled(id: 3, false))
    }

    func testCloudCloudGrantAndReadyByRoundTripForEodRecapAndMeetingPrep() {
        let cloud = ["eod_recap": true, "meeting_prep": false]
        let readyBy = ["eod_recap": "17:45"]
        let grants = ["eod_recap": ["meeting", "items:transcript", "routine_run"], "meeting_prep": ["meeting", "meeting_prep"]]
        let values = RoutineSyncFields.cloudValues(cloud: cloud, readyBy: readyBy, grants: grants)
        XCTAssertEqual(values["routine.eod_recap.cloud"], "true")
        XCTAssertEqual(values["routine.meeting_prep.cloud"], "false")
        XCTAssertEqual(values["routine.eod_recap.readyBy"], "17:45")
        XCTAssertNil(values["routine.meeting_prep.readyBy"], "no ready-by chosen: nothing pushed")
        // The grant is pushed only while the routine is in the cloud, as core's list.
        XCTAssertEqual(values["routine.meeting_prep.cloudGrant"], "[]")
        for (key, value) in values {
            XCTAssertNotNil(RoutineSyncFields.cloudChange(key: key, value: value), key)
        }
        XCTAssertEqual(RoutineSyncFields.cloudChange(key: "routine.eod_recap.cloud", value: values["routine.eod_recap.cloud"]!), .cloud(template: "eod_recap", true))
        XCTAssertEqual(RoutineSyncFields.cloudChange(key: "routine.meeting_prep.cloud", value: "false"), .cloud(template: "meeting_prep", false))
        XCTAssertEqual(RoutineSyncFields.cloudChange(key: "routine.eod_recap.readyBy", value: "17:45"), .readyBy(template: "eod_recap", "17:45"))
        XCTAssertEqual(RoutineSyncFields.cloudChange(key: "routine.eod_recap.cloudGrant", value: values["routine.eod_recap.cloudGrant"]!),
                       .cloudGrant(template: "eod_recap", ["meeting", "items:transcript", "routine_run"]))
        XCTAssertEqual(RoutineSyncFields.cloudKey(template: "meeting_prep", field: .cloudGrant), "routine.meeting_prep.cloudGrant")
        // Malformed values are not applied, so they are pulled again.
        XCTAssertNil(RoutineSyncFields.cloudChange(key: "routine.eod_recap.cloud", value: "yes"))
        XCTAssertNil(RoutineSyncFields.cloudChange(key: "routine.eod_recap.readyBy", value: "5pm"))
        XCTAssertNil(RoutineSyncFields.cloudChange(key: "routine.eod_recap.cloudGrant", value: "meeting"))
        // Cloud fields are not routine-row fields: they never go to the store path.
        XCTAssertFalse(RoutineSyncFields.isRoutineKey("routine.eod_recap.cloud"))
        XCTAssertTrue(RoutineSyncFields.isCloudKey("routine.eod_recap.readyBy"))
    }

    func testAnUnknownTemplateKeyIsIgnored() {
        let routines = [routine(1, "daily_brief", enabled: true, at: "08:30"), routine(2, "focus_time", enabled: true, at: "09:00")]
        for key in ["routine.reply_priorities.cloud", "routine.focus_time.cloud", "routine.away_follow_up.readyBy", "routine.someday.cloud"] {
            XCTAssertFalse(RoutineSyncFields.isCloudKey(key), key)
            XCTAssertNil(RoutineSyncFields.cloudChange(key: key, value: "true"), key)
        }
        for key in ["routine.focus_time.enabled", "routine.someday.enabled", "routine.daily_brief.colour", "routine.daily_brief"] {
            XCTAssertFalse(RoutineSyncFields.isRoutineKey(key), key)
            XCTAssertNil(RoutineSyncFields.change(key: key, value: "true", routines: routines), key)
        }
        XCTAssertNil(RoutineSyncFields.cloudValues(cloud: ["reply_priorities": true, "focus_time": true], readyBy: [:], grants: [:])["routine.reply_priorities.cloud"])
        XCTAssertTrue(RoutineSyncFields.cloudValues(cloud: ["focus_time": true], readyBy: ["focus_time": "09:00"], grants: [:]).keys.allSatisfy { !$0.contains("focus_time") })
    }

    func testAPulledValueMapsToTheLocalRoutineOfThatTemplate() {
        let routines = [routine(7, "daily_brief", enabled: true, at: "08:30")]
        XCTAssertEqual(RoutineSyncFields.change(key: "routine.daily_brief.enabled", value: "false", routines: routines), .enabled(id: 7, false))
        XCTAssertEqual(RoutineSyncFields.change(key: "routine.daily_brief.schedule", value: "12:00", routines: routines), .schedule(id: 7, "12:00"))
    }

    func testBadOrUnknownInputIsRejectedSoItRetriesNextSync() {
        let routines = [routine(7, "daily_brief", enabled: true, at: "08:30")]
        XCTAssertNil(RoutineSyncFields.change(key: "routine.daily_brief.schedule", value: "25:99", routines: routines))
        XCTAssertNil(RoutineSyncFields.change(key: "routine.daily_brief.schedule", value: "8:30", routines: routines))
        XCTAssertNil(RoutineSyncFields.change(key: "routine.daily_brief.enabled", value: "yes", routines: routines))
        XCTAssertNil(RoutineSyncFields.change(key: "routine.focus_time.enabled", value: "true", routines: [routine(1, "focus_time", enabled: false, at: "17:00")]))
        XCTAssertNil(RoutineSyncFields.change(key: "routine.reply_priorities.enabled", value: "true", routines: routines), "not seeded yet")
        XCTAssertNil(RoutineSyncFields.change(key: "other.key", value: "true", routines: routines))
    }

    /// Task 0's carry: the runner writes `cloud.runner.lastRun`, and later versions may write other `cloud.*`
    /// keys. This Mac keeps the last-run time for Settings, ignores what it does not know, and never pushes them.
    @MainActor
    func testUnknownCloudPulledKeysAreToleratedAndTheRunnersLastRunIsKept() {
        let suite = "RoutineSyncFieldsTests.cloud"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        let cloud = CloudRoutineSettings(defaults: defaults)
        let sync = SettingsSyncCoordinator(
            routing: RoutingSettings(), retention: RetentionSettings(), promptLibrary: PromptLibrarySettings(),
            profile: ProfileSettings(defaults: defaults), routineHost: RoutineHostSettings(defaults: defaults),
            syncState: SyncState(signIn: nil, worker: nil), cloudRoutines: cloud
        )
        func field(_ v: String, deleted: Bool = false) -> CoreMemoryStore.SyncSettingsField {
            CoreMemoryStore.SyncSettingsField(value: v, updatedAtUnixMs: 5, isDeleted: deleted)
        }
        XCTAssertTrue(sync.apply(fieldKey: "cloud.runner.lastRun", field: field("1763190000000")))
        XCTAssertEqual(cloud.runnerLastRunMs, 1_763_190_000_000)
        XCTAssertEqual(CloudRoutineSettings(defaults: defaults).runnerLastRunMs, 1_763_190_000_000, "kept across launches")
        XCTAssertFalse(sync.apply(fieldKey: "cloud.runner.lastRun", field: field("soon")))
        XCTAssertEqual(cloud.runnerLastRunMs, 1_763_190_000_000, "an unreadable value keeps the last good time")
        XCTAssertFalse(sync.apply(fieldKey: "cloud.run_request.abc", field: field("{}")))
        XCTAssertFalse(sync.apply(fieldKey: "cloud.somethingNew", field: field("x")))
        XCTAssertFalse(sync.apply(fieldKey: "routine.eod_recap.cloud", field: field("true", deleted: true)))
        XCTAssertTrue(sync.apply(fieldKey: "routine.eod_recap.cloud", field: field("true")))
        XCTAssertTrue(cloud.isCloud("eod_recap"))
        XCTAssertTrue(sync.apply(fieldKey: "routine.eod_recap.readyBy", field: field("17:30")))
        XCTAssertEqual(cloud.readyBy["eod_recap"], "17:30")
        XCTAssertTrue(sync.apply(fieldKey: "profile.timeZone", field: field("Europe/Paris")), "another device's zone is accepted, not an error")

        let pushed = sync.gatherLocalSettings().fields
        XCTAssertNil(pushed["cloud.runner.lastRun"], "the heartbeat is the runner's; this Mac never pushes it")
        XCTAssertEqual(pushed["profile.timeZone"]?.value, TimeZone.current.identifier)
        XCTAssertEqual(pushed["routine.eod_recap.cloud"]?.value, "true")
        XCTAssertEqual(pushed["routine.eod_recap.readyBy"]?.value, "17:30")
        XCTAssertTrue(pushed["routine.eod_recap.cloudGrant"]?.value.contains("\"meeting\"") == true, "core's grant list")
        XCTAssertNil(pushed["routine.reply_priorities.cloud"])
    }

    func testRoutineKeysAreRecognised() {
        XCTAssertTrue(RoutineSyncFields.isRoutineKey("routine.daily_brief.enabled"))
        XCTAssertFalse(RoutineSyncFields.isRoutineKey("routines.primaryDeviceId"))
    }
}
