import AppKit
import SwiftUI
import os

/// M1.4 sub-project 3 (docs/superpowers/specs/2026-08-06-chat-prompt-library-and-task-
/// routing-design.md): substitutes Ollama for a non-time-critical query, but ONLY when
/// the picker hasn't been explicitly touched this visit — an explicit pick always wins,
/// even one that happens to equal the account default (see `pickerTouched`'s doc
/// comment on why value-equality alone can't tell those two cases apart). A free
/// function (not a TimelineView method) so it's directly unit-testable without
/// constructing a SwiftUI view.
///
/// Known ceiling: substituting the provider to `.ollama` also changes the *fallback*
/// model on the non-Ollama chain link. `core/ffi`'s `build_chain` only honors
/// `routing.preferred_model` for the Anthropic link when `routing.preferred_provider ==
/// "anthropic"`; when we substitute to Ollama here, `preferred_provider` becomes
/// "ollama", so the Anthropic fallback (used if Ollama is unreachable) silently reverts
/// to the hardcoded `FALLBACK_ANTHROPIC_MODEL` ("claude-haiku-4-5") instead of the
/// user's actual selected model. `RoutingConfig` has no way to express "prefer Ollama,
/// but if it fails fall back to my actual pick" with a single `preferred_model` field
/// keyed to `preferred_provider`. Fixing this needs a `core/ffi` change — out of scope
/// for this pass.
func effectiveAskRoutingProvider(
    query: String,
    pickerProvider: RoutingSettings.Provider,
    pickerModelLabel: String,
    pickerTouched: Bool,
    defaults: (provider: RoutingSettings.Provider, ollamaModel: String, hostedModel: String)
) -> (provider: RoutingSettings.Provider, modelLabel: String) {
    guard !CoreMemoryStore.isTimeCritical(query), !pickerTouched else {
        return (pickerProvider, pickerModelLabel)
    }
    return (.ollama, defaults.ollamaModel)
}

/// FR-CHAT-6: starred conversations first (regardless of age), then most-recently-
/// updated. A free function so it's testable without constructing a SwiftUI view —
/// matches `effectiveAskRoutingProvider`'s own precedent. `core/memory`'s
/// `list_conversations` already returns rows in this order; this exists so the sidebar
/// view can re-sort after a local mutation (e.g. toggling a star) without a full re-fetch.
func sortedForSidebar(_ conversations: [CoreMemoryStore.ConversationSummary]) -> [CoreMemoryStore.ConversationSummary] {
    conversations.sorted { lhs, rhs in
        if lhs.starred != rhs.starred { return lhs.starred }
        return lhs.updatedAtUnixMs > rhs.updatedAtUnixMs
    }
}

/// One item retrieved as context for an Ask-mode turn, capture or transcript — mirrors
/// `ContextItem` on the Rust side, purely for local citation-matching/display (see
/// `citedItems(for:)`), not sent back over FFI.
enum ContextDisplayItem: Identifiable {
    case capture(CoreCaptureItem)
    case transcript(CoreMeetingTranscriptSegment)
    case routineRun(CoreRoutineRunWithName)
    case episode(CoreEpisode)
    case thread(CoreThread)
    case snapshot(CoreMeetingSnapshotHit)
    /// A citation reloaded from a saved conversation.
    case saved(SavedCitation)

    var id: String {
        switch self {
        case .saved(let citation): return "\(citation.item.kind.rawValue)-\(citation.item.id)"
        case .capture(let item): return "capture-\(item.id)"
        case .transcript(let segment): return "transcript-\(segment.id)"
        case .routineRun(let run): return "routine_run-\(run.id)"
        case .episode(let episode): return "episode-\(episode.id)"
        case .thread(let thread): return "thread-\(thread.id)"
        case .snapshot(let hit): return "snapshot-\(hit.id)"
        }
    }

    var matchesRef: (CoreContextItemRef) -> Bool {
        { ref in
            switch self {
            case .capture(let item): return ref.kind == .capture && ref.id == item.id
            case .transcript(let segment): return ref.kind == .transcript && ref.id == segment.id
            case .routineRun(let run): return ref.kind == .routineRun && ref.id == run.id
            case .episode(let episode): return ref.kind == .episode && ref.id == episode.id
            case .thread(let thread): return ref.kind == .thread && ref.id == thread.id
            case .snapshot(let hit): return ref.kind == .snapshot && ref.id == hit.id
            case .saved(let citation): return ref == citation.item
            }
        }
    }

