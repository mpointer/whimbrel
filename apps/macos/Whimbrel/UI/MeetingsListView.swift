import AppKit
import OSLog
import SwiftUI
import UniformTypeIdentifiers

/// What an import did, for one line under the Meetings list.
enum ImportSummary {
    static func text(_ o: CoreImportOutcome, limit: Int) -> String {
        var parts: [String] = []
        if !o.meetingIds.isEmpty { parts.append(String(localized: "Imported \(o.meetingIds.count) meetings.")) }
        if o.notes > 0 { parts.append(String(localized: "Imported \(o.notes) notes.")) }
        if o.skipped > 0 { parts.append(String(localized: "Skipped \(o.skipped) already here.")) }
        if o.overlapping > 0 { parts.append(String(localized: "Skipped \(o.overlapping) you recorded here.")) }
        if !o.failed.isEmpty { parts.append(String(localized: "Couldn't read \(o.failed.count) files.")) }
        if o.meetingIds.count > limit { parts.append(String(localized: "Summarizing the newest \(limit).")) }
        return parts.isEmpty ? String(localized: "Nothing to import.") : parts.joined(separator: " ")
    }
}

/// A reload counter the store queue can read without hopping to the main thread.
final class LoadGeneration {
    private let lock = NSLock()
    private var value = 0
    var current: Int { lock.lock(); defer { lock.unlock() }; return value }
    func bump() -> Int { lock.lock(); defer { lock.unlock() }; value += 1; return value }
}

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
    @State private var selectedTopic: TopicRef?
    /// Chip lookups for the loaded meetings, filled by one batch after the list lands. Rows render
    /// without them first; a meeting missing here simply has no Topic or Prep chip.
    @State private var topicByMeetingId: [Int64: TopicRef] = [:]
    @State private var prepKeyByMeetingId: [Int64: String] = [:]
    /// Bumped on every reload; a batch that starts or finishes after a newer reload began is dropped.
    @State private var loadGeneration = LoadGeneration()
    /// The sorted meeting ids the last applied chip batch covered; nil before the first.
    @State private var lastBatchedIds: [Int64]?
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

    @State private var importMessage: String?

    private func importTranscripts() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.allowedContentTypes = ["vtt", "srt", "txt", "csv"].compactMap { UTType(filenameExtension: $0) }
        panel.message = String(localized: "Choose transcripts (.vtt, .srt, .txt) or a Granola CSV")
        guard panel.runModal() == .OK else { return }
        let urls = panel.urls, deviceId = appState.deviceId
        appState.runImport({ $0.importFiles(urls, deviceId: deviceId) }, done: finishImport)
    }

    private func importMarkdownFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.message = String(localized: "Choose a folder of Markdown notes")
        guard panel.runModal() == .OK, let folder = panel.url else { return }
        let deviceId = appState.deviceId
        appState.runImport({ $0.importMarkdownFolder(folder, deviceId: deviceId) }, done: finishImport)
    }

    private func finishImport(_ outcome: CoreImportOutcome?) {
        importMessage = outcome.map { ImportSummary.text($0, limit: AppState.importSummaryLimit) } ?? String(localized: "Import failed.")
        refresh()
    }

    /// One card model per meeting, in order. A meeting absent from `topics` / `preps` gets no such chip.
    static func rowModels(meetings: [CoreMeeting], topics: [Int64: TopicRef], preps: [Int64: String], now: Date) -> [MeetingCardModel] {
        meetings.map { MeetingCardModel.forRecorded($0, prepKey: preps[$0.id], topic: topics[$0.id], now: now) }
    }

    /// Why a reload ran. Only `.appeared` (the view or window came back) may skip the chip batch.
    enum ChipBatchTrigger { case appeared, meetingsChanged }

    /// Whether the chip lookups need re-running: the first time, after any change to the meetings
    /// (a processed meeting gains its topic under the same id), or when the set of ids differs.
    static func needsChipBatch(ids: [Int64], lastBatched: [Int64]?, trigger: ChipBatchTrigger) -> Bool {
        guard let lastBatched, trigger == .appeared else { return true }
        return ids.sorted() != lastBatched.sorted()
    }

    /// The key `showPrepBrief` looks up. `eventId` already holds the stored key ("event@startMs", or
    /// the bare event id on older rows); `meetingPrep(key:)` finds both, so pass it through unchanged.
    static func prepKey(for prep: CoreMeetingPrep) -> String { prep.eventId }

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
            refresh(.appeared)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { notification in
            guard let window = notification.object as? NSWindow, window.title == "Whimbrel" else { return }
            refresh(.appeared)
        }
        // A meeting forgotten on another device was deleted here by sync.
        .onReceive(NotificationCenter.default.publisher(for: MeetingLifecycle.meetingsDidChange)) { _ in refresh() }
        .onChange(of: appState.processingMeetingId) { old, new in
            guard old != nil, new == nil else { return }
            refresh()
        }
        .onChange(of: selectedMeetingId) { _, newValue in
            guard let newValue else {
                loadedMeetingId = nil
                selectedDraft = nil
                selectedActionItems = []
                selectedTopic = nil
                return
            }
            loadDetail(meetingId: newValue)
        }
    }

    private var meetingList: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.lg, pinnedViews: []) {
                HStack {
                    if let importMessage {
                        Text(importMessage)
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                    }
                    Spacer()
                    Menu("Import") {
                        Button("Transcripts…", action: importTranscripts)
                        Button("Markdown Folder…", action: importMarkdownFolder)
                    }
                    .fixedSize()
                }
                .padding(.horizontal, Spacing.md)
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
                    WhimbrelEmptyState(systemImage: SkinStore.shared.selected.icons[.meetings], title: searchText.isEmpty ? String(localized: "No meetings yet") : String(localized: "No matches"), message: searchText.isEmpty ? String(localized: "Recorded meetings appear here. You can also bring in transcripts from Granola, Otter or files.") : String(localized: "Try a different search."), actionTitle: searchText.isEmpty ? String(localized: "Import Transcripts…") : nil, action: searchText.isEmpty ? importTranscripts : nil)
                }
            }
            .padding(.vertical, Spacing.lg)
            .frame(maxWidth: 760)
            .frame(maxWidth: .infinity)
        }
        .searchable(text: $searchText, prompt: "Search meetings")
    }

    private func meetingRow(_ meeting: CoreMeeting) -> some View {
        let model = Self.rowModels(meetings: [meeting], topics: topicByMeetingId, preps: prepKeyByMeetingId, now: Date())[0]
        return MeetingListRow(
            model: model,
            onOpen: { selectedMeetingId = meeting.id },
            onChip: { chip in
                switch chip {
                case let .prep(key): appState.showPrepBrief(key: key)
                case let .topic(id, _): appState.openTopic(id)
                case .summary, .actionItems: selectedMeetingId = meeting.id
                }
            }
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
                MeetingDetailContentView(meeting: meeting, initialDraft: selectedDraft, initialActionItems: selectedActionItems, topic: selectedTopic)
            }
            .id(id)
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func refresh(_ trigger: ChipBatchTrigger = .meetingsChanged) {
        guard let store = appState.coreStore else {
            Self.log.error("refresh: appState.coreStore is nil, cannot load meetings")
            hasLoaded = true
            return
        }
        Self.log.log("refresh: starting recentMeetings(limit: 200) fetch")
        let generation = loadGeneration.bump()
        let counter = loadGeneration
        let lastBatched = lastBatchedIds
        CoreMemoryStore.accessQueue.async {
            let result = store.recentMeetingsGrouped(limit: 200)
            DispatchQueue.main.async {
                Self.log.log("refresh: assigning \(result.count, privacy: .public) meetings to state")
                meetings = result
                hasLoaded = true
            }
            // One batch for the chips, on the same queue turn; rows are already showing. Skip it when a
            // newer reload is already queued, or when the same meetings were batched last time.
            guard generation == counter.current else { return }
            let ids = result.map(\.id).sorted()
            guard Self.needsChipBatch(ids: ids, lastBatched: lastBatched, trigger: trigger) else { return }
            var topics: [Int64: TopicRef] = [:]
            var preps: [Int64: String] = [:]
            for meeting in result {
                if let topic = store.topicForMeeting(meetingId: meeting.id) { topics[meeting.id] = topic }
                if let prep = store.meetingPrep(meetingId: meeting.id), prep.brief != nil {
                    preps[meeting.id] = Self.prepKey(for: prep)
                }
            }
            DispatchQueue.main.async {
                guard generation == counter.current else { return }
                topicByMeetingId = topics
                prepKeyByMeetingId = preps
                lastBatchedIds = ids
            }
        }
    }

    private func loadDetail(meetingId: Int64) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let draft = store.getMeetingNoteDraft(meetingId: meetingId)
            let items = store.listMeetingActionItems(meetingId: meetingId)
            let topic = store.topicForMeeting(meetingId: meetingId)
            DispatchQueue.main.async {
                guard selectedMeetingId == meetingId else { return }
                selectedDraft = draft
                selectedActionItems = items
                selectedTopic = topic
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
