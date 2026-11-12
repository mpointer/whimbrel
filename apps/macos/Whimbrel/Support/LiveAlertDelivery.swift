import AppKit
import Foundation
import os
import UserNotifications

/// `whimbrel://meeting/<id>?at=<centiseconds>[&origin=<deviceId>&src=<sourceMeetingId>]`: opens
/// a meeting at a moment. Used by alert notifications and the alert email. `origin` and `src`
/// name the device that made the link and its own id for the meeting, so the email's link
/// also opens the right meeting on the user's other devices, through `sync_meeting_map`.
enum DeepLink {
    struct Target: Equatable {
        let meetingId: Int64
        let atCs: Int64
        var origin: String?
        var sourceMeetingId: Int64?
    }

    static func meetingURL(id: Int64, atCs: Int64, origin: String? = nil) -> URL {
        var parts = URLComponents()
        parts.scheme = "whimbrel"
        parts.host = "meeting"
        parts.path = "/\(id)"
        parts.queryItems = [URLQueryItem(name: "at", value: String(atCs))]
        if let origin {
            parts.queryItems! += [URLQueryItem(name: "origin", value: origin), URLQueryItem(name: "src", value: String(id))]
        }
        return parts.url!
    }

    static func parse(_ url: URL) -> Target? {
        guard url.scheme == "whimbrel", url.host == "meeting", let id = Int64(url.lastPathComponent) else { return nil }
        let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func value(_ name: String) -> String? { items.first { $0.name == name }?.value }
        guard let origin = value("origin") else {
            return Target(meetingId: id, atCs: value("at").flatMap { Int64($0) } ?? 0)
        }
        guard !origin.isEmpty, origin.count <= 128, let src = value("src").flatMap({ Int64($0) }) else { return nil }
        return Target(meetingId: id, atCs: value("at").flatMap { Int64($0) } ?? 0, origin: origin, sourceMeetingId: src)
    }

    /// This device's id for the link's meeting: the id itself when this device made the link
    /// (or it names no device), else the peer meeting's local id from `lookup`
    /// (origin, source meeting id), nil when it hasn't synced here.
    static func localMeetingId(_ target: Target, deviceId: String, lookup: (String, Int64) -> Int64?) -> Int64? {
        guard let origin = target.origin, origin != deviceId else { return target.meetingId }
        return target.sourceMeetingId.flatMap { lookup(origin, $0) }
    }
}

/// Delivers live question alerts: a notification on this Mac (time-sensitive when allowed), an
/// email to the user's own Gmail, and notifications for alerts other devices heard, after sync
/// pulls them. Every side effect is a closure so tests use fakes.
/// See docs/superpowers/specs/2026-11-02-live-question-alerts-design.md.
@MainActor
final class LiveAlertDelivery: AlertDelivering {
    struct Notice: Equatable {
        let id: String
        let title: String
        var subtitle: String?
        let body: String
        let deepLink: URL?
        /// Only a notice's first post plays a sound; replacing it in place (a draft arriving,
        /// the "N more" count growing) is silent.
        var sound = true
    }

    /// The email waits this long for the draft, and a failed send is retried once after it.
    static let emailWait: UInt64 = 60
    /// Peer alerts shown per (origin device, meeting) before one "N more" summary.
    static let peerCeiling = LiveQuestionWatcher.maxAlerts
    /// Peer alerts older than this when pulled only appear on the meeting page.
    static let peerMaxAgeMs: Int64 = 30 * 60 * 1000
    /// No keyboard or mouse input for this long counts as away from the Mac.
    static let awayIdleSeconds: TimeInterval = 5 * 60

    /// The email is for a question the user missed: only during an Away Mode meeting, or after
    /// the Mac has sat idle. In a meeting they're at, they heard it (Mike, 2026-11-05).
    nonisolated static func isAway(awayModeRunning: Bool, idleSeconds: TimeInterval) -> Bool {
        awayModeRunning || idleSeconds >= awayIdleSeconds
    }