    var displayLabel: String {
        switch self {
        case .capture(let item): return item.appBundleId
        case .transcript(let segment): return "meeting \"\(segment.meetingTitle)\", \(segment.speakerLabel)"
        case .routineRun(let run): return "routine \"\(run.routineName)\""
        case .episode(let episode): return "episode \"\(episode.windowTitle ?? episode.appBundleId)\""
        case .thread(let thread): return "thread: \(thread.subject ?? thread.lastMessageFrom)"
        case .snapshot(let hit): return "slide in meeting \"\(hit.meetingTitle)\""
        case .saved(let citation): return citation.label
        }
    }

    var displayText: String {
        switch self {
        case .capture(let item): return item.text
        case .transcript(let segment): return segment.text
        case .routineRun(let run): return run.outputSummary
        case .episode(let episode): return episode.summary ?? "(summary pending)"
        case .thread(let thread): return thread.summary ?? "(summary pending)"
        case .snapshot(let hit): return hit.text
        case .saved(let citation): return citation.text
        }
    }

    /// Icon for this citation's source chip (routine runs use the skin's routines symbol). Main thread only.
    var systemImage: String {
        switch self {
        case .capture: return "macwindow"
        case .transcript: return "waveform"
        case .routineRun: return SkinStore.shared.selected.icons[.routines]
        case .episode: return "rectangle.stack"
        case .thread: return "envelope"
        case .snapshot: return "photo"
        case .saved(let citation):
            switch citation.item.kind {
            case .capture: return "macwindow"
            case .transcript: return "waveform"
            case .routineRun: return SkinStore.shared.selected.icons[.routines]
            case .episode: return "rectangle.stack"
            case .thread: return "envelope"
            case .snapshot: return "photo"
            }
        }
    }

    /// A slide's image file, shown in its source chip instead of the icon. Saved citations have none.
    var thumbnailPath: String? {
        if case .snapshot(let hit) = self { return hit.filePath }
        return nil
    }

    /// Short, one-line name for this citation's source chip; the full text is its tooltip.
    var chipText: String {
        switch self {
        case .capture(let item): return item.windowTitle.isEmpty ? item.appBundleId : item.windowTitle
        case .transcript(let segment): return "\(segment.meetingTitle) · \(segment.speakerLabel)"
        case .routineRun(let run): return run.routineName
        case .episode(let episode): return episode.windowTitle ?? episode.appBundleId
        case .thread(let thread): return thread.subject ?? thread.lastMessageFrom
        case .snapshot(let hit): return "\(hit.meetingTitle) · slide"
        case .saved(let citation): return citation.label
        }
    }

    /// Tap target for opening this citation's detail window, mirroring the existing
    /// openWindow(id:value:) pattern already used by Timeline rows. nil means the
    /// citation renders as plain (non-button) text -- routine runs have no detail
    /// window anywhere in this app.
    var detailWindowId: String? {
        switch self {
        case .capture: return "capture-detail"
        case .transcript: return "meeting-detail"
        case .routineRun: return nil
        case .episode: return "episode-detail"
        case .thread: return "thread-detail"
        case .snapshot: return "meeting-detail"
        case .saved(let citation): return citation.open.map { "\($0.kind)-detail" }
        }
    }

    /// The Int64 value for openWindow(id: detailWindowId, value:). For .transcript this
    /// is the segment's *meeting* id (there's no per-segment detail window, only
    /// meeting-detail).
    var detailWindowValue: Int64? {
        switch self {
        case .capture(let item): return item.id
        case .transcript(let segment): return segment.meetingId
        case .routineRun: return nil
        case .episode(let episode): return episode.id
        case .thread(let thread): return thread.id
        case .snapshot(let hit): return hit.meetingId
        case .saved(let citation): return citation.open?.id
        }
    }

    /// What a saved conversation stores for this citation: enough to draw its chip and
    /// open it again without the item it came from.
    var saved: SavedCitation {
        if case .saved(let citation) = self { return citation }
        let open = detailWindowId.flatMap { windowId in
            detailWindowValue.map { SavedCitation.Open(kind: String(windowId.dropLast("-detail".count)), id: $0) }
        }
        return SavedCitation(item: ref, label: chipText, text: displayText, open: open)
    }

