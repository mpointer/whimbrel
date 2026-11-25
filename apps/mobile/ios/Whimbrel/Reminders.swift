import Foundation

// Turns synced data into scheduled reminders: stores the phone-local settings and hands
// ReminderPlanner's output to the scheduler. Reminders run on the phone, so every plan has them.
// The system scheduler (NotificationScheduler) is handed in by WhimbrelApp.

/// The reminder settings, phone-local (never part of PhoneSettingsSync).
final class ReminderSettingsStore {
    private let defaults: UserDefaults
    private enum Key {
        static let meeting = "whimbrel.reminders.meetingEnabled"
        static let lead = "whimbrel.reminders.leadMinutes"
        static let digest = "whimbrel.reminders.digestEnabled"
        static let digestMinute = "whimbrel.reminders.digestMinuteOfDay"
    }

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    var settings: ReminderSettings {
        get {
            let fallback = ReminderSettings()
            return ReminderSettings(
                meetingEnabled: defaults.object(forKey: Key.meeting) as? Bool ?? fallback.meetingEnabled,
                leadMinutes: defaults.object(forKey: Key.lead) as? Int ?? fallback.leadMinutes,
                digestEnabled: defaults.object(forKey: Key.digest) as? Bool ?? fallback.digestEnabled,
                digestMinuteOfDay: defaults.object(forKey: Key.digestMinute) as? Int ?? fallback.digestMinuteOfDay)
        }
        set {
            defaults.set(newValue.meetingEnabled, forKey: Key.meeting)
            defaults.set(newValue.leadMinutes, forKey: Key.lead)
            defaults.set(newValue.digestEnabled, forKey: Key.digest)
            defaults.set(newValue.digestMinuteOfDay, forKey: Key.digestMinute)
        }
    }
}

/// What the reminders remember about the phone across launches: today only whether the
/// notification permission was asked. The key is unchanged from when this also cached the plan.
final class ReminderStateStore {
    private let defaults: UserDefaults
    private static let askedKey = "whimbrel.reminders.permissionAsked"

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    /// The notification-permission prompt is asked once ever, so a relaunch doesn't repeat it.
    var permissionAsked: Bool {
        get { defaults.bool(forKey: Self.askedKey) }
        set { defaults.set(newValue, forKey: Self.askedKey) }
    }
}

/// The system's pending-notification store, faked in tests.
protocol ReminderScheduling: AnyObject {
    /// Makes exactly `reminders` the pending set (an empty list clears it).
    func replaceAll(_ reminders: [PlannedReminder]) async
    /// Asks for notification permission if it was never asked; true when allowed.
    func requestPermissionIfNeeded() async -> Bool
}

@MainActor
final class ReminderCoordinator: ObservableObject {
    /// What one read of the replica returns.
    typealias ReplicaRead = (events: [PhoneCalendarEvent], snapshots: [PhoneTopicSnapshot])

    /// False in demo runs: nothing is scheduled or asked.
    var isEnabled = true
    /// Set from the scene phase. The permission prompt is only ever shown in the foreground; if
    /// the phone was approved in the background, it is asked when this becomes true.
    var isForeground = false {
        didSet {
            guard isForeground, !oldValue else { return }
            permissionTask = Task { await askPermissionIfNeeded() }
        }
    }
    /// The permission ask started by `isForeground`, exposed so tests can await it.
    private(set) var permissionTask: Task<Void, Never>?

    static let horizon: TimeInterval = 48 * 3600

    private let scheduler: ReminderScheduling
    /// The one store, shared with the Settings screen, which writes it and then calls `reschedule()`.
    let settings: ReminderSettingsStore
    private let state: ReminderStateStore
    private let now: () -> Date
    /// Whether this phone is signed in and approved, so it has synced data to remind from.
    private let isApproved: () -> Bool
    private let readReplica: (Int64, Int64) async -> ReplicaRead?
    /// Bumped by every `reschedule()`; a run that finds it changed after its awaits is stale.
    private var generation = 0

