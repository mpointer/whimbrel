import Foundation

// "Run in the cloud" in Settings → Routines, and "Whimbrel Cloud" in Settings → Devices
// (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md). The words and rules live here, apart from the
// views, so tests run without a store.

/// What core says about a template and the cloud (`whimbrel_routines_cloud_info`).
struct CloudRoutineInfo: Decodable, Equatable {
    let eligible: Bool
    /// The row types the runner may read for it (`routines::grant_for`).
    let grant: [String]
    /// Those row types in plain words, as a list ("meetings, transcripts and client settings").
    let grantList: String
    /// Core's own sentence about the grant.
    let grantSentence: String
    /// The Google Calendar line for a routine that reads calendar events; empty otherwise.
    let googleCalendarLimit: String

    enum CodingKeys: String, CodingKey {
        case eligible, grant
        case grantList = "grant_list"
        case grantSentence = "grant_sentence"
        case googleCalendarLimit = "google_calendar_limit"
    }

    /// Core's answer for `template`; nil only when the call fails.
    static func load(_ template: String) -> CloudRoutineInfo? {
        CoreMemoryStore.routinesCloudInfo(template: template)
    }
}

/// One routine run, as the freshness label needs it.
struct CloudRunStamp: Equatable {
    let ranAtUnixMs: Int64
    let status: String
}

/// How fresh a cloud routine's result is.
enum CloudFreshness: Equatable {
    /// No result yet, and not late.
    case none
    /// The newest good result.
    case updated(Date)
    /// Today's run is missing past the ready-by time: the last good result (if any) shows instead.
    case late(lastGood: Date?)
}

enum CloudRoutineCopy {
    /// Only the templates core lets run in the cloud get the toggle (never Reply Priorities, Focus Time or
    /// Away follow-up).
    static func showsToggle(_ info: CloudRoutineInfo?) -> Bool { info?.eligible == true }

    // MARK: Who sees any of it (author-only phase, docs/RELEASING.md "Cloud routine runner gate")

    /// The hidden switch that shows the cloud UI before a runner is on the account. Off unless set by hand:
    /// `defaults write com.mpointer.kestrel com.mpointer.whimbrel.cloudRoutines.enabled -bool true`.
    static let uiFlagKey = "com.mpointer.whimbrel.cloudRoutines.enabled"

    static func uiFlag(_ defaults: UserDefaults = .standard) -> Bool { defaults.bool(forKey: uiFlagKey) }

    /// An approved Whimbrel Cloud is on the account.
    static func runnerOnAccount(devices: [SyncDeviceRecord]) -> Bool { !runnerMissing(devices: devices) }

    /// Whether a routine shows "Run in the cloud" (with its ready-by time and Updated/Late line): with the hidden
    /// switch on, or once a Whimbrel Cloud is on the account. A routine already set to run in the cloud keeps it
    /// too, so it can always be turned off. With the switch off, no runner and nothing set, nothing new shows.
    static func showsCloudSection(flag: Bool, devices: [SyncDeviceRecord], on: Bool) -> Bool {
        flag || on || runnerOnAccount(devices: devices)
    }

    /// "Add Whimbrel Cloud…" (and its sheet) only with the hidden switch on.
    static func showsAddRunner(flag: Bool) -> Bool { flag }

    /// A Whimbrel Cloud in the device list can always be removed while it is not already, switch or no switch.
    static func canRemoveRunner(_ device: SyncDeviceRecord) -> Bool { isRunner(device) && device.status != "revoked" }

    static let toggleLabel = String(localized: "Run in the cloud")

    /// The binding sentence (plan, Global Constraints), with core's list of what the grant reads, then core's
    /// Google Calendar line where it applies. The host's secret store holds the key at all times, not only during
    /// a run, so the sentence says that first.
    static func toggleCopy(_ info: CloudRoutineInfo) -> String {
        let sentence = String(localized: "Whimbrel Cloud holds your sync key. While this runs it can read your \(info.grantList). It keeps nothing after the run.")
        return info.googleCalendarLimit.isEmpty ? sentence : "\(sentence) \(info.googleCalendarLimit)"
    }