    var ref: CoreContextItemRef {
        switch self {
        case .capture(let item): return CoreContextItemRef(kind: .capture, id: item.id)
        case .transcript(let segment): return CoreContextItemRef(kind: .transcript, id: segment.id)
        case .routineRun(let run): return CoreContextItemRef(kind: .routineRun, id: run.id)
        case .episode(let episode): return CoreContextItemRef(kind: .episode, id: episode.id)
        case .thread(let thread): return CoreContextItemRef(kind: .thread, id: thread.id)
        case .snapshot(let hit): return CoreContextItemRef(kind: .snapshot, id: hit.id)
        case .saved(let citation): return citation.item
        }
    }
}

/// One cited item as a saved conversation stores it (`conversation_exchange.citations_json`).
/// The same fields as the Windows app's `ContextEntry`, plus the item text for the tooltip. `open.kind`
/// names the detail window: capture, meeting, episode or thread.
struct SavedCitation: Codable, Equatable {
    struct Open: Codable, Equatable {
        let kind: String
        let id: Int64
    }
    let item: CoreContextItemRef
    let label: String
    let text: String
    let open: Open?
}

/// One question/answer exchange in an Ask-mode conversation (FR-CHAT), including the
/// context items retrieved for that specific turn (retrieval is per-turn, not
/// history-aware — see `CoreMemoryStore.askQuestion`'s docs).
struct ChatExchange: Identifiable {
    let id = UUID()
    let query: String
    let result: Result<CoreChatAnswer, CoreLLMError>
    let contextItems: [ContextDisplayItem]
    /// The model that answered, shown under the answer. Nil for exchanges saved before
    /// the model was stored.
    var modelLabel: String? = nil
}

/// #74: everything `handleStreamingAskPhaseChange`/`finalizeStreamingAnswer` need once
/// `streamingAsk`'s run reaches a terminal state — captured at `askStreaming`'s call
/// site since `streamingAsk` itself only knows about the raw prompt/text stream, not
/// this turn's prepare_id, citation context, or conversation-persistence bookkeeping.
struct PendingStreamAsk {
    let prepareId: String
    let query: String
    let contextItems: [ContextDisplayItem]
    let activeConversationId: Int64?
    let isFirstMessageInConversation: Bool
    let ollamaModel: String
    /// The model label the answer itself streams from — equals `ollamaModel` on the
    /// Ollama route, the picked Gemini model on the Google route (#83). Recorded via
    /// `finalizeAsk` for the usage log; `ollamaModel` stays the title-gen model.
    let answerModel: String
    let titleGenEffective: (provider: RoutingSettings.Provider, modelLabel: String)
    /// Runs `askBlocking` with this same turn's already-captured arguments — used only
    /// when the stream reaches `.failed` with zero output ever emitted (see
    /// `handleStreamingAskPhaseChange`).
    let fallbackToBlocking: () -> Void
}

/// FR-UI-2: a single row in the merged Recent/Keyword/Semantic list — a screen-text
/// capture, a persisted meeting record, or (Keyword/Semantic only) a matched transcript
/// segment. Recent mode merges captures + meetings, sorted by timestamp; Keyword/Semantic
/// modes merge captures + transcript segments, grouped (captures first) rather than
/// interleaved by relevance score, since FTS5/vec0 rank/distance values aren't
/// comparable across the two underlying tables.
enum TimelineEntry: Identifiable {
    case capture(CoreCaptureItem)
    case meeting(CoreMeeting)
    case transcriptSegment(CoreMeetingTranscriptSegment)
    case snapshot(CoreMeetingSnapshotHit)

    var id: String {
        switch self {
        case .capture(let item): return "capture-\(item.id)"
        case .meeting(let meeting): return "meeting-\(meeting.id)"
        case .transcriptSegment(let segment): return "transcript-\(segment.id)"
        case .snapshot(let hit): return "snapshot-\(hit.id)"
        }
    }

    var sortTimestamp: Int64 {
        switch self {
        case .capture(let item): return item.tsUnixMs
        case .meeting(let meeting): return meeting.startUnixMs
        case .transcriptSegment, .snapshot: return 0
        }
    }
}

/// A titled run of Library rows: a day ("Today", "Yesterday", a date) in Recent, or a
/// result kind in Keyword and Semantic.
struct LibrarySection<Item>: Identifiable {
    let label: String
    let items: [Item]
    var id: String { label }
}