    private let deviceId: String
    private let alertsEnabled: () -> Bool
    private let postNotice: (Notice) -> Void
    private let sendEmail: (String, String) async -> Result<Void, SelfSendError>
    private let setEmailStatus: (Int64, String) async -> Void
    private let emailEnabled: () -> Bool
    private let userAway: () -> Bool
    private let evidence: ([CoreContextItemRef]) async -> [String]
    private let context: (CoreLiveAlert) async -> [String]
    private let sleep: (UInt64) async -> Void

    /// Notice ids already posted, so a replacement is silent.
    private var postedIds = Set<String>()
    /// Alerts whose email hasn't gone yet; the timer and the draft race to remove it.
    private var emailPending = Set<String>()
    // ponytail: in memory, so a relaunch mid-meeting restarts the peer count; persist if that matters.
    private var peerShown: [String: Int] = [:]
    private var peerExtra: [String: Int] = [:]
    // ponytail: grows by two small tasks per alert for the app's life; prune if alerts ever become frequent.
    private var tasks: [Task<Void, Never>] = []

    init(
        deviceId: String,
        alertsEnabled: @escaping () -> Bool,
        post: @escaping (Notice) -> Void,
        sendEmail: @escaping (String, String) async -> Result<Void, SelfSendError>,
        setEmailStatus: @escaping (Int64, String) async -> Void,
        emailEnabled: @escaping () -> Bool,
        userAway: @escaping () -> Bool,
        evidence: @escaping ([CoreContextItemRef]) async -> [String],
        context: @escaping (CoreLiveAlert) async -> [String],
        sleep: @escaping (UInt64) async -> Void = { try? await Task.sleep(nanoseconds: $0 * 1_000_000_000) }
    ) {
        self.deviceId = deviceId
        self.alertsEnabled = alertsEnabled
        self.postNotice = post
        self.sendEmail = sendEmail
        self.setEmailStatus = setEmailStatus
        self.emailEnabled = emailEnabled
        self.userAway = userAway
        self.evidence = evidence
        self.context = context
        self.sleep = sleep
    }

    // MARK: - Content

    static func content(for alert: CoreLiveAlert, answer: CoreChatAnswer?) -> (title: String, body: String) {
        let asker = alert.asker?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let title = asker.isEmpty ? String(localized: "Question for you") : String(localized: "\(asker) asked you")
        let firstLine = answer?.text.split(whereSeparator: \.isNewline).first.map(String.init)
        return (title, [alert.restatement, firstLine].compactMap { $0 }.joined(separator: "\n"))
    }

    static func emailSubject(for alert: CoreLiveAlert) -> String {
        String(localized: "Question for you in \"\(alert.meetingTitle)\"")
    }

    /// `context` is the meeting's lines from the 2 minutes before the question
    /// (`contextLines`); `evidence` is one "title · date" line per cited item.
    static func emailBody(
        alert: CoreLiveAlert, answer: CoreChatAnswer?, meetingURL: URL, context: [String] = [], evidence: [String] = []
    ) -> String {
        let asker = alert.asker?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let at = clock(alert.atCs)
        var lines = context.isEmpty ? [] : [String(localized: "Just before:")] + context + [""]
        lines += [
            "\u{201C}\(alert.quote)\u{201D}",
            asker.isEmpty
                ? String(localized: "Someone asked at \(at) in \"\(alert.meetingTitle)\".")
                : String(localized: "\(asker) asked at \(at) in \"\(alert.meetingTitle)\"."),
            String(localized: "In short: \(alert.restatement)"),
            "",
            String(localized: "Draft answer:"),
            answer?.text ?? String(localized: "Draft wasn't ready in time; it's on the meeting page."),
        ]
        if !evidence.isEmpty {
            lines += ["", String(localized: "Evidence:")] + evidence.map { "- \($0)" }
        }
        lines += ["", String(localized: "Open the meeting in Whimbrel: \(meetingURL.absoluteString)")]
        return lines.joined(separator: "\n")
    }