    /// What the cloud cannot undo or see (ledger Task 3 I2 and Task 6 ruling 1).
    static let disclosure = String(localized: "Results already made are not recalled if you later delete a client. On the cloud, exclusion matches by name, alias and contact address; items you filed by hand are not known to the cloud.")

    static let readyByLabel = String(localized: "Ready by")
    /// The ready-by time syncs for the runner, which starts early enough to be ready by it; on this Mac it decides
    /// when a run is late and, with no result, when this Mac stands in.
    static let readyByNote = String(localized: "Whimbrel Cloud starts early enough to have the result ready by this time. If no result has arrived by then, Settings shows the run as late, with the last result.")

    /// Whether a routine can be late: only the daily ones that are on. The Weekly Digest runs one day a week and
    /// Meeting Prep runs before each meeting, so they only show "Updated".
    static func canBeLate(template: String, enabled: Bool) -> Bool {
        enabled && (template == "daily_brief" || template == "eod_recap")
    }

    /// The ready-by time when none is chosen: an hour after the routine's own time, since the runner starts at that
    /// time and the result then has to sync. Nil when the time does not parse.
    static func defaultReadyBy(schedule: String) -> String? {
        guard RoutineSyncFields.isHhmm(schedule) else { return nil }
        let parts = schedule.split(separator: ":").compactMap { Int($0) }
        let minutes = min(parts[0] * 60 + parts[1] + 60, 23 * 60 + 59)
        return String(format: "%02d:%02d", minutes / 60, minutes % 60)
    }

    /// Why "Run in the cloud" cannot be turned on, or nil when it can. The runner reaches this Mac's settings only
    /// through sync, and it has no model of its own, only a synced provider key.
    /// Order (matches Windows): sync off, routine off, local-only, no synced key. A routine that is off cannot go to
    /// the cloud: turning it on would push this Mac's "off" over another device's "on".
    static func blockedReason(syncReady: Bool, routineEnabled: Bool, localOnly: Bool, hasSyncedKey: Bool) -> String? {
        if !syncReady { return String(localized: "Turn on sync in Settings → Devices first: Whimbrel Cloud gets this routine's settings through sync.") }
        if !routineEnabled { return String(localized: "Turn this routine on first.") }
        if localOnly { return String(localized: "Local-only mode is on, so Whimbrel Cloud can't use a hosted model for this routine.") }
        if !hasSyncedKey {
            return String(localized: "Whimbrel Cloud uses a provider key you sync. First turn on Sync to devices for a hosted provider's key in Settings → AI Providers.")
        }
        return nil
    }

    /// No approved Whimbrel Cloud is on the account. An empty list (sync off, or not loaded yet) counts as none:
    /// the note then says what happens rather than staying silent.
    static func runnerMissing(devices: [SyncDeviceRecord]) -> Bool {
        !devices.contains { isRunner($0) && $0.status == "approved" }
    }

    /// The one line under the toggle that says why it is off-limits or why the routine will not run in the cloud.
    /// A blocked reason (sync off, local-only, no synced key) wins whether the toggle is on or off: while on, the
    /// routine then runs on this computer after the grace, and the reason says why. Only with nothing blocking does an
    /// on toggle say that no Whimbrel Cloud is on the account.
    static func statusNote(on: Bool, blockedReason: String?, runnerMissing: Bool, template: String = "") -> String? {
        if let blockedReason { return blockedReason }
        guard on && runnerMissing else { return nil }
        return template == "meeting_prep" ? noRunnerPrep : noRunner
    }

