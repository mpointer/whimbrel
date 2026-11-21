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
            VStack(alignment: .leading, spacing: WTokens.Space.lg) {
                if let snap = model.snapshot {
                    page(snap)
                } else if model.loaded {
                    WStateView(kind: .empty, title: String(localized: "This topic isn't on this iPhone."))
                } else {
                    WStateView(kind: .loading, title: String(localized: "Loading…"))
                }
            }
            .padding(.horizontal, WTokens.Space.lg)
            .padding(.vertical, WTokens.Space.md)
            .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
            .frame(maxWidth: .infinity)
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
        VStack(alignment: .leading, spacing: WTokens.Space.xs) {
            Text(snap.name)
                .wReading(WTokens.TypeScale.readingTitle)
                .foregroundStyle(Color.theme(.textPrimary))
                .accessibilityAddTraits(.isHeader)
            if let lead = snap.lead {
                Text(TopicText.lead(lead))
                    .wType(WTokens.TypeScale.secondary)
                    .foregroundStyle(Color.theme(.textSecondary))
            }
        }
        if let role = snap.role, !role.text.isEmpty {
            section("Your role") { Text(role.text).wType(WTokens.TypeScale.body).foregroundStyle(Color.theme(.textPrimary)) }
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
                .wType(WTokens.TypeScale.secondary)
                .foregroundStyle(Color.theme(.textSecondary))
        }
        TimelineView(.periodic(from: .now, by: 60)) { context in
            Text(TopicText.footer(snap, now: context.date))
                .wType(WTokens.TypeScale.secondary)
                .multilineTextAlignment(.center)
                .foregroundStyle(Color.theme(.textSecondary))
                .frame(maxWidth: .infinity)
        }
    }

    // MARK: Next meeting

    private func nextMeeting(_ next: PhoneTopicSnapshot.NextMeeting) -> some View {
        VStack(alignment: .leading, spacing: WTokens.Space.xs) {
            Text(next.title)
                .wType(WTokens.TypeScale.subtitle)
                .foregroundStyle(Color.theme(.textPrimary))
            Text(TopicText.dayTime(ms: next.startUnixMs))
                .wType(WTokens.TypeScale.time)
                .foregroundStyle(Color.theme(.accentPrimary))
            let orgs = TopicText.attendees(next)
            if !orgs.isEmpty {
                Text(orgs).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
            }
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: Steps

    @ViewBuilder
    private func steps(_ list: [PhoneTopicSnapshot.Step], empty: LocalizedStringKey?, done: Bool = false) -> some View {
        if list.isEmpty {
            if let empty { Text(empty).wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary)) }
        } else {
            VStack(alignment: .leading, spacing: WTokens.Space.md) {
                ForEach(Array(list.enumerated()), id: \.offset) { _, step in
                    linked(step.meeting) { stepRow(step, done: done) }
                }
            }
        }
    }

    /// A step and where it came from. A done step has a check as well as the strike, and says
    /// so to a screen reader. The details sit side by side, or stacked when the text is large.
    private func stepRow(_ step: PhoneTopicSnapshot.Step, done: Bool) -> some View {
        HStack(alignment: .top, spacing: WTokens.Space.sm) {
            if done {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(Color.theme(.success))
                    .padding(.top, 2).accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
                Text(step.text)
                    .wType(WTokens.TypeScale.body)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .strikethrough(done)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityLabel(done ? String(localized: "Done: \(step.text)") : step.text)
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: WTokens.Space.sm) { stepDetails(step, done: done, stacked: false) }
                    VStack(alignment: .leading, spacing: WTokens.Space.xxs) { stepDetails(step, done: done, stacked: true) }
                }
                .wType(WTokens.TypeScale.caption)
                .foregroundStyle(Color.theme(.textSecondary))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private func stepDetails(_ step: PhoneTopicSnapshot.Step, done: Bool, stacked: Bool) -> some View {
        if let title = step.sourceTitle, !title.isEmpty {
            // Side by side the title keeps to one line; stacked (large text) it may wrap rather than be cut off.
            Label(title, systemImage: Self.icon(step.sourceKind)).lineLimit(stacked ? 3 : 1)
                .multilineTextAlignment(.leading)
        }
        if let asked = step.askedAtUnixMs { Text("asked \(TopicText.monthDay(ms: asked))") }
        if let due = step.dueDate {
            Text(TopicText.dueText(due))
                .fontWeight(TopicText.isOverdue(due) && !done ? .semibold : .regular)
                .foregroundStyle(TopicText.isOverdue(due) && !done ? Color.theme(.error) : Color.theme(.textSecondary))
        }
    }

    // MARK: Recent

    @ViewBuilder
    private func recent(_ snap: PhoneTopicSnapshot) -> some View {
        if snap.recent.isEmpty {
            Text("Nothing yet.").wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
        } else {
            VStack(alignment: .leading, spacing: WTokens.Space.md) {
                ForEach(Array(snap.recent.enumerated()), id: \.offset) { _, item in
                    linked(item.meeting) { recentRow(item) }
                }
                if snap.recentTotal > Int64(snap.recent.count) {
                    Text("\(snap.recentTotal - Int64(snap.recent.count)) older items not shown")
                        .wType(WTokens.TypeScale.caption)
                        .foregroundStyle(Color.theme(.textSecondary))
                }
            }
        }
    }

    private func recentRow(_ item: PhoneTopicSnapshot.Recent) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: WTokens.Space.md) {
            Image(systemName: Self.icon(item.kind)).foregroundStyle(Color.theme(.textSecondary)).frame(width: 24).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: WTokens.Space.xxs) {
                Text(item.title.isEmpty ? item.kind.capitalized : item.title)
                    .wType(WTokens.TypeScale.body)
                    .foregroundStyle(Color.theme(.textPrimary))
                    .multilineTextAlignment(.leading)
                Text(([item.sender].compactMap { $0 } + [TopicText.monthDay(ms: item.tsUnixMs)]).joined(separator: " · "))
                    .wType(WTokens.TypeScale.secondary)
                    .foregroundStyle(Color.theme(.textSecondary))
                if item.kind == "email" {
                    Text("Open on your computer").wType(WTokens.TypeScale.caption).foregroundStyle(Color.theme(.textSecondary))
                }
            }
        }
        .frame(maxWidth: .infinity, minHeight: WTokens.Size.target, alignment: .leading)
        .accessibilityElement(children: .combine)
        .contentShape(Rectangle())
    }

    // MARK: Pieces

    /// A link to the meeting's detail when the replica holds it; otherwise the row as is.
    @ViewBuilder
    private func linked<Content: View>(_ ref: PhoneMeetingRef?, @ViewBuilder _ content: () -> Content) -> some View {
        if let ref, let id = model.meetingIds[ref] {
            NavigationLink(value: id) {
                HStack(alignment: .center, spacing: WTokens.Space.sm) {
                    content()
                    Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                        .foregroundStyle(Color.theme(.textSecondary)).accessibilityHidden(true)
                }
                .frame(minHeight: WTokens.Size.target)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityHint("Opens the source")
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
        VStack(alignment: .leading, spacing: WTokens.Space.sm) {
            Text(title)
                .textCase(.uppercase)
                .wType(WTokens.TypeScale.caption)
                .fontWeight(.bold)
                .tracking(0.6)
                .foregroundStyle(Color.theme(.textSecondary))
                .accessibilityAddTraits(.isHeader)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(WTokens.Space.lg)
        .wCard()
    }
}
