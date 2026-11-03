import Combine
import SwiftUI

/// "Questions for you": the live question alerts heard in a meeting, each with its draft
/// answer, evidence chips, "Jump to moment" and the email status. Shown on the meeting page
/// and in the live section; renders nothing when there are no alerts and alerts aren't paused.
/// The host loads `rows` with `load(store:meetingId:update:)` and reloads on `refresh`.
/// See docs/superpowers/specs/2026-11-02-live-question-alerts-design.md.
struct LiveAlertsSection: View {
    struct Row: Identifiable, Equatable {
        let alert: CoreLiveAlert
        let draft: String?
        let evidence: [LiveAlertDelivery.Evidence]
        /// The draft cites sources this device can't resolve: another device recorded it.
        var sourcesElsewhere = false
        var id: Int64 { alert.id }
    }

    @EnvironmentObject var appState: AppState
    @Environment(\.openURL) private var openURL
    let rows: [Row]
    var paused: String?
    /// Scrolls the host's transcript to a moment (centiseconds); nil hides "Jump to moment".
    var onJump: ((Int64) -> Void)?

    // ponytail: polls instead of a change notification; while recording, new alerts, drafts and
    // email status show within 10 s (`shouldPoll`). Add a notification if that lag matters.
    static let refresh = Timer.publish(every: 10, on: .main, in: .common).autoconnect()

    var body: some View {
        if !rows.isEmpty || paused != nil {
            VStack(alignment: .leading, spacing: Spacing.md) {
                Text("Questions for you").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                if let paused {
                    Text(paused).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                }
                ForEach(rows) { card($0) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(Spacing.md)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: Radius.card))
            .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(Color.whimbrelBorder))
        }
    }

    private func card(_ row: Row) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            Text(row.alert.restatement).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            Text(verbatim: "\u{201C}\(row.alert.quote)\u{201D}")
                .font(.whimbrelBody(.callout)).italic().foregroundStyle(Color.whimbrelTextSecondary)
            if let draft = row.draft {
                AskAnswerText(markdown: draft)
            } else {
                Text("No draft answer yet.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if row.sourcesElsewhere {
                Text("Sources are on the device that recorded this meeting.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if !row.evidence.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: Spacing.xs) {
                        ForEach(row.evidence, id: \.self) { item in
                            Button { open(item.link) } label: { Text(verbatim: item.text) }
                                .buttonStyle(.whimbrelSecondary).controlSize(.small).disabled(item.link == nil)
                        }
                    }
                }
            }
            HStack(spacing: Spacing.sm) {
                if let onJump, row.alert.meetingId != nil {
                    Button("Jump to moment") { onJump(row.alert.atCs) }.buttonStyle(.whimbrelSecondary).controlSize(.small)
                }
                if let status = Self.emailStatusText(row.alert.emailStatus) {
                    Text(status).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
                }
            }
        }
    }

    private func open(_ link: URL?) {
        guard let link else { return }
        if let target = DeepLink.parse(link) {
            appState.open(target)
        } else if let web = LiveAlertDelivery.webLink(link) {
            openURL(web)
        }
    }

    /// "Question alerts paused" while live transcription is paused and alerts would run.
    static func pausedLabel(status: LiveTranscriptionService.Status, alertsOn: Bool) -> String? {
        guard case .paused = status, alertsOn else { return nil }
        return String(localized: "Question alerts paused")
    }

    /// The status line for a stored `email_status`: only a failure is shown.
    static func emailStatusText(_ status: String?) -> String? {
        guard let status, status.hasPrefix("failed:") else { return nil }
        let reason = String(status.dropFirst("failed:".count))
        return String(localized: "Email not sent: \(reason)")
    }

    /// The host reloads alerts every 10 s only while this meeting records with alerts on;
    /// otherwise it loads them once.
    static func shouldPoll(recording: Bool, alertsOn: Bool) -> Bool {
        recording && alertsOn
    }

    /// One card's data. Evidence ids are the origin device's local ids, so an alert another
    /// device heard shows no chips, only a note that its sources live there. `resolve` turns
    /// citations into chips (on `CoreMemoryStore.accessQueue` in the app).
    static func row(_ alert: CoreLiveAlert, resolve: ([CoreContextItemRef]) -> [LiveAlertDelivery.Evidence]) -> Row {
        let answer = alert.draftJson.flatMap { try? JSONDecoder().decode(CoreChatAnswer.self, from: Data($0.utf8)) }
        let citations = answer?.citations ?? []
        guard alert.originDeviceId == nil else {
            return Row(alert: alert, draft: answer?.text, evidence: [], sourcesElsewhere: !citations.isEmpty)
        }
        return Row(alert: alert, draft: answer?.text, evidence: citations.isEmpty ? [] : resolve(citations))
    }

    /// The id of the line whose start is closest to `atCs`.
    static func nearest(_ lines: [(id: Int64, startCs: Int64)], to atCs: Int64) -> Int64? {
        lines.min { abs($0.startCs - atCs) < abs($1.startCs - atCs) }?.id
    }

    /// Loads the meeting's alerts with their decoded drafts and evidence off main, then calls
    /// `update` on main.
    static func load(store: CoreMemoryStore?, meetingId: Int64, update: @escaping ([Row]) -> Void) {
        guard let store else { return }
        CoreMemoryStore.accessQueue.async {
            let rows = store.listLiveAlerts(meetingId: meetingId).map { row($0) { LiveAlertDelivery.evidence($0, store: store) } }
            DispatchQueue.main.async { update(rows) }
        }
    }
}