    /// The computer stands in 3 hours after the ready-by time, but by 23:30 that day; with no ready-by time, 3 hours
    /// after the routine's own time (core `tick_cloud`, `DEVICE_STAND_IN_CAP_HHMM`).
    static let noRunner = String(localized: "Whimbrel Cloud is not on your account yet, so this routine runs on your computer 3 hours after its ready-by time or at 23:30, whichever is earlier (3 hours after its own time if it has no ready-by time). Add Whimbrel Cloud in Settings → Devices.")
    /// Meeting Prep has no ready-by time: the computer prepares a meeting itself 5 minutes before it starts when no
    /// brief has synced in (core `PREP_CLOUD_DEVICE_BEFORE_START_MS`).
    static let noRunnerPrep = String(localized: "Whimbrel Cloud is not on your account yet, so your computer prepares each meeting 5 minutes before it starts. Add Whimbrel Cloud in Settings → Devices.")

    /// The freshness of a routine whose runs are `runs` (newest or not, any order), at `now`. Failed rows (a run
    /// that did not finish, or one the runner left for this computer) are never a result.
    static func freshness(runs: [CloudRunStamp], readyBy: String, now: Date, calendar: Calendar) -> CloudFreshness {
        let lastGood = runs.filter { $0.status != "failed" }.map(\.ranAtUnixMs).max()
            .map { Date(timeIntervalSince1970: Double($0) / 1000) }
        if let lastGood, calendar.isDate(lastGood, inSameDayAs: now) { return .updated(lastGood) }
        if let due = readyByTime(readyBy, on: now, calendar: calendar), now >= due { return .late(lastGood: lastGood) }
        return lastGood.map(CloudFreshness.updated) ?? .none
    }

    static func label(_ freshness: CloudFreshness, now: Date, calendar: Calendar, locale: Locale = .current) -> String? {
        switch freshness {
        case .none:
            return nil
        case .updated(let at):
            return String(localized: "Updated \(when(at, now: now, calendar: calendar, locale: locale))")
        case .late(let lastGood?):
            return String(localized: "Late: showing the last result from \(when(lastGood, now: now, calendar: calendar, locale: locale))")
        case .late(nil):
            return String(localized: "Late: no result yet")
        }
    }

    /// The time alone for today ("6:58 AM"), else the day and time.
    static func when(_ date: Date, now: Date, calendar: Calendar, locale: Locale) -> String {
        var style = Date.FormatStyle(locale: locale, calendar: calendar, timeZone: calendar.timeZone)
        style = calendar.isDate(date, inSameDayAs: now) ? style.hour().minute() : style.month(.abbreviated).day().hour().minute()
        return date.formatted(style)
    }

    /// The ready-by "HH:MM" on `day`'s date, or nil when it does not parse.
    static func readyByTime(_ hhmm: String, on day: Date, calendar: Calendar) -> Date? {
        guard RoutineSyncFields.isHhmm(hhmm) else { return nil }
        let parts = hhmm.split(separator: ":").compactMap { Int($0) }
        return calendar.date(bySettingHour: parts[0], minute: parts[1], second: 0, of: day)
    }

    // MARK: Settings → Devices

    /// The runner's name on the account (`platform` "cloud").
    static let runnerName = String(localized: "Whimbrel Cloud")
    static let addRunner = String(localized: "Add Whimbrel Cloud…")
    static let removeRunnerTitle = String(localized: "Remove Whimbrel Cloud?")
    /// Honest about what removal does: relay access ends; the copy of the sync key it held is not recalled.
    static let removeRunnerMessage = String(localized: "Whimbrel Cloud loses its access to your sync at once. Routines still set to run in the cloud then run on your computer 3 hours after their ready-by time or at 23:30, whichever is earlier (3 hours after their own time if they have no ready-by time, and Meeting Prep 5 minutes before each meeting), until you turn Run in the cloud off. To use Whimbrel Cloud again, run its setup again and add it again. Like a lost laptop, the copy of your sync key it held is not recalled.")

    static func isRunner(_ device: SyncDeviceRecord) -> Bool { device.platform == "cloud" }

    /// What the host's secret store needs from this sheet, by name (`whimbrel-cloud-runner` reads these).
    static let provisionInstructions = String(localized: "Add these to Whimbrel Cloud's secret store, next to RUNNER_DEVICE_SECRET_KEY from its setup. The provision value carries your sync key: paste it only there. It is shown once.")
}
