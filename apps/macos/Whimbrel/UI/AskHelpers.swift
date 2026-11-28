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

/// Ask copy for a question that core answers on this Mac because it involves a client excluded from hosted
/// models. Generic on purpose: it never says which client.
enum AskLocalRouteCopy {
    /// While the question is in flight.
    static let inFlight = String(localized: "Answering on this Mac: this question involves a client excluded from hosted models…")
    /// Under the finished answer.
    static let answered = String(localized: "Answered locally: this question involves a client excluded from hosted models.")

    /// Before the check ran: no model named, since the question may be answered on this Mac.
    static let askingPlain = String(localized: "Asking…")
    /// While the question waits for its turn on the model lane (other work holds it).
    static let waiting = String(localized: "Waiting for background work…")
    /// How long a question may wait before the spinner says so (a normal turn starts at once).
    static let waitingShownAfter: TimeInterval = 1

    /// The spinner text: the local note when the question is routed locally, else the wait note while
    /// it is queued, else the picked model.
    static func spinnerTitle(routedLocally: Bool, waitingForLane: Bool = false, modelConfirmed: Bool, modelLabel: String) -> String {
        if routedLocally { return inFlight }
        if waitingForLane { return waiting }
        // The model's name only once the question is known not to be routed to this Mac.
        return modelConfirmed ? String(localized: "Asking \(modelLabel)…") : askingPlain
    }
}

/// The label for a finished blocking answer: the model core says answered, else the one asked for.
func answeredModelLabel(requested: String, answeredBy: CoreAnsweredBy?) -> String {
    guard let model = answeredBy?.model.trimmingCharacters(in: .whitespacesAndNewlines), !model.isEmpty else { return requested }
    return model
}

/// Whether to show "Answered locally" under an answer: the question named an excluded client and core
/// says it answered on this Mac (not refused, not a hosted answer).
func answeredLocallyForExclusion(namesExcludedClient: Bool, outcome: CoreExclusionOutcome?) -> Bool {
    namesExcludedClient && outcome == .localOnly
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
        case .routineRun(let run): return run.displaySummary
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
            case .mcpItem: return "tray.and.arrow.down"
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

/// Text and constants the Ask and Library code share.
enum AskShared {
    static var storeUnavailableMessage: String { String(localized: "Whimbrel's local database didn't open. Try restarting Whimbrel.") }

    /// A query guaranteed to classify as non-time-critical by `CoreMemoryStore.isTimeCritical`
    /// (contains one of its keywords) — used only to force `effectiveAskRoutingProvider`'s
    /// non-time-critical substitution path for calls, like title-gen, that are never
    /// time-critical by definition, without needing a second classifier entry point.
    /// Not `private` — `AskRoutingTests` asserts against this exact constant.
    static let nonTimeCriticalSentinelQuery = "summarize"
}

/// The time lines under a meeting or an episode in a Library list.
enum LibraryTimeText {
    static func formattedEpisodeTime(startUnixMs: Int64, endUnixMs: Int64) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let end = Date(timeIntervalSince1970: TimeInterval(endUnixMs) / 1000)
        let startText = start.formatted(date: .omitted, time: .shortened)
        let minutes = max(0, Int((end.timeIntervalSince1970 - start.timeIntervalSince1970) / 60))
        return "\(startText) · \(minutes) min"
    }

    /// Start time, plus duration if the meeting has ended — e.g. "Jul 26, 2:30 PM · 45 min".
    /// Still-in-progress/scheduled meetings (`endUnixMs == nil`) show just the start time.
    static func formattedMeetingTime(startUnixMs: Int64, endUnixMs: Int64?) -> String {
        let start = Date(timeIntervalSince1970: TimeInterval(startUnixMs) / 1000)
        let startText = start.formatted(date: .abbreviated, time: .shortened)
        guard let endUnixMs else { return startText }
        let end = Date(timeIntervalSince1970: TimeInterval(endUnixMs) / 1000)
        let minutes = Int((end.timeIntervalSince1970 - start.timeIntervalSince1970) / 60)
        return "\(startText) · \(minutes) min"
    }
}