    /// "m:ss  Speaker: text" for each live line starting in the 2 minutes before `atCs`. Live
    /// rows count from the current part's start, `partOffsetCs` after the meeting's, which is
    /// where `atCs` and the printed times count from.
    nonisolated static func contextLines(_ segments: [CoreLiveSegment], atCs: Int64, partOffsetCs: Int64 = 0) -> [String] {
        segments
            .map { (segment: $0, at: $0.startCs + partOffsetCs) }
            .filter { $0.at >= atCs - 12_000 && $0.at < atCs }
            .sorted { ($0.at, $0.segment.id) < ($1.at, $1.segment.id) }
            .map { "\(clock($0.at))  \($0.segment.speakerLabel.isEmpty ? $0.segment.channel : $0.segment.speakerLabel): \($0.segment.text)" }
    }

    nonisolated private static func clock(_ cs: Int64) -> String {
        let seconds = cs / 100
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }

    /// One line per cited item for the email: title · date, plus a link where there is one.
    /// Call on `CoreMemoryStore.accessQueue`.
    nonisolated static func evidenceLines(_ refs: [CoreContextItemRef], store: CoreMemoryStore, origin: String? = nil) -> [String] {
        evidence(refs, store: store, origin: origin).map { ([$0.text] + [$0.link?.absoluteString].compactMap { $0 }).joined(separator: " · ") }
    }

    struct Evidence: Equatable, Hashable {
        let text: String
        let link: URL?
    }

    /// `url` when it's http or https (any case), else nil: the only links a capture may carry,
    /// the same rule as `AskAnswerText`.
    nonisolated static func webLink(_ url: URL) -> URL? {
        ["http", "https"].contains(url.scheme?.lowercased() ?? "") ? url : nil
    }

    /// "title · date", or just the title when the date is unknown.
    nonisolated static func evidenceText(_ title: String, unixMs: Int64?, formatter: DateFormatter) -> String {
        guard let unixMs else { return title }
        return "\(title) · \(formatter.string(from: Date(timeIntervalSince1970: Double(unixMs) / 1000)))"
    }

    /// Each cited item once, as "title · date" and its link, if any. Transcript citations name
    /// their meeting and link to that moment (carrying `origin`, this device's id, when the link
    /// leaves the device in an email); slide and routine citations get a plain label.
    /// Call on `CoreMemoryStore.accessQueue`.
    nonisolated static func evidence(_ refs: [CoreContextItemRef], store: CoreMemoryStore, origin: String? = nil) -> [Evidence] {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        func item(_ title: String, _ ms: Int64?, _ link: URL? = nil) -> Evidence {
            Evidence(text: evidenceText(title, unixMs: ms, formatter: formatter), link: link)
        }
        var out: [Evidence] = []
        for ref in refs {
            let entry: Evidence
            switch ref.kind {
            case .capture:
                guard let capture = store.getCaptureItem(id: ref.id) else { continue }
                entry = item(capture.windowTitle.isEmpty ? capture.appBundleId : capture.windowTitle, capture.tsUnixMs,
                             capture.url.flatMap(URL.init(string:)).flatMap(webLink))
            case .thread:
                guard let thread = store.getThread(id: ref.id) else { continue }
                entry = item(thread.subject ?? thread.lastMessageFrom, thread.lastActivityTsUnixMs)
            case .episode:
                guard let episode = store.getEpisode(id: ref.id) else { continue }
                entry = item(episode.windowTitle ?? episode.appBundleId, episode.startTsUnixMs)
            case .transcript:
                guard let segment = store.getTranscriptSegment(id: ref.id) else { continue }
                entry = item(segment.meetingTitle, store.getMeeting(id: segment.meetingId)?.startUnixMs,
                             DeepLink.meetingURL(id: segment.meetingId, atCs: segment.startCs, origin: origin))
            case .snapshot: entry = Evidence(text: String(localized: "A meeting slide"), link: nil)
            case .routineRun: entry = Evidence(text: String(localized: "A routine result"), link: nil)
            }
            out.append(entry)
        }
        return deduped(out)
    }

