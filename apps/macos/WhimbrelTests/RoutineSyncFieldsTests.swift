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

    func testOnlyTheFailoverTemplatesAreSynced() {
        let routines = [
            routine(1, "daily_brief", enabled: true, at: "08:30"),
            routine(2, "focus_time", enabled: true, at: "09:00"),
            routine(3, "eod_recap", enabled: true, at: "17:00"),
            routine(4, "reply_priorities", enabled: false, at: "10:00"),
        ]
        let values = RoutineSyncFields.values(from: routines)
        XCTAssertEqual(values, [
            "routine.daily_brief.enabled": "true", "routine.daily_brief.schedule": "08:30", "routine.daily_brief.model_pref": "default",
            "routine.reply_priorities.enabled": "false", "routine.reply_priorities.schedule": "10:00", "routine.reply_priorities.model_pref": "default",
        ])
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
        XCTAssertNil(RoutineSyncFields.change(key: "routine.eod_recap.enabled", value: "true", routines: [routine(1, "eod_recap", enabled: false, at: "17:00")]))
        XCTAssertNil(RoutineSyncFields.change(key: "routine.reply_priorities.enabled", value: "true", routines: routines), "not seeded yet")
        XCTAssertNil(RoutineSyncFields.change(key: "other.key", value: "true", routines: routines))
    }

    func testRoutineKeysAreRecognised() {
        XCTAssertTrue(RoutineSyncFields.isRoutineKey("routine.daily_brief.enabled"))
        XCTAssertFalse(RoutineSyncFields.isRoutineKey("routines.primaryDeviceId"))
    }
}
