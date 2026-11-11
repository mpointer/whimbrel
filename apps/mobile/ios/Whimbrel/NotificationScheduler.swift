import BackgroundTasks
import Foundation
import UserNotifications

// The system side of reminders: pending local notifications, the permission prompt, taps, and
// the background refresh that keeps the pending set current. The decisions are in
// ReminderPlanner and ReminderCoordinator; this file only adapts them to Apple's APIs.

extension ReminderRoute {
    /// What a notification carries so a tap can find its way back.
    var userInfo: [String: Any] {
        switch self {
        case .today: return ["route": "today"]
        case .topics: return ["route": "topics"]
        case let .topic(key): return ["route": "topic", "key": key]
        }
    }

    /// Nil for anything that isn't a route this app wrote.
    init?(userInfo: [AnyHashable: Any]) {
        switch userInfo["route"] as? String {
        case "today": self = .today
        case "topics": self = .topics
        case "topic":
            guard let key = userInfo["key"] as? String, !key.isEmpty else { return nil }
            self = .topic(key: key)
        default: return nil
        }
    }
}

/// Where a tapped reminder wants to go. RootView consumes it and sets it back to nil.
@MainActor
final class ReminderRouter: ObservableObject {
    @Published var pending: ReminderRoute?
}

final class NotificationScheduler: NSObject, ReminderScheduling, UNUserNotificationCenterDelegate {
    private let center: UNUserNotificationCenter
    private let router: ReminderRouter
    private let lock = NSLock()
    private var tail: Task<Void, Never>?

    init(router: ReminderRouter, center: UNUserNotificationCenter = .current()) {
        self.router = router
        self.center = center
        super.init()
        center.delegate = self
    }

    func replaceAll(_ reminders: [PlannedReminder]) async {
        // Each call waits for the one before it. Remove-all then add-all awaits in between, so
        // two calls running side by side could interleave and leave a mix of both sets pending.
        let run: Task<Void, Never> = lock.withLock {
            let previous = tail
            let task = Task { [self] in
                await previous?.value
                await apply(reminders)
            }
            tail = task
            return task
        }
        await run.value
    }

    private func apply(_ reminders: [PlannedReminder]) async {
        center.removeAllPendingNotificationRequests()
        for reminder in reminders {
            let content = UNMutableNotificationContent()
            content.title = reminder.title
            content.body = reminder.body ?? ""
            content.sound = .default
            content.userInfo = reminder.route.userInfo
            let parts = Calendar.current.dateComponents([.year, .month, .day, .hour, .minute], from: reminder.fireAt)
            let trigger = UNCalendarNotificationTrigger(dateMatching: parts, repeats: false)
            try? await center.add(UNNotificationRequest(identifier: reminder.id, content: content, trigger: trigger))
        }
    }

    func requestPermissionIfNeeded() async -> Bool {
        let current = await center.notificationSettings()
        switch current.authorizationStatus {
        case .notDetermined:
            return (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        case .authorized, .provisional, .ephemeral:
            return true
        default:
            return false
        }
    }

    // Reminders that arrive while the app is open still show.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let route = ReminderRoute(userInfo: response.notification.request.content.userInfo) else { return }
        await MainActor.run { router.pending = route }
    }
}

/// Succeeds once, for the first caller.
final class OnceFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var claimed = false
    func claim() -> Bool { lock.withLock { defer { claimed = true }; return !claimed } }
}

/// The background refresh: a short run now and then so reminders follow new meetings.
enum ReminderBackgroundRefresh {
    static let identifier = "com.mpointer.whimbrel.mobile.refresh"
    static let interval: TimeInterval = 30 * 60

    /// Call from `WhimbrelApp.init`, before launch finishes.
    @MainActor
    static func register(sync: SyncModel, coordinator: ReminderCoordinator) {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { task in
            schedule() // the next run, whatever happens to this one
            // Whichever path finishes first completes the task; the other does nothing.
            let completion = OnceFlag()
            let work = Task { @MainActor in
                await sync.syncNow()
                if !Task.isCancelled { await coordinator.refreshPlanIfDue(force: false) }
                if !Task.isCancelled { await coordinator.reschedule() }
                if completion.claim() { task.setTaskCompleted(success: !Task.isCancelled) }
            }
            task.expirationHandler = {
                work.cancel()
                if completion.claim() { task.setTaskCompleted(success: false) }
            }
        }
    }

    static func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        try? BGTaskScheduler.shared.submit(request)
    }
}