    /// Each "title · date" once, in first-seen order, with the first link any of its copies has.
    nonisolated static func deduped(_ items: [Evidence]) -> [Evidence] {
        var out: [Evidence] = []
        for item in items {
            if let i = out.firstIndex(where: { $0.text == item.text }) {
                if out[i].link == nil { out[i] = item }
            } else {
                out.append(item)
            }
        }
        return out
    }

    /// An alert's notification link: its meeting here, or, for a peer's alert whose meeting
    /// hasn't synced to this device yet, the peer's own link, resolved when it's tapped.
    nonisolated static func link(for alert: CoreLiveAlert, atCs: Int64) -> URL? {
        if let id = alert.meetingId { return DeepLink.meetingURL(id: id, atCs: atCs) }
        return alert.originDeviceId.map { DeepLink.meetingURL(id: alert.sourceMeetingId, atCs: atCs, origin: $0) }
    }

    /// Another device's alert names its meeting, which isn't the one on screen here.
    private func notice(for alert: CoreLiveAlert, answer: CoreChatAnswer?) -> Notice {
        let (title, body) = Self.content(for: alert, answer: answer)
        return Notice(
            id: "live-alert-\(alert.originDeviceId ?? deviceId)-\(alert.sourceId)", title: title,
            subtitle: alert.originDeviceId == nil ? nil : alert.meetingTitle, body: body,
            deepLink: Self.link(for: alert, atCs: alert.atCs)
        )
    }

    private func post(_ notice: Notice) {
        var notice = notice
        notice.sound = postedIds.insert(notice.id).inserted
        postNotice(notice)
    }

    // MARK: - This Mac's alerts

    func alertDetected(_ alert: CoreLiveAlert) {
        post(notice(for: alert, answer: nil))
        let status = !emailEnabled() ? "off" : !userAway() ? "present" : nil
        if let status {
            // id -1: the store insert failed, so there's no row to mark.
            if alert.id >= 0 { track { [setEmailStatus] in await setEmailStatus(alert.id, status) } }
            return
        }
        emailPending.insert(key(alert))
        track { [self] in
            await sleep(Self.emailWait)
            await email(alert, answer: nil)
        }
    }

    func alertDrafted(_ alert: CoreLiveAlert, answer: CoreChatAnswer?) {
        if let answer { post(notice(for: alert, answer: answer)) }
        track { [self] in await email(alert, answer: answer) }
    }

    func alertsCapped(meetingId: Int64, extra: Int) {
        post(Self.summary(origin: deviceId, sourceMeetingId: meetingId, title: "Whimbrel", extra: extra,
                          deepLink: DeepLink.meetingURL(id: meetingId, atCs: 0)))
    }

    /// Sends the alert's email once: whichever of the 60 s timer and the draft comes first.
    private func email(_ alert: CoreLiveAlert, answer: CoreChatAnswer?) async {
        guard emailPending.remove(key(alert)) != nil else { return }
        let link = DeepLink.meetingURL(id: alert.meetingId ?? alert.sourceMeetingId, atCs: alert.atCs, origin: deviceId)
        let body = Self.emailBody(
            alert: alert, answer: answer, meetingURL: link, context: await context(alert), evidence: await evidence(answer?.citations ?? [])
        )
        let subject = Self.emailSubject(for: alert)
        var result = await sendEmail(subject, body)
        if case .failure = result {
            await sleep(Self.emailWait)
            result = await sendEmail(subject, body)
        }
        guard alert.id >= 0 else { return }
        switch result {
        case .success: await setEmailStatus(alert.id, "sent")
        case .failure(let error): await setEmailStatus(alert.id, "failed:\(error.reason)")
        }
    }

