import Foundation

// Turns synced data into scheduled reminders: stores the phone-local settings, remembers
// whether the account is on Cloud, and hands ReminderPlanner's output to the scheduler.
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

/// The last plan answer the Worker gave, kept so reminders still work offline.
final class PlanCache {
    private let defaults: UserDefaults
    private static let cloudKey = "whimbrel.reminders.isCloud"
    private static let askedKey = "whimbrel.reminders.permissionAsked"
    private static let checkedKey = "whimbrel.reminders.planCheckedAt"

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    var isCloud: Bool { defaults.bool(forKey: Self.cloudKey) }

    /// The notification-permission prompt is asked once ever, so a relaunch doesn't repeat it.
    var permissionAsked: Bool {
        get { defaults.bool(forKey: Self.askedKey) }
        set { defaults.set(newValue, forKey: Self.askedKey) }
    }

    /// Nil (a failed check) keeps the cached value; a plan replaces it.
    func update(_ plan: AccountPlan?) {
        guard let plan else { return }
        defaults.set(plan.isCloud, forKey: Self.cloudKey)
    }

    /// When the last successful check ran; the 30-minute throttle reads it.
    var lastCheckedAt: Date? {
        get { (defaults.object(forKey: Self.checkedKey) as? Double).map { Date(timeIntervalSince1970: $0) } }
        set { defaults.set(newValue?.timeIntervalSince1970, forKey: Self.checkedKey) }
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

    /// Cloud or not, as last known (the cache until a check answers).
    @Published private(set) var isCloud: Bool
    /// False in demo runs: nothing is checked, scheduled or asked, whatever the cached plan says.
    var isEnabled = true
    /// Set from the scene phase. The permission prompt is only ever shown in the foreground; if
    /// Cloud turned on in the background, it is asked when this becomes true.
    var isForeground = false {
        didSet {
            guard isForeground, !oldValue else { return }
            permissionTask = Task { await askPermissionIfNeeded() }
        }
    }
    /// The permission ask started by `isForeground`, exposed so tests can await it.
    private(set) var permissionTask: Task<Void, Never>?

    /// Plan checks after a sync wait at least this long since the last good one.
    static let planCheckInterval: TimeInterval = 30 * 60
    static let horizon: TimeInterval = 48 * 3600

    private let scheduler: ReminderScheduling
    /// The one store, shared with the Settings screen, which writes it and then calls `reschedule()`.
    let settings: ReminderSettingsStore
    private let plan: PlanCache
    private let now: () -> Date
    private let fetchPlan: () async -> AccountPlan?
    private let readReplica: (Int64, Int64) async -> ReplicaRead?
    private var checkingPlan = false
    /// Bumped by every `reschedule()`; a run that finds it changed after its awaits is stale.
    private var generation = 0

    /// `fetchPlan` and `readReplica` default to the enrollment model and the sync replica; tests inject fakes.
    init(sync: SyncModel, enrollment: EnrollmentModel, scheduler: ReminderScheduling, settings: ReminderSettingsStore,
         plan: PlanCache, fetchPlan: (() async -> AccountPlan?)? = nil, readReplica: ((Int64, Int64) async -> ReplicaRead?)? = nil,
         now: @escaping () -> Date = Date.init) {
        self.scheduler = scheduler
        self.settings = settings
        self.plan = plan
        self.now = now
        self.fetchPlan = fetchPlan ?? { [weak enrollment] in await enrollment?.fetchAccountPlan() }
        self.readReplica = readReplica ?? { [weak sync] fromMs, untilMs in
            await sync?.read { (events: $0.upcomingEvents(fromUnixMs: fromMs, untilUnixMs: untilMs, limit: 200),
                                snapshots: $0.topicSnapshots()) }
        }
        isCloud = plan.isCloud
    }

    /// Asks the Worker for the plan: always with `force` (launch), otherwise at most every 30
    /// minutes. A check already in flight absorbs overlapping calls. A failed check changes
    /// nothing and is not counted, so the next sync retries. Reschedules when the answer flips
    /// Cloud on or off.
    func refreshPlanIfDue(force: Bool) async {
        guard isEnabled, !checkingPlan else { return }
        let current = now()
        if !force, let last = plan.lastCheckedAt, current.timeIntervalSince(last) < Self.planCheckInterval, current >= last { return }
        checkingPlan = true
        let answer = await fetchPlan()
        checkingPlan = false
        guard let answer else { return }
        plan.update(answer)
        plan.lastCheckedAt = now()
        let changed = plan.isCloud != isCloud
        isCloud = plan.isCloud
        if changed { await reschedule() }
        await askPermissionIfNeeded()
    }

    /// Once ever, in the foreground, on Cloud.
    private func askPermissionIfNeeded() async {
        guard isEnabled, isForeground, isCloud, !plan.permissionAsked else { return }
        plan.permissionAsked = true
        _ = await scheduler.requestPermissionIfNeeded()
    }

    /// Plans from the replica's next 48 hours and replaces every pending reminder. Off Cloud it
    /// clears them. With the store not open it leaves the pending set as it is. Overlapping
    /// runs: only the newest may replace, and a run whose plan changed meanwhile drops out.
    func reschedule() async {
        guard isEnabled else { return }
        generation += 1
        let mine = generation
        let cloud = isCloud
        guard cloud else { return await scheduler.replaceAll([]) }
        let from = now()
        let fromMs = Int64(from.timeIntervalSince1970 * 1000)
        let untilMs = Int64(from.addingTimeInterval(Self.horizon).timeIntervalSince1970 * 1000)
        guard let read = await readReplica(fromMs, untilMs) else { return }
        let planned = ReminderPlanner.plan(events: read.events, snapshots: read.snapshots, settings: settings.settings,
                                           isCloud: true, now: from, calendar: .current)
        guard Self.mayReplace(mine: mine, current: generation, plannedAsCloud: cloud, isCloud: isCloud) else { return }
        await scheduler.replaceAll(planned)
    }

    /// The generation guard: a run may replace only if no newer run started and the plan did not change.
    nonisolated static func mayReplace(mine: Int, current: Int, plannedAsCloud: Bool, isCloud: Bool) -> Bool {
        mine == current && plannedAsCloud == isCloud
    }
}

/// Which rows the Settings -> Reminders section shows, and the digest time's minute/Date mapping.
struct ReminderSettingsRows: Equatable {
    var showsUpgradeLines: Bool
    var showsMeetingToggle: Bool
    var showsLeadPicker: Bool
    var showsDigestToggle: Bool
    var showsDigestTime: Bool
    var showsDeniedNotice: Bool
    var showsFooter: Bool

    /// Off Cloud only the two upgrade lines show. `denied` is notification permission refused.
    static func make(isCloud: Bool, settings: ReminderSettings, denied: Bool) -> ReminderSettingsRows {
        ReminderSettingsRows(showsUpgradeLines: !isCloud,
                             showsMeetingToggle: isCloud,
                             showsLeadPicker: isCloud && settings.meetingEnabled,
                             showsDigestToggle: isCloud,
                             showsDigestTime: isCloud && settings.digestEnabled,
                             showsDeniedNotice: isCloud && denied,
                             showsFooter: isCloud)
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
