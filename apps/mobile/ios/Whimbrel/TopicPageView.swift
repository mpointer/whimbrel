import SwiftUI

/// A topic page as the computer built it, read-only, in the Mac's order: the lead, your role,
/// the next meeting, what you owe, what is owed to you, what was done recently and recent items.
/// A meeting the replica holds opens its detail; the stack's root registers that destination.
struct TopicPageView: View {
    @ObservedObject var sync: SyncModel
    @StateObject private var model: TopicPageModel

    init(key: String, sync: SyncModel) {
        _sync = ObservedObject(wrappedValue: sync)
        _model = StateObject(wrappedValue: TopicPageModel(key: key))
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let snap = model.snapshot {
                    page(snap)
                } else {
                    Text(model.loaded ? "This topic isn't on this iPhone." : "Loading…")
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
        }
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle(model.snapshot?.name ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: sync.readRevision) { await model.reload(sync: sync) }
        .refreshable { await sync.syncNow(manual: true) }
    }

    @ViewBuilder
    private func page(_ snap: PhoneTopicSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(snap.name)
                .font(.system(.title, design: .serif))
                .foregroundStyle(Color.theme(.textPrimary))
            if let lead = snap.lead {
                Text(TopicText.lead(lead))
                    .font(.subheadline)
                    .foregroundStyle(Color.theme(.textSecondary))
            }
        }
        if let role = snap.role, !role.text.isEmpty {
            section("Your role") { Text(role.text).foregroundStyle(Color.theme(.textPrimary)) }
        }
        if let next = snap.upcomingMeeting() {
            section("Next meeting") { nextMeeting(next) }
        }
        section("You owe") { steps(snap.youOwe, empty: "Nothing you owe right now.") }
        section("Owed to you") { steps(snap.owedToYou, empty: "Nothing owed to you right now.") }
        if !snap.done.isEmpty {
            section("Done recently") { steps(snap.done, empty: nil, done: true) }
        }
        section("Recent") { recent(snap) }
        if snap.truncated {
            Text("Showing the latest items. See the rest on your computer.")
                .font(.footnote)
                .foregroundStyle(Color.theme(.textSecondary))
        }
        TimelineView(.periodic(from: .now, by: 60)) { context in
            Text(TopicText.footer(snap, now: context.date))
                .font(.footnote)
                .foregroundStyle(Color.theme(.textSecondary))
                .frame(maxWidth: .infinity)
        }
    }

    // MARK: Next meeting

    private func nextMeeting(_ next: PhoneTopicSnapshot.NextMeeting) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(next.title)
                .font(.headline)
                .foregroundStyle(Color.theme(.textPrimary))
            Text(TopicText.dayTime(ms: next.startUnixMs))
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Color.theme(.accentPrimary))
            let orgs = TopicText.attendees(next)
            if !orgs.isEmpty {
                Text(orgs).font(.subheadline).foregroundStyle(Color.theme(.textSecondary))
            }
        }
    }

    // MARK: Steps

    @ViewBuilder
    private func steps(_ list: [PhoneTopicSnapshot.Step], empty: LocalizedStringKey?, done: Bool = false) -> some View {
        if list.isEmpty {
            if let empty { Text(empty).font(.subheadline).foregroundStyle(Color.theme(.textSecondary)) }
        } else {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(Array(list.enumerated()), id: \.offset) { _, step in
                    linked(step.meeting) { stepRow(step, done: done) }
                }
            }
        }
    }

    private func stepRow(_ step: PhoneTopicSnapshot.Step, done: Bool) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(step.text)
                .foregroundStyle(Color.theme(.textPrimary))
                .strikethrough(done)
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                if let title = step.sourceTitle, !title.isEmpty {
                    Label(title, systemImage: Self.icon(step.sourceKind)).lineLimit(1)
                }
                if let asked = step.askedAtUnixMs { Text("asked \(TopicText.monthDay(ms: asked))") }
                if let due = step.dueDate {
                    Text(TopicText.dueText(due))
                        .foregroundStyle(TopicText.isOverdue(due) && !done ? Color.theme(.error) : Color.theme(.textSecondary))
                }
            }
            .font(.caption)
            .foregroundStyle(Color.theme(.textSecondary))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    // MARK: Recent

    @ViewBuilder
    private func recent(_ snap: PhoneTopicSnapshot) -> some View {
        if snap.recent.isEmpty {
            Text("Nothing yet.").font(.subheadline).foregroundStyle(Color.theme(.textSecondary))
        } else {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(Array(snap.recent.enumerated()), id: \.offset) { _, item in
                    linked(item.meeting) { recentRow(item) }
                }
                if snap.recentTotal > Int64(snap.recent.count) {
                    Text("\(snap.recentTotal - Int64(snap.recent.count)) older items not shown")
                        .font(.caption)
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
        }
    }

    private func recentRow(_ item: PhoneTopicSnapshot.Recent) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: Self.icon(item.kind)).foregroundStyle(Color.theme(.textSecondary))
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title.isEmpty ? item.kind.capitalized : item.title)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .multilineTextAlignment(.leading)
                Text(([item.sender].compactMap { $0 } + [TopicText.monthDay(ms: item.tsUnixMs)]).joined(separator: " · "))
                    .font(.subheadline)
                    .foregroundStyle(Color.theme(.textSecondary))
                if item.kind == "email" {
                    Text("Open on your computer").font(.caption).foregroundStyle(Color.theme(.textSecondary))
                }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
    }

    // MARK: Pieces

    /// A link to the meeting's detail when the replica holds it; otherwise the row as is.
    @ViewBuilder
    private func linked<Content: View>(_ ref: PhoneMeetingRef?, @ViewBuilder _ content: () -> Content) -> some View {
        if let ref, let id = model.meetingIds[ref] {
            NavigationLink(value: id) { content() }.buttonStyle(.plain)
        } else {
            content()
        }
    }

    private static func icon(_ kind: String) -> String {
        switch kind {
        case "meeting": "person.2"
        case "email": "envelope"
        default: "doc.text"
        }
    }

    private func section<Content: View>(_ title: LocalizedStringKey, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .textCase(.uppercase)
                .font(.caption.weight(.semibold))
                .tracking(0.6)
                .foregroundStyle(Color.theme(.textSecondary))
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.theme(.border), lineWidth: 1))
    }
}