enum LibraryDayGrouping {
    /// Groups items under day labels in the order each day first appears, so a
    /// newest-first list stays newest-first.
    static func groupedByDay<Item>(
        _ items: [Item], timestamp: (Item) -> Int64, now: Date = Date(), calendar: Calendar = .current
    ) -> [LibrarySection<Item>] {
        var order: [String] = []
        var buckets: [String: [Item]] = [:]
        for item in items {
            let label = dayLabel(unixMs: timestamp(item), now: now, calendar: calendar)
            if buckets[label] == nil { order.append(label) }
            buckets[label, default: []].append(item)
        }
        return order.map { LibrarySection(label: $0, items: buckets[$0] ?? []) }
    }

    static func dayLabel(unixMs: Int64, now: Date, calendar: Calendar) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(unixMs) / 1000)
        let days = calendar.dateComponents(
            [.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)
        ).day ?? 0
        if days == 0 { return "Today" }
        if days == 1 { return "Yesterday" }
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        let sameYear = calendar.isDate(date, equalTo: now, toGranularity: .year)
        formatter.setLocalizedDateFormatFromTemplate(sameYear ? "EEEEMMMd" : "yMMMd")
        return formatter.string(from: date)
    }
}

/// FR-MEM-1/2: browse the captured timeline, or search it (keyword/semantic), backed
/// directly by the Rust core (NFR-PRT-1) via CoreBridge.swift.
struct TimelineView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) var openWindow

    static let log = Logger(subsystem: "com.mpointer.kestrel", category: "timeline")


    @State var query: String = ""
    @State var mode: SearchMode = .recent
    @FocusState var searchFieldFocused: Bool
    @State var entries: [TimelineEntry] = []
    @State var episodes: [CoreEpisode] = []
    @State var isLoadingEntries = false
    @State var digestError: String?
    // Task 10: no @AppStorage-backed filter precedent exists in this view (the one
    // other list filter, `appState.showScreenCaptures`, is a UserDefaults-backed
    // AppState property with its Toggle living in SettingsView, not here) -- plain
    // @State matches this view's own pattern for everything else.
    @State var hiddenEpisodeIds: Set<Int64> = []
    @State var showHiddenEpisodes = false
    @State var selectedProvider: RoutingSettings.Provider = .ollama
    @State var ollamaModel: String = "gemma4:12b"
    @State var selectedHostedModel: String = "claude-haiku-4-5"
    // Distinguishes "picker still shows the account default because the user never
    // touched it" from "the user explicitly picked the account default's own value" --
    // effectiveAskRoutingProvider's value-equality check alone can't tell those apart,
    // which silently substituted Ollama even for an explicit pick that matched the
    // default. Only flips true via the Binding wrapper passed to ProviderModelPicker
    // (a genuine UI interaction); .onAppear's programmatic reseed resets it to false.
    @State var providerPickerTouched = false
    @State var exchanges: [ChatExchange] = []
    @State var activeConversationId: Int64?
    @State var conversationList: [CoreMemoryStore.ConversationSummary] = []
    @State var showConversationSidebar = true
    @State var renamingConversation: CoreMemoryStore.ConversationSummary?
    @State var renameText: String = ""
    @State var isAsking = false
    @State var inFlightModelLabel: String = ""
    // #74: drives askIfNeeded's Ollama-route streaming path (askStreaming/
    // handleStreamingAskPhaseChange) — see PendingStreamAsk's doc comment.
    @StateObject var streamingAsk = StreamingAskViewModel()
    @State var pendingStreamAsk: PendingStreamAsk?
    @State var spendTodayCents: Int64?
    @State var digest: CoreDailyDigest?
    @State var isGeneratingDigest = false
    @State var showModelSetup = false
    @State var meetingPendingDeletion: CoreMeeting?
    @State var headphonesWarningDismissed = false

    enum SearchMode: String, CaseIterable, Identifiable {
        case recent = "Recent"
        case episodes = "Episodes"
        case keyword = "Keyword"
        case semantic = "Semantic"
        case ask = "Ask"
        case digest = "Digest"
        var id: String { rawValue }
    }

    var isBusy: Bool { isAsking || isGeneratingDigest }

    /// Acts on ⌘N and ⌘F from the menu bar (see `AppState.requestNewConversation` and
    /// `requestLibrarySearch`). The search field appears only in the modes that have one,
    /// so focus is set on the next run-loop turn, after the mode switch renders it.
    func handleMenuRequests() {
        if appState.pendingNewConversation {
            appState.pendingNewConversation = false
            let prompt = appState.pendingAskPrompt
            appState.pendingAskPrompt = nil
            if !isBusy {
                mode = .ask
                exchanges = []
                activeConversationId = nil
                // From Today's composer: send it as the first message.
                if let prompt, !prompt.isEmpty { query = prompt; refresh() }
            }
        } else if appState.pendingLibrarySearch {
            appState.pendingLibrarySearch = false
            if ![.keyword, .semantic, .ask].contains(mode) { mode = .keyword }
        } else {
            return
        }
        DispatchQueue.main.async { searchFieldFocused = true }
    }

    static let storeUnavailableMessage = "Whimbrel's local database didn't open. Try restarting Whimbrel."

    /// Recent reads newest-first under day headings. Keyword and Semantic keep their
    /// relevance order, with screen hits and transcript hits under separate labels.
    var entrySections: [LibrarySection<TimelineEntry>] {
        switch mode {
        case .keyword, .semantic:
            let isTranscript: (TimelineEntry) -> Bool = {
                if case .transcriptSegment = $0 { return true } else { return false }
            }
            let isSnapshot: (TimelineEntry) -> Bool = {
                if case .snapshot = $0 { return true } else { return false }
            }
            return [
                LibrarySection(label: "Screen activity", items: entries.filter { !isTranscript($0) && !isSnapshot($0) }),
                LibrarySection(label: "Transcript matches", items: entries.filter(isTranscript)),
                LibrarySection(label: "Slide matches", items: entries.filter(isSnapshot)),
            ].filter { !$0.items.isEmpty }
        default:
            return LibraryDayGrouping.groupedByDay(entries, timestamp: \.sortTimestamp)
        }
    }

    var emptyListMessage: String {
        if appState.coreStore == nil { return Self.storeUnavailableMessage }
        switch mode {
        case .keyword, .semantic:
            return query.isEmpty ? "Type something to search for." : "No matches for “\(query)”."
        default:
            return appState.showScreenCaptures
                ? "Nothing yet. Meetings and screen activity appear here as you work."
                : "No meetings yet. Turn on “Show screen captures in Library” in Settings → Capture to see screen activity here too."
        }
    }

    /// Where an Ask or Digest request will actually go: local-only mode forces Ollama
    /// whatever the picker says.
    var answeringProviderPhrase: String {
        let provider: RoutingSettings.Provider = appState.routing.localOnlyMode ? .ollama : selectedProvider
        return provider == .ollama ? "on this Mac with Ollama" : "by \(provider.displayName)"
    }

    var currentModelLabel: String {
        selectedProvider == .ollama ? ollamaModel : selectedHostedModel
    }

    var body: some View {
        HStack(spacing: 0) {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Library")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)

            captureStatusHeader

            Picker("Mode", selection: $mode) {
                ForEach(SearchMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .disabled(isBusy)
            .onChange(of: mode) { _, newMode in
                refresh()
                if newMode == .ask && showConversationSidebar { refreshConversationList() }
            }
            .onChange(of: appState.showScreenCaptures) { _, _ in if mode == .recent { refresh() } }

            if mode == .ask || mode == .digest {
                ProviderModelPicker(
                    selectedProvider: Binding(
                        get: { selectedProvider },
                        set: { selectedProvider = $0; providerPickerTouched = true }
                    ),
                    ollamaModel: Binding(
                        get: { ollamaModel },
                        set: { ollamaModel = $0; providerPickerTouched = true }
                    ),
                    selectedHostedModel: Binding(
                        get: { selectedHostedModel },
                        set: { selectedHostedModel = $0; providerPickerTouched = true }
                    )
                )
            }

            if mode != .recent && mode != .digest && mode != .episodes {
                if mode == .ask {
                    usageTodayLine
                }
                if mode == .ask && exchanges.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: Spacing.xs) {
                            ForEach(CoreMemoryStore.starterPrompts()) { prompt in
                                Button(prompt.title) { sendLibraryPrompt(prompt) }
                                    .buttonStyle(.whimbrelSecondary)
                                    .disabled(isBusy)
                            }
                        }
                    }
                }
                if mode == .ask {
                    HStack {
                        Button(showConversationSidebar ? "Hide Conversations" : "Show Conversations") {
                            showConversationSidebar.toggle()
                            if showConversationSidebar { refreshConversationList() }
                        }
                        .buttonStyle(.whimbrelSecondary)
                    }
                }
                HStack {
                    TextField(mode == .ask ? "Ask a question…" : "Search…", text: $query)
                        .textFieldStyle(.roundedBorder)
                        .focused($searchFieldFocused)
                        .onSubmit { refresh() }
                    Button(mode == .ask ? "Ask" : "Search") { refresh() }
                        .disabled(isBusy)
                        .buttonStyle(.whimbrelSecondary)
                    if mode == .ask && !exchanges.isEmpty {
                        Button("New Conversation") {
                            exchanges = []
                            activeConversationId = nil
                        }
                            .disabled(isBusy)
                            .buttonStyle(.whimbrelSecondary)
                    }
                }
            }

            if mode == .digest {
                Button("Generate Today's Digest") { generateDigestIfNeeded() }
                    .disabled(isBusy)
                    .buttonStyle(.whimbrelPrimary)
            }

            if mode == .ask {
                HStack(alignment: .top, spacing: Spacing.md) {
                    if showConversationSidebar {
                        conversationSidebar
                    }
                    askModeBody
                }
            } else if mode == .digest {
                digestModeBody
            } else if mode == .episodes {
                episodesModeBody
            } else if entries.isEmpty && isLoadingEntries {
                ProgressView()
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
            } else if entries.isEmpty {
                Text(emptyListMessage)
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: Spacing.sm) {
                        ForEach(entrySections) { section in
                            WhimbrelSectionLabel(text: section.label)
                                .padding(.top, Spacing.sm)
                            ForEach(section.items) { entry in
                                entryRow(for: entry)
                            }
                        }
                    }
                }
            }
        }
        .padding(Spacing.xl)
        .frame(minWidth: 520, idealWidth: 600, minHeight: 480, idealHeight: 560, maxHeight: .infinity, alignment: .top)
        .background(Color.whimbrelBackgroundPage)
        .onChange(of: appState.pendingLibrarySearch) { _, pending in if pending { handleMenuRequests() } }
        .onChange(of: appState.pendingNewConversation) { _, pending in if pending { handleMenuRequests() } }
        .onAppear {
            refresh()
            refreshSpendToday()
            if !ModelDownloadService.allModelsPresent {
                // Deferred to the next run-loop turn: setting this @State synchronously
                // inside .onAppear can silently fail to present the sheet on macOS
                // because the view isn't yet fully attached to its window hierarchy at
                // that exact point in the lifecycle. Dispatching async (after this
                // update transaction completes) reliably presents it.
                DispatchQueue.main.async {
                    showModelSetup = true
                }
            }
            let defaults = appState.routing.defaultSelection()
            selectedProvider = defaults.provider
            ollamaModel = defaults.ollamaModel
            if defaults.provider != .ollama {
                selectedHostedModel = defaults.hostedModel
            }
            providerPickerTouched = false
            // After the picker is seeded: a prompt from Today's composer is sent from here.
            handleMenuRequests()
            NSApp.activate(ignoringOtherApps: true)
            appState.openTimeline = { openWindow(id: "timeline") }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { notification in
            guard let window = notification.object as? NSWindow, window.title == "Whimbrel" else { return }
            // Task 10: Episodes mode needs the same re-fetch-on-refocus as Recent --
            // hiding/unhiding an episode happens in the detail window, which closes
            // (onHidden -> dismiss()) and hands focus back here. Without this, the
            // just-hidden episode would keep showing (and hiddenEpisodeIds would go
            // stale) until the user manually flipped modes.
            if mode == .recent || mode == .episodes { refresh() }
        }
        .sheet(isPresented: $showModelSetup) {
            ModelSetupView()
        }
        .alert("Forget this meeting?", isPresented: Binding(
            get: { meetingPendingDeletion != nil },
            set: { if !$0 { meetingPendingDeletion = nil } }
        )) {
            Button("Forget meeting", role: .destructive) {
                if let meeting = meetingPendingDeletion {
                    deleteMeeting(meeting)
                }
                meetingPendingDeletion = nil
            }
            Button("Cancel", role: .cancel) {
                meetingPendingDeletion = nil
            }
        } message: {
            Text("This permanently removes the meeting note, action items, transcript, local audio file, and search/index entries for this meeting. This does not affect calendar events.")
        }

        Divider().overlay(Color.whimbrelBorder)
        AtAGlancePanelView()
        }
    }

    static let askScrollBottomAnchorID = "ask-scroll-bottom-anchor"

    func refresh() {
        guard let store = appState.coreStore else {
            entries = []
            return
        }
        switch mode {
        case .recent:
            let includeScreenCaptures = appState.showScreenCaptures
            isLoadingEntries = true
            CoreMemoryStore.accessQueue.async {
                let captureEntries = includeScreenCaptures
                    ? store.recent(limit: 50).map(TimelineEntry.capture)
                    : []
                let rawMeetings = store.recentMeetingsGrouped(limit: 20)
                Self.log.log("refresh(.recent): recentMeetings returned \(rawMeetings.count, privacy: .public) rows")
                let meetingEntries = rawMeetings.map(TimelineEntry.meeting)
                let sorted = (captureEntries + meetingEntries).sorted { $0.sortTimestamp > $1.sortTimestamp }
                DispatchQueue.main.async {
                    entries = sorted
                    isLoadingEntries = false
                }
            }
        case .keyword:
            guard !query.isEmpty else {
                entries = []
                return
            }
            let capturedQuery = query
            isLoadingEntries = true
            CoreMemoryStore.accessQueue.async {
                let captureResults = store.searchKeyword(capturedQuery, limit: 50).map(TimelineEntry.capture)
                let transcriptResults = store.searchTranscriptKeyword(capturedQuery, limit: 50).map(TimelineEntry.transcriptSegment)
                let snapshotResults = store.searchSnapshotKeyword(capturedQuery, limit: 50).map(TimelineEntry.snapshot)
                DispatchQueue.main.async {
                    entries = captureResults + transcriptResults + snapshotResults
                    isLoadingEntries = false
                }
            }
        case .semantic:
            guard !query.isEmpty else {
                entries = []
                return
            }
            let capturedQuery = query
            isLoadingEntries = true
            CoreMemoryStore.accessQueue.async {
                let captureResults = store.searchSemantic(capturedQuery, limit: 50).map(TimelineEntry.capture)
                let transcriptResults = store.searchTranscriptSemantic(capturedQuery, limit: 50).map(TimelineEntry.transcriptSegment)
                let snapshotResults = store.searchSnapshotSemantic(capturedQuery, limit: 50).map(TimelineEntry.snapshot)
                DispatchQueue.main.async {
                    entries = captureResults + transcriptResults + snapshotResults
                    isLoadingEntries = false
                }
            }
        case .ask:
            askIfNeeded(store: store)
        case .digest:
            digest = nil
        case .episodes:
            let deviceId = appState.deviceId
            isLoadingEntries = true
            CoreMemoryStore.accessQueue.async {
                let result = store.recentEpisodes(limit: 50) ?? []
                // The `deviceId:` argument is ignored by the core; hidden state syncs
                // across devices by episode identity -- same as EpisodeDetailView's
                // identical lookup, see its comment.
                let hidden = Set(store.listHiddenEpisodeIds(deviceId: deviceId))
                DispatchQueue.main.async {
                    episodes = result
                    hiddenEpisodeIds = hidden
                    isLoadingEntries = false
                }
            }
        }
    }

    func deleteMeeting(_ meeting: CoreMeeting) {
        guard let store = appState.coreStore else { return }
        guard meeting.status != .recording else { return }
        MeetingLifecycle.forgetMeeting(
            store: store,
            meetingId: meeting.id
        ) { result in
            if case .failed = result {
                Self.log.error("Failed to delete meeting id \(meeting.id, privacy: .public)")
            }
            refresh()
        }
    }

    /// A query guaranteed to classify as non-time-critical by `CoreMemoryStore.isTimeCritical`
    /// (contains one of its keywords) — used only to force `effectiveAskRoutingProvider`'s
    /// non-time-critical substitution path for calls, like title-gen, that are never
    /// time-critical by definition, without needing a second classifier entry point.
    /// Not `private` — `TimelineViewRoutingTests` asserts against this exact constant.
    static let nonTimeCriticalSentinelQuery = "summarize"

}