    private func key(_ alert: CoreLiveAlert) -> String { "\(alert.originDeviceId ?? deviceId)-\(alert.sourceId)" }

    // MARK: - Other devices' alerts

    /// After a content sync pull: notifies the alerts other devices heard, at most
    /// `peerCeiling` per (origin, meeting), then one "N more" summary updated in place.
    func afterPull(take: () async -> [CoreLiveAlert]) async {
        // Taken even when alerts are off here, so they're marked notified and never fire later.
        let alerts = await take()
        guard alertsEnabled() else { return }
        var summaries: [String: CoreLiveAlert] = [:]
        for alert in alerts {
            guard let origin = alert.originDeviceId else { continue }
            let meetingKey = "\(origin)-\(alert.sourceMeetingId)"
            if peerShown[meetingKey, default: 0] < Self.peerCeiling {
                peerShown[meetingKey, default: 0] += 1
                let draft = alert.draftJson.flatMap { try? JSONDecoder().decode(CoreChatAnswer.self, from: Data($0.utf8)) }
                post(notice(for: alert, answer: draft))
            } else {
                peerExtra[meetingKey, default: 0] += 1
                summaries[meetingKey] = alert
            }
        }
        for (meetingKey, alert) in summaries.sorted(by: { $0.key < $1.key }) {
            post(Self.summary(
                origin: alert.originDeviceId ?? deviceId, sourceMeetingId: alert.sourceMeetingId, title: alert.meetingTitle,
                extra: peerExtra[meetingKey, default: 0], deepLink: Self.link(for: alert, atCs: 0)
            ))
        }
    }

    private static func summary(origin: String, sourceMeetingId: Int64, title: String, extra: Int, deepLink: URL?) -> Notice {
        Notice(id: "live-alert-more-\(origin)-\(sourceMeetingId)", title: title, body: String(localized: "\(extra) more questions for you"), deepLink: deepLink)
    }

    // MARK: - Tasks

    private func track(_ work: @escaping @MainActor () async -> Void) {
        tasks.append(Task { @MainActor in await work() })
    }

    /// Waits for every email and status write started so far (tests).
    func settle() async {
        while !tasks.isEmpty {
            let pending = tasks
            tasks = []
            for task in pending { await task.value }
        }
    }

    // MARK: - The real notification center

    /// Posts `notice` (replacing any with the same identifier). Time-sensitive when the user
    /// allows it for Whimbrel, otherwise an ordinary notification. The app doesn't carry the
    /// time-sensitive entitlement yet (it needs a provisioning profile the release build doesn't
    /// have), so today this is always an ordinary notification.
    static func postToNotificationCenter(_ notice: Notice) {
        let center = UNUserNotificationCenter.current()
        Task {
            let settings = await center.notificationSettings()
            let content = UNMutableNotificationContent()
            content.title = notice.title
            if let subtitle = notice.subtitle { content.subtitle = subtitle }
            content.body = notice.body
            content.sound = notice.sound ? .default : nil
            if settings.timeSensitiveSetting == .enabled { content.interruptionLevel = .timeSensitive }
            if let link = notice.deepLink { content.userInfo = ["deepLink": link.absoluteString] }
            do {
                try await center.add(UNNotificationRequest(identifier: notice.id, content: content, trigger: nil))
            } catch {
                Logger(subsystem: "com.mpointer.kestrel", category: "live-question")
                    .error("Failed to post live question notification: \(error.localizedDescription, privacy: .public)")
            }
        }
    }
}