    /// `isApproved` and `readReplica` default to the enrollment model and the sync replica; tests inject fakes.
    init(sync: SyncModel, enrollment: EnrollmentModel, scheduler: ReminderScheduling, settings: ReminderSettingsStore,
         state: ReminderStateStore, isApproved: (() -> Bool)? = nil, readReplica: ((Int64, Int64) async -> ReplicaRead?)? = nil,
         now: @escaping () -> Date = Date.init) {
        self.scheduler = scheduler
        self.settings = settings
        self.state = state
        self.now = now
        self.isApproved = isApproved ?? { [weak enrollment] in enrollment?.status == .approved }
        self.readReplica = readReplica ?? { [weak sync] fromMs, untilMs in
            await sync?.read { (events: $0.upcomingEvents(fromUnixMs: fromMs, untilUnixMs: untilMs, limit: 200),
                                snapshots: $0.topicSnapshots()) }
        }
    }

    /// Once ever, in the foreground, on an approved phone. Called on each return to the
    /// foreground and after each sync, so a phone approved while the app is open is asked too.
    func askPermissionIfNeeded() async {
        guard isEnabled, isForeground, isApproved(), !state.permissionAsked else { return }
        state.permissionAsked = true
        _ = await scheduler.requestPermissionIfNeeded()
    }

    /// Plans from the replica's next 48 hours and replaces every pending reminder. On a phone that
    /// is not approved it plans nothing (a wipe clears the pending set). With the store not open
    /// it leaves the pending set as it is. Overlapping runs: only the newest may replace.
    func reschedule() async {
        guard isEnabled, isApproved() else { return }
        generation += 1
        let mine = generation
        let from = now()
        let fromMs = Int64(from.timeIntervalSince1970 * 1000)
        let untilMs = Int64(from.addingTimeInterval(Self.horizon).timeIntervalSince1970 * 1000)
        guard let read = await readReplica(fromMs, untilMs) else { return }
        let planned = ReminderPlanner.plan(events: read.events, snapshots: read.snapshots, settings: settings.settings,
                                           now: from, calendar: .current)
        guard Self.mayReplace(mine: mine, current: generation, isApproved: isApproved()) else { return }
        await scheduler.replaceAll(planned)
    }

    /// The generation guard: a run may replace only if no newer run started and the phone is still approved.
    nonisolated static func mayReplace(mine: Int, current: Int, isApproved: Bool) -> Bool {
        mine == current && isApproved
    }
}

/// Which rows the Settings -> Reminders section shows, and the digest time's minute/Date mapping.
struct ReminderSettingsRows: Equatable {
    var showsMeetingToggle: Bool
    var showsLeadPicker: Bool
    var showsDigestToggle: Bool
    var showsDigestTime: Bool
    var showsDeniedNotice: Bool
    var showsFooter: Bool

    /// `denied` is notification permission refused.
    static func make(settings: ReminderSettings, denied: Bool) -> ReminderSettingsRows {
        ReminderSettingsRows(showsMeetingToggle: true,
                             showsLeadPicker: settings.meetingEnabled,
                             showsDigestToggle: true,
                             showsDigestTime: settings.digestEnabled,
                             showsDeniedNotice: denied,
                             showsFooter: true)
    }

    /// The Date on `day` at `minute` minutes after local midnight.
    static func date(forMinuteOfDay minute: Int, on day: Date = Date(), calendar: Calendar = .current) -> Date {
        calendar.date(bySettingHour: minute / 60, minute: minute % 60, second: 0, of: day) ?? day
    }

    static func minuteOfDay(of date: Date, calendar: Calendar = .current) -> Int {
        let parts = calendar.dateComponents([.hour, .minute], from: date)
        return (parts.hour ?? 0) * 60 + (parts.minute ?? 0)
    }
}
