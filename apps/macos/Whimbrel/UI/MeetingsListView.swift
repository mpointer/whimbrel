import AppKit
import OSLog
import SwiftUI

/// A calm searchable list of meetings, opening `MeetingDetailContentView` (After
/// state) on selection. Preserves the existing meeting-detail window (opened from
/// Timeline via `openWindow(id: "meeting-detail", value:)`) untouched — this is a new,
/// additional entry point, not a replacement for it. `MeetingDetailContentView` needs
/// its meeting's `CoreMeetingNoteDraft`/`[CoreMeetingActionItem]` (same as the
/// meeting-detail window's own `loadMeetingIfNeeded`), fetched here on selection change
/// via `CoreMemoryStore.accessQueue.async` + `DispatchQueue.main.async` — never
/// synchronously from the view body.
struct MeetingsListView: View {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "meetings-list-diag")
    @EnvironmentObject var appState: AppState
    @State private var meetings: [CoreMeeting] = []
    /// False until the first fetch returns, so a slow fetch (it queues behind a content sync)
    /// shows a spinner instead of "No meetings yet".
    @State private var hasLoaded = false
    @State private var searchText: String = ""
    @State private var selectedMeetingId: Int64?
    @State private var selectedDraft: CoreMeetingNoteDraft?
    @State private var selectedActionItems: [CoreMeetingActionItem] = []
    /// The meeting id `selectedDraft`/`selectedActionItems` actually belong to — used to
    /// gate `MeetingDetailContentView`'s construction until its real data has loaded, and
    /// to force a fresh instance per meeting (`.id(selectedMeetingId)` below).
    /// `MeetingDetailContentView` copies its `initial*` params into `@State` in `init`,
    /// which only takes effect the first time a given view identity is constructed — so
    /// without this, either the view would seed itself with empty content and never pick
    /// up the async fetch, or (worse, without `.id`) switching from meeting A to meeting B
    /// would keep A's edited-but-unsaved `@State` alive under B's title, and a subsequent
    /// edit would autosave into B's draft row.
    @State private var loadedMeetingId: Int64?

    private var filteredMeetings: [CoreMeeting] {
        guard !searchText.isEmpty else { return meetings }
        return meetings.filter { $0.title.localizedCaseInsensitiveContains(searchText) }
    }

    private var groups: [MeetingDayGroup] {
        MeetingDayGrouping.groups(
            filteredMeetings.map { (id: $0.id, startUnixMs: $0.startUnixMs) },
            now: Date(), calendar: .current, locale: .current
        )
    }

    var body: some View {
        Group {
            if let selectedMeetingId, let meeting = meetings.first(where: { $0.id == selectedMeetingId }) {
                meetingPage(meeting, id: selectedMeetingId)
            } else {
                meetingList
            }
        }
        .onAppear {
            Self.log.log("onAppear fired, appState.coreStore is \(appState.coreStore == nil ? "nil" : "non-nil", privacy: .public)")
            refresh()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { notification in
            guard let window = notification.object as? NSWindow, window.title == "Whimbrel" else { return }
            refresh()
        }
        .onChange(of: appState.processingMeetingId) { old, new in
            guard old != nil, new == nil else { return }
            refresh()
        }
        .onChange(of: selectedMeetingId) { _, newValue in
            guard let newValue else {
                loadedMeetingId = nil
                selectedDraft = nil
                selectedActionItems = []
                return
            }
            loadDetail(meetingId: newValue)
        }
    }

    private var meetingList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.lg, pinnedViews: []) {
                ForEach(groups, id: \.title) { group in
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Text(group.title)
                            .font(.whimbrelBody(.callout).bold())
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .padding(.horizontal, Spacing.md)
                        ForEach(group.ids, id: \.self) { id in
                            if let meeting = meetings.first(where: { $0.id == id }) {
                                meetingRow(meeting)
                            }
                        }
                    }
                }
                if groups.isEmpty, !hasLoaded {
                    ProgressView().frame(maxWidth: .infinity, minHeight: 240)
                } else if groups.isEmpty {
                    WhimbrelEmptyState(systemImage: SkinStore.shared.selected.icons[.meetings], title: searchText.isEmpty ? "No meetings yet" : "No matches", message: searchText.isEmpty ? "Recorded meetings appear here." : "Try a different search.")
                }
            }
            .padding(.vertical, Spacing.lg)
            .frame(maxWidth: 760)
            .frame(maxWidth: .infinity)
        }
        .searchable(text: $searchText, prompt: "Search meetings")
    }

    private func meetingRow(_ meeting: CoreMeeting) -> some View {
        MeetingRowView(
            title: meeting.title, subtitle: MeetingRowText.subtitle(for: meeting),
            time: MeetingRowText.time(meeting.startUnixMs), action: { selectedMeetingId = meeting.id }
        )
    }

    @ViewBuilder
    private func meetingPage(_ meeting: CoreMeeting, id: Int64) -> some View {
        if loadedMeetingId == id {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Button { selectedMeetingId = nil } label: { Label("Meetings", systemImage: "chevron.left") }
                        .buttonStyle(.whimbrelQuiet)
                        .keyboardShortcut("[", modifiers: .command)
                    Spacer()
                    MeetingOptionsMenu(meeting: meeting, appState: appState, onForgotten: {
                        self.selectedMeetingId = nil
                        refresh()
                    })
                }
                .padding([.horizontal, .top], Spacing.md)
                if let copies = meeting.alsoRecorded, !copies.isEmpty {
                    AlsoRecordedRow(copies: copies, ownDeviceId: appState.deviceId).padding(.horizontal, Spacing.md)
                }
                MeetingDetailContentView(meeting: meeting, initialDraft: selectedDraft, initialActionItems: selectedActionItems)
            }
            .id(id)
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func refresh() {
        guard let store = appState.coreStore else {
            Self.log.error("refresh: appState.coreStore is nil, cannot load meetings")
            hasLoaded = true
            return
        }
        Self.log.log("refresh: starting recentMeetings(limit: 200) fetch")
        CoreMemoryStore.accessQueue.async {
            let result = store.recentMeetingsGrouped(limit: 200)
            DispatchQueue.main.async {
                Self.log.log("refresh: assigning \(result.count, privacy: .public) meetings to state")
                meetings = result
                hasLoaded = true
            }
        }
    }

    private func loadDetail(meetingId: Int64) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let draft = store.getMeetingNoteDraft(meetingId: meetingId)
            let items = store.listMeetingActionItems(meetingId: meetingId)
            DispatchQueue.main.async {
                guard selectedMeetingId == meetingId else { return }
                selectedDraft = draft
                selectedActionItems = items
                loadedMeetingId = meetingId
            }
        }
    }

    private func formattedDate(_ unixMs: Int64) -> String {
        let date = Date(timeIntervalSince1970: Double(unixMs) / 1000)
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}

/// "Also recorded on …" for a call recorded more than once; each opens that copy.
struct AlsoRecordedRow: View {
    let copies: [CoreMeetingCopy]
    let ownDeviceId: String
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        HStack(spacing: Spacing.xs) {
            Text("Also recorded on").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            ForEach(copies) { copy in
                Button(copy.deviceId == ownDeviceId ? "this Mac" : "device \(copy.deviceId.prefix(6))") {
                    openWindow(id: "meeting-detail", value: copy.id)
                }
                .buttonStyle(.link)
                .font(.whimbrelBody(.caption))
                .help(copy.title)
            }
        }
    }
}