/// Notification clicks: an alert opens its meeting at the moment it was asked; a budget notice
/// opens Settings → AI Providers. Also lets live question alerts and budget notices show while
/// Whimbrel is the frontmost app; every other notification stays hidden there, as before this
/// delegate existed.
final class NotificationClickHandler: NSObject, UNUserNotificationCenterDelegate {
    private let open: @MainActor (DeepLink.Target) -> Void
    private let openPrep: @MainActor (String) -> Void
    private let openAIProviders: @MainActor () -> Void
    /// The low-disk notice: Settings → Storage with Free up space showing.
    private let openStorage: @MainActor () -> Void
    /// "Still in a meeting?"'s buttons: true for "Keep recording", false for "Stop".
    private let stillInMeeting: @MainActor (_ keep: Bool) -> Void

    /// Meeting Prep notifications carry their brief's key here, not a deep link.
    static let prepKeyInfo = "prepKey"
    /// Identifier prefix of Meeting Prep's "Prep ready" notifications.
    static let prepIdentifierPrefix = "meeting-prep-"

    init(open: @escaping @MainActor (DeepLink.Target) -> Void, openPrep: @escaping @MainActor (String) -> Void = { _ in },
         openAIProviders: @escaping @MainActor () -> Void,
         openStorage: @escaping @MainActor () -> Void = {},
         stillInMeeting: @escaping @MainActor (_ keep: Bool) -> Void = { _ in }) {
        self.open = open
        self.openPrep = openPrep
        self.openAIProviders = openAIProviders
        self.openStorage = openStorage
        self.stillInMeeting = stillInMeeting
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let content = response.notification.request.content
        let link = (content.userInfo["deepLink"] as? String).flatMap(URL.init(string:)).flatMap(DeepLink.parse)
        let prepKey = content.userInfo[Self.prepKeyInfo] as? String
        let click = prepKey != nil && response.actionIdentifier == UNNotificationDefaultActionIdentifier
            ? .openPrep : Self.click(category: content.categoryIdentifier, action: response.actionIdentifier, hasLink: link != nil)
        let open = open
        let openPrep = openPrep
        let openAIProviders = openAIProviders
        let openStorage = openStorage
        let stillInMeeting = stillInMeeting
        Task { @MainActor in
            switch click {
            case .openAIProviders: openAIProviders()
            case .openStorage: openStorage()
            case .stillInMeeting(let keep): stillInMeeting(keep)
            case .openLink: if let link { open(link) }
            case .openPrep: if let prepKey { openPrep(prepKey) }
            case .activate: NSApp.activate(ignoringOtherApps: true)
            case .ignore: break
            }
            completionHandler()
        }
    }

    enum Click: Equatable { case openAIProviders, openStorage, stillInMeeting(keep: Bool), openLink, openPrep, activate, ignore }

    /// What a click does: a budget notice's body or its "Raise budget" action opens Settings →
    /// AI Providers and dismissing it does nothing; "Still in a meeting?"'s buttons answer it
    /// (its body brings Whimbrel forward, where the panel shows); anything else opens its deep link, or
    /// brings Whimbrel forward.
    static func click(category: String, action: String, hasLink: Bool) -> Click {
        if category == BudgetNotices.category { return action == UNNotificationDismissActionIdentifier ? .ignore : .openAIProviders }
        if category == LowDiskNotice.category { return action == UNNotificationDismissActionIdentifier ? .ignore : .openStorage }
        if category == StillInMeeting.notificationCategory, [StillInMeeting.keepAction, StillInMeeting.stopAction].contains(action) {
            return .stillInMeeting(keep: action == StillInMeeting.keepAction)
        }
        return hasLink ? .openLink : .activate
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler(Self.presentationOptions(forIdentifier: notification.request.identifier))
    }

    static func presentationOptions(forIdentifier id: String) -> UNNotificationPresentationOptions {
        // Budget notices usually follow the user's own Ask, with Whimbrel in front; a prep brief
        // lands minutes before a meeting, often while Whimbrel is in front.
        ["live-alert-", "budget-", "low-disk-", prepIdentifierPrefix].contains(where: id.hasPrefix) ? [.banner, .list, .sound] : []
    }
}
