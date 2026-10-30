import Foundation
import OSLog

/// JSON shape matching core/memory's `CaptureItem` (NFR-PRT-1 FFI boundary).
struct CoreCaptureItem: Codable {
    let id: Int64
    let deviceId: String
    let tsUnixMs: Int64
    let appBundleId: String
    let windowTitle: String
    let url: String?
    let text: String
    let hash: Int64
    let source: String?

    enum CodingKeys: String, CodingKey {
        case id
        case deviceId = "device_id"
        case tsUnixMs = "ts_unix_ms"
        case appBundleId = "app_bundle_id"
        case windowTitle = "window_title"
        case url, text, hash, source
    }
}

/// JSON shape matching core/memory's `MeetingTranscriptSegment` (FR-CHAT-1 follow-on).
struct CoreMeetingTranscriptSegment: Codable, Identifiable {
    let id: Int64
    let meetingId: Int64
    let meetingTitle: String
    let startCs: Int64
    let speakerLabel: String
    let text: String

    enum CodingKeys: String, CodingKey {
        case id
        case meetingId = "meeting_id"
        case meetingTitle = "meeting_title"
        case startCs = "start_cs"
        case speakerLabel = "speaker_label"
        case text
    }
}

/// JSON shape returned by `whimbrel_memory_get_transcript_segments_page`.
struct CoreTranscriptSegmentsPage: Codable {
    let segments: [CoreMeetingTranscriptSegment]
    let total: Int
}

/// One transcript line to insert — matches `whimbrel_memory_insert_transcript_segments`'s
/// expected per-element JSON shape (`meeting_id` is passed separately to that function,
/// not repeated per element, so it's not a field here).
struct NewTranscriptSegmentInput: Encodable {
    let startCs: Int64
    let speakerLabel: String
    let text: String

    enum CodingKeys: String, CodingKey {
        case startCs = "start_cs"
        case speakerLabel = "speaker_label"
        case text
    }
}

/// One calendar-event item to upsert — matches
/// `whimbrel_memory_upsert_calendar_event_items`'s expected per-element JSON shape.
struct NewCalendarEventItemInput: Codable, Equatable {
    let deviceId: String
    let source: String
    let externalId: String
    let tsUnixMs: Int64
    let title: String
    let actors: [String]

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case source
        case externalId = "external_id"
        case tsUnixMs = "ts_unix_ms"
        case title
        case actors
    }
}

/// JSON shape matching core/ingest's `IngestStats`.
struct CoreIngestStats: Codable {
    let recordsRead: Int
    let itemsInserted: Int
    let itemsDeduped: Int
    let parseErrors: Int

    enum CodingKeys: String, CodingKey {
        case recordsRead = "records_read"
        case itemsInserted = "items_inserted"
        case itemsDeduped = "items_deduped"
        case parseErrors = "parse_errors"
    }
}

/// JSON shape matching `whimbrel_mcp_list_client_tokens`'s per-element metadata (Task
/// 5's `core/memory::list_mcp_client_tokens`) — deliberately excludes the token
/// hash/salt, which the FFI layer never surfaces to Swift.
struct CoreMcpClientTokenInfo: Codable, Identifiable {
    let id: Int64
    let name: String
    let scopes: [String]
    let createdAtUnixMs: Int64
    let revokedAtUnixMs: Int64?

    enum CodingKeys: String, CodingKey {
        case id, name, scopes
        case createdAtUnixMs = "created_at_unix_ms"
        case revokedAtUnixMs = "revoked_at_unix_ms"
    }
}

/// JSON shape matching core/llm's `ContextItemRef` — a citation tagged with which table
/// (capture item, transcript segment, routine run, episode, or thread) the id belongs to.
struct CoreContextItemRef: Codable, Equatable {
    enum Kind: String, Codable {
        case capture, transcript
        case routineRun = "routine_run"
        case episode, thread, snapshot
    }
    let kind: Kind
    let id: Int64
}

/// JSON shape matching core/llm's `ChatAnswer` (FR-CHAT-1).
struct CoreChatAnswer: Codable {
    let text: String
    let citations: [CoreContextItemRef]
}

/// #74: `whimbrel_llm_prepare_ask`'s success payload.
struct CorePrepareAskData: Decodable {
    let prepareId: String
    let prompt: String

    enum CodingKeys: String, CodingKey {
        case prepareId = "prepare_id"
        case prompt
    }
}

/// `whimbrel_llm_prepare_meeting_ask`'s success payload.
struct CorePrepareMeetingAskData: Decodable {
    let prepareId: String
    let prompt: String
    let narrowed: Bool

    enum CodingKeys: String, CodingKey {
        case prepareId = "prepare_id"
        case prompt, narrowed
    }
}

/// #74: `whimbrel_llm_finalize_ask`'s plain (non-envelope) response shape.
private struct CoreFinalizeAskData: Decodable {
    let citations: [CoreContextItemRef]
}

/// JSON shape matching core/llm's `ChatTurn`/`ChatRole` — one turn of prior
/// conversation, oldest first, fed back into `askQuestion` for multi-turn follow-ups.
struct CoreChatTurn: Codable {
    enum Role: String, Codable {
        case user = "User"
        case assistant = "Assistant"
    }
    let role: Role
    let text: String
}

/// JSON shape matching core/llm's `DailyDigest` (FR-MEM-3).
struct CoreDailyDigest: Codable {
    let summary: String
    let openLoops: [String]

    enum CodingKeys: String, CodingKey {
        case summary
        case openLoops = "open_loops"
    }
}

/// JSON shape matching core/memory's `RoutineRun` (FR-RTN-1..4) — one completed
/// (or failed) execution of a routine, as returned by `whimbrel_routines_tick`.
/// `Equatable` (final-review fix) so `RoutinesInboxView` can `.onChange(of:)`
/// `RoutineScheduler.recentRuns` as a "go refresh from persisted history" signal.
/// What Away Mode knows about a meeting it could not attend (core's `AwayFollowupRequest`).
struct AwayFollowupRequest: Encodable {
    let meetingId: Int64
    let meetingTitle: String
    let meetingStartUnixMs: Int64
    let organizerEmail: String?
    let organizerName: String?
    let selfIsOrganizer: Bool
    let profileName: String
    let nowUnixMs: Int64

    enum CodingKeys: String, CodingKey {
        case meetingId = "meeting_id", meetingTitle = "meeting_title", meetingStartUnixMs = "meeting_start_unix_ms"
        case organizerEmail = "organizer_email", organizerName = "organizer_name", selfIsOrganizer = "self_is_organizer"
        case profileName = "profile_name", nowUnixMs = "now_unix_ms"
    }
}

/// `routineRunId` is the pending organizer draft in the approvals inbox, nil when none was filed.
struct AwayFollowupResult: Decodable {
    let followupId: Int64
    let routineRunId: Int64?

    enum CodingKeys: String, CodingKey {
        case followupId = "followup_id", routineRunId = "routine_run_id"
    }
}

/// A to-do Away Mode left behind for a meeting it could not attend (core's `MeetingFollowup`).
struct CoreMeetingFollowup: Decodable, Identifiable {
    let id: Int64
    let meetingId: Int64
    let text: String
    let routineRunId: Int64?
    let createdAtUnixMs: Int64
    let doneAtUnixMs: Int64?

    enum CodingKeys: String, CodingKey {
        case id, text
        case meetingId = "meeting_id", routineRunId = "routine_run_id"
        case createdAtUnixMs = "created_at_unix_ms", doneAtUnixMs = "done_at_unix_ms"
    }
}

/// A brief stored for a calendar event (core's `MeetingPrepRow`). `briefJson` is the
/// serialized brief; `brief` decodes it, or is nil if it does not parse.
struct CoreMeetingPrep: Decodable, Identifiable {
    let eventId: String
    let title: String
    let startUnixMs: Int64
    let generatedAtUnixMs: Int64
    let briefJson: String
    let provider: String
    let model: String

    var id: String { eventId }
    var brief: CoreMeetingBrief? {
        try? JSONDecoder().decode(CoreMeetingBrief.self, from: Data(briefJson.utf8))
    }

    enum CodingKeys: String, CodingKey {
        case eventId = "event_id", title, startUnixMs = "start_unix_ms", generatedAtUnixMs = "generated_at_unix_ms"
        case briefJson = "brief_json", provider, model
    }
}

struct CoreMeetingBrief: Decodable {
    struct Claim: Decodable { let text: String; let sources: [String] }
    struct AttendeeNote: Decodable {
        let who: String
        let note: String
        let sources: [String]
        let lastInteractionUnixMs: Int64?
        enum CodingKeys: String, CodingKey { case who, note, sources, lastInteractionUnixMs = "last_interaction_unix_ms" }
    }
    struct Source: Decodable, Identifiable {
        let id: String
        let kind: String
        let title: String
        let tsUnixMs: Int64
        let meetingId: Int64?
        let threadId: Int64?
        enum CodingKeys: String, CodingKey {
            case id, kind, title, tsUnixMs = "ts_unix_ms", meetingId = "meeting_id", threadId = "thread_id"
        }
    }
    let summary: Claim
    let attendees: [AttendeeNote]
    let openItems: [Claim]
    let openQuestions: [Claim]
    let sources: [Source]
    enum CodingKeys: String, CodingKey {
        case summary, attendees, openItems = "open_items", openQuestions = "open_questions", sources
    }
}

struct CoreRoutineRun: Codable, Identifiable, Equatable {
    let id: Int64
    let routineId: Int64
    let ranAtUnixMs: Int64
    let status: String
    let outputSummary: String
    let outputJson: String

    enum CodingKeys: String, CodingKey {
        case id
        case routineId = "routine_id"
        case ranAtUnixMs = "ran_at_unix_ms"
        case status
        case outputSummary = "output_summary"
        case outputJson = "output_json"
    }
}

/// JSON shape matching core/memory's `RoutineRunWithName` (FR-RTN-8 final-review fix) —
/// `CoreRoutineRun` plus the owning routine's `name`, as returned by
/// `whimbrel_routines_recent_runs`. This is the persisted-history shape `RoutinesInboxView`
/// fetches from, distinct from `CoreRoutineRun` (which comes from the in-memory,
/// relaunch-fragile `RoutineScheduler.recentRuns`).
struct CoreRoutineRunWithName: Codable, Identifiable {
    let id: Int64
    let routineId: Int64
    let routineName: String
    let ranAtUnixMs: Int64
    let status: String
    let outputSummary: String
    let outputJson: String
    /// The device that ran this routine, when it is not this one (a run pulled from another device).
    let originDeviceId: String?

    enum CodingKeys: String, CodingKey {
        case id
        case routineId = "routine_id"
        case routineName = "routine_name"
        case ranAtUnixMs = "ran_at_unix_ms"
        case status
        case outputSummary = "output_summary"
        case outputJson = "output_json"
        case originDeviceId = "origin_device_id"
    }
}

/// One row from `episodes` — see `whimbrel_memory_recent_episodes`/`whimbrel_memory_get_episode`.
/// One device's capture count and newest capture time over a range.
struct CoreDeviceActivity: Decodable, Equatable {
    let deviceId: String
    let captures: Int64
    let lastTsUnixMs: Int64

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case captures
        case lastTsUnixMs = "last_ts_unix_ms"
    }
}

struct CoreEpisode: Codable, Identifiable {
    let id: Int64
    let deviceId: String
    let appBundleId: String
    let windowTitle: String?
    let startTsUnixMs: Int64
    let endTsUnixMs: Int64
    let itemCount: Int64
    let summary: String?
    let status: String

    enum CodingKeys: String, CodingKey {
        case id
        case deviceId = "device_id"
        case appBundleId = "app_bundle_id"
        case windowTitle = "window_title"
        case startTsUnixMs = "start_ts_unix_ms"
        case endTsUnixMs = "end_ts_unix_ms"
        case itemCount = "item_count"
        case summary
        case status
    }
}

/// One row from `threads` — see `whimbrel_threads_needing_attention`/`whimbrel_memory_get_thread`.
struct CoreThread: Codable, Identifiable {
    let id: Int64
    let source: String
    let externalThreadId: String
    let accountEmail: String
    let subject: String?
    let messageCount: Int64
    let lastActivityTsUnixMs: Int64
    let lastMessageFrom: String
    let summary: String?
    let status: String
    let summarizeStatus: String

    enum CodingKeys: String, CodingKey {
        case id
        case source
        case externalThreadId = "external_thread_id"
        case accountEmail = "account_email"
        case subject
        case messageCount = "message_count"
        case lastActivityTsUnixMs = "last_activity_ts_unix_ms"
        case lastMessageFrom = "last_message_from"
        case summary
        case status
        case summarizeStatus = "summarize_status"
    }
}

/// JSON shape matching core/memory's `Routine` (FR-RTN-1..4) — a scheduled
/// automation's config, as returned by `whimbrel_routines_list`.
struct CoreRoutine: Codable, Identifiable {
    let id: Int64
    let name: String
    let template: String
    let scheduleHhmm: String
    let enabled: Bool
    let autoApproveWriteActions: Bool
    /// Which model chain the routine's runs use: `default`, `cheapest`, or
    /// `<provider>/<model>` (core/api::routing::RoutineModelPref).
    var modelPref: String = "default"
    let createdAtUnixMs: Int64

    enum CodingKeys: String, CodingKey {
        case id, name, template
        case scheduleHhmm = "schedule_hhmm"
        case enabled
        case autoApproveWriteActions = "auto_approve_write_actions"
        case modelPref = "model_pref"
        case createdAtUnixMs = "created_at_unix_ms"
    }

    init(id: Int64, name: String, template: String, scheduleHhmm: String, enabled: Bool, autoApproveWriteActions: Bool, modelPref: String = "default", createdAtUnixMs: Int64) {
        self.id = id
        self.name = name
        self.template = template
        self.scheduleHhmm = scheduleHhmm
        self.enabled = enabled
        self.autoApproveWriteActions = autoApproveWriteActions
        self.modelPref = modelPref
        self.createdAtUnixMs = createdAtUnixMs
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        template = try c.decode(String.self, forKey: .template)
        scheduleHhmm = try c.decode(String.self, forKey: .scheduleHhmm)
        enabled = try c.decode(Bool.self, forKey: .enabled)
        autoApproveWriteActions = try c.decode(Bool.self, forKey: .autoApproveWriteActions)
        // Absent from a core older than the column: read as the chain it ran on before.
        modelPref = try c.decodeIfPresent(String.self, forKey: .modelPref) ?? "default"
        createdAtUnixMs = try c.decode(Int64.self, forKey: .createdAtUnixMs)
    }
}

/// JSON shape matching core/agent's `WriteActionPayload` (Task 2) -- the exact
/// payload `whimbrel_routines_pending_action_json` returns for one pending write
/// action. A single `"kind"`-tagged enum, not two separate structs, so decoding
/// can switch straight to the right variant off the JSON's own `"kind"` field
/// rather than guessing which shape to try first -- same discriminated-decode
/// pattern as `CoreLLMError`'s own `init(from:)` above.
enum CoreWriteActionPayload: Decodable {
    case createCalendarEvent(title: String, startUnixMs: Int64, endUnixMs: Int64)
    case sendEmail(to: String, subject: String, body: String)

    private enum CodingKeys: String, CodingKey {
        case kind, title
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case to, subject, body
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(String.self, forKey: .kind)
        switch kind {
        case "create_calendar_event":
            self = .createCalendarEvent(
                title: try container.decode(String.self, forKey: .title),
                startUnixMs: try container.decode(Int64.self, forKey: .startUnixMs),
                endUnixMs: try container.decode(Int64.self, forKey: .endUnixMs)
            )
        case "send_email":
            self = .sendEmail(
                to: try container.decode(String.self, forKey: .to),
                subject: try container.decode(String.self, forKey: .subject),
                body: try container.decode(String.self, forKey: .body)
            )
        default:
            throw DecodingError.dataCorruptedError(forKey: .kind, in: container, debugDescription: "unknown write action kind: \(kind)")
        }
    }
}

/// JSON shape of `whimbrel_routines_plan_write_action`'s `plan` (core/api::write_action).
struct CoreWriteActionPlan: Codable, Equatable {
    let runId: Int64
    /// `"google-calendar"` or `"gmail"`.
    let connector: String
    let tool: String
    let argsJson: String
    let tokenEnv: String
    let noTokenMessage: String
    let successSummary: String
    let terminalFailureSummary: String

    enum CodingKeys: String, CodingKey {
        case runId = "run_id"
        case connector, tool
        case argsJson = "args_json"
        case tokenEnv = "token_env"
        case noTokenMessage = "no_token_message"
        case successSummary = "success_summary"
        case terminalFailureSummary = "terminal_failure_summary"
    }
}

enum CoreWriteActionPlanResult {
    case notClaimable
    case undecodable
    case planned(CoreWriteActionPlan)
}

/// The real outcome of one connector round trip, as `whimbrel_routines_settle_write_action`
/// takes it. `success`/`terminalFailure` reached Google (finalize); `retryableFailure`/
/// `noToken` did not (release, the draft survives).
enum CoreWriteActionOutcome: Equatable {
    case success(resultJson: String)
    case terminalFailure
    case retryableFailure(reason: String)
    case noToken

    fileprivate var json: String {
        switch self {
        case let .success(resultJson):
            let data = (try? JSONSerialization.data(withJSONObject: ["kind": "success", "result_json": resultJson])) ?? Data()
            return String(decoding: data, as: UTF8.self)
        case .terminalFailure: return #"{"kind":"terminal_failure"}"#
        case let .retryableFailure(reason):
            let data = (try? JSONSerialization.data(withJSONObject: ["kind": "retryable_failure", "reason": reason])) ?? Data()
            return String(decoding: data, as: UTF8.self)
        case .noToken: return #"{"kind":"no_token"}"#
        }
    }
}

/// What `whimbrel_oauth_begin` takes (core/api::oauth::ProviderConfig).
struct CoreOAuthConfig: Encodable {
    let provider: String
    let clientId: String
    let clientSecret: String?
    let scope: String

    enum CodingKeys: String, CodingKey {
        case provider
        case clientId = "client_id"
        case clientSecret = "client_secret"
        case scope
    }
}

/// What `whimbrel_oauth_begin` returns: open `authURL` in a browser, then wait.
struct CoreOAuthSession: Decodable {
    let sessionId: String
    let authURL: URL
    let redirectURI: String

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case authURL = "auth_url"
        case redirectURI = "redirect_uri"
    }
}

/// core/api::oauth's error set, as `whimbrel_oauth_wait`/`whimbrel_oauth_refresh` report it.
enum CoreOAuthError: Error, Equatable {
    case listenerFailed
    case stateMismatch
    case timedOut
    case authorizationDenied(String)
    case tokenExchangeFailed
    case tokenRefreshFailed
    /// The provider answered `invalid_grant`: the refresh token is revoked or expired.
    case refreshTokenRevoked
    case noRefreshTokenReturned
    case cancelled
    case unknownSession
    case unknownProvider

    fileprivate init(code: String, reason: String?) {
        switch code {
        case "state_mismatch": self = .stateMismatch
        case "timed_out": self = .timedOut
        case "authorization_denied": self = .authorizationDenied(reason ?? "")
        case "token_exchange_failed": self = .tokenExchangeFailed
        case "token_refresh_failed": self = .tokenRefreshFailed
        case "refresh_token_revoked": self = .refreshTokenRevoked
        case "no_refresh_token_returned": self = .noRefreshTokenReturned
        case "cancelled": self = .cancelled
        case "unknown_session": self = .unknownSession
        case "unknown_provider": self = .unknownProvider
        default: self = .listenerFailed
        }
    }
}

struct CoreWriteActionSettled: Decodable {
    let finalized: Bool
    let userMessage: String?

    enum CodingKeys: String, CodingKey {
        case finalized
        case userMessage = "user_message"
    }
}

/// One connector tool call the shell makes between `*SyncRequest` and `apply*Sync`
/// (core/api::connectors::ToolCall).
struct CoreConnectorToolCall: Decodable {
    let tool: String
    let argsJson: String

    enum CodingKeys: String, CodingKey {
        case tool
        case argsJson = "args_json"
    }
}

/// JSON shape of `whimbrel_connector_calendar_apply`'s result.
struct CoreCalendarApplyOutcome: Decodable {
    let needsFullResync: Bool
    let meetings: [QualifyingMeeting]
    let cancelledCount: Int
    let deletionFailed: Bool
    let tokenSaved: Bool

    enum CodingKeys: String, CodingKey {
        case needsFullResync = "needs_full_resync"
        case meetings
        case cancelledCount = "cancelled_count"
        case deletionFailed = "deletion_failed"
        case tokenSaved = "token_saved"
    }
}

/// JSON shape of `whimbrel_connector_gmail_apply`'s result.
struct CoreGmailApplyOutcome: Decodable {
    let needsFullResync: Bool
    let deleted: Int
    let upserted: Int
    let applied: Bool
    let historySaved: Bool

    enum CodingKeys: String, CodingKey {
        case needsFullResync = "needs_full_resync"
        case deleted, upserted, applied
        case historySaved = "history_saved"
    }
}

/// JSON shape of `whimbrel_connector_accounts_list`'s result (core/api::connectors::accounts).
struct CoreConnectorAccounts: Decodable, Equatable {
    let emails: [String]
    let defaultEmail: String?

    enum CodingKeys: String, CodingKey {
        case emails
        case defaultEmail = "default_email"
    }
}

/// One calendar event, as sent to `whimbrel_routines_tick`'s `today_events_json`
/// parameter (Swift's EventKit read of today's qualifying meetings).
struct CoreCalendarEventSummary: Encodable {
    let title: String
    let startUnixMs: Int64

    enum CodingKeys: String, CodingKey {
        case title
        case startUnixMs = "start_unix_ms"
    }
}

/// JSON shape sent to `whimbrel_llm_answer`/`whimbrel_llm_daily_digest` as
/// `routing_config_json` (Settings UI — see
/// docs/superpowers/specs/2026-07-27-settings-ui-routing-design.md). Null key fields
/// mean "no key configured"; null cap fields mean "unset, use the conservative built-in
/// default" — `0` is the distinct, meaningful "explicit opt-out" value, so callers must
/// never coerce an unset cap to `0` before constructing this.
struct CoreRoutingConfig: Encodable {
    let anthropicKey: String?
    let openaiKey: String?
    let googleKey: String?
    let typesafeKey: String?
    let userDailyCapCents: Int?
    let globalDailyCapCents: Int?
    let sensitive: Bool
    let preferredProvider: String
    let preferredModel: String
    let customInstructions: String
    let dailyDigestInstructions: String
    let weeklyDigestInstructions: String
    /// The user's Profile text (empty when off); core composes it into prompts.
    var profile: String = ""
    /// Cheap-first routing: light tasks use the cheapest model the user has. Core's
    /// default is on (docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md).
    var cheapFirst: Bool = true
    /// Whether Ollama is a link in the chain at all; ignored in local-only mode.
    var useLocalModel: Bool = true
    var openrouterKey: String? = nil
    /// The user's OpenRouter pick; empty means none yet, and core adds no link.
    var openrouterModel: String = ""
    /// Cents per 1K input and output per OpenRouter model id, cached from the catalogue.
    var openrouterPrices: [String: Price] = [:]

    /// Cents per 1K tokens, in and out, as core's price table counts.
    struct Price: Codable, Equatable {
        let `in`: Double
        let out: Double
    }

    /// A copy carrying the TypeSafe key; set only at the moment of the core call.
    func withTypesafeKey(_ key: String?) -> CoreRoutingConfig {
        CoreRoutingConfig(
            anthropicKey: anthropicKey, openaiKey: openaiKey, googleKey: googleKey, typesafeKey: key,
            userDailyCapCents: userDailyCapCents, globalDailyCapCents: globalDailyCapCents,
            sensitive: sensitive, preferredProvider: preferredProvider, preferredModel: preferredModel,
            customInstructions: customInstructions, dailyDigestInstructions: dailyDigestInstructions,
            weeklyDigestInstructions: weeklyDigestInstructions, profile: profile,
            cheapFirst: cheapFirst, useLocalModel: useLocalModel,
            openrouterKey: openrouterKey, openrouterModel: openrouterModel, openrouterPrices: openrouterPrices
        )
    }

    enum CodingKeys: String, CodingKey {
        case anthropicKey = "anthropic_key"
        case openaiKey = "openai_key"
        case googleKey = "google_key"
        case typesafeKey = "typesafe_key"
        case userDailyCapCents = "user_daily_cap_cents"
        case globalDailyCapCents = "global_daily_cap_cents"
        case sensitive
        case preferredProvider = "preferred_provider"
        case preferredModel = "preferred_model"
        case customInstructions = "custom_instructions"
        case dailyDigestInstructions = "daily_digest_instructions"
        case weeklyDigestInstructions = "weekly_digest_instructions"
        case profile
        case cheapFirst = "cheap_first"
        case useLocalModel = "use_local_model"
        case openrouterKey = "openrouter_key"
        case openrouterModel = "openrouter_model"
        case openrouterPrices = "openrouter_prices"
    }
}

/// One OpenRouter catalogue entry with its price, as `whimbrel_key_check` returns it
/// for that provider (core/api::hosted_models::OpenRouterModel).
struct CoreOpenRouterModel: Codable, Equatable {
    let id: String
    let inCentsPer1k: Double
    let outCentsPer1k: Double

    enum CodingKeys: String, CodingKey {
        case id
        case inCentsPer1k = "in_cents_per_1k"
        case outCentsPer1k = "out_cents_per_1k"
    }
}

/// JSON shape matching `whimbrel_usage_totals` (core/api::usage::UsageTotals): spend in
/// cents for the windows the Model usage screen shows, UTC days.
struct CoreUsageTotals: Codable, Equatable {
    struct ProviderSpend: Codable, Equatable {
        let provider: String
        let cents: Int64
    }

    let todayCents: Int64
    let last30DaysCents: Int64
    let monthToDateCents: Int64
    let last30DaysByProvider: [ProviderSpend]

    enum CodingKeys: String, CodingKey {
        case todayCents = "today_cents"
        case last30DaysCents = "last_30_days_cents"
        case monthToDateCents = "month_to_date_cents"
        case last30DaysByProvider = "last_30_days_by_provider"
    }

    /// Last-30-day spend for one provider id, zero when it made no calls.
    func cents(for provider: String) -> Int64 {
        last30DaysByProvider.first { $0.provider == provider }?.cents ?? 0
    }
}

/// One call as `whimbrel_usage_list` returns it (core/api::usage::UsageRow).
struct CoreUsageRow: Codable, Identifiable, Equatable {
    let tsUnixMs: Int64
    let task: String
    let provider: String
    let model: String
    let inputTokens: Int64?
    let outputTokens: Int64?
    let estimatedCostCents: Int64
    /// A local call is not metered; the screen says "local", never a zero.
    let local: Bool

    var id: String { "\(tsUnixMs)-\(provider)-\(model)-\(task)" }

    enum CodingKeys: String, CodingKey {
        case tsUnixMs = "ts_unix_ms"
        case task, provider, model, local
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case estimatedCostCents = "estimated_cost_cents"
    }
}

/// One row of the key on-ramp table (`whimbrel_providers`, core/api::providers).
struct CoreProviderInfo: Codable, Identifiable, Equatable {
    struct DataTerms: Codable, Equatable {
        let text: String
        /// ISO date the policy page was last read.
        let checked: String
    }

    let id: String
    let name: String
    let keyPageUrl: String?
    let policyUrl: String?
    let keyPrefix: String?
    let dataTerms: DataTerms?
    let cheapModel: String

    enum CodingKeys: String, CodingKey {
        case id, name
        case keyPageUrl = "key_page_url"
        case policyUrl = "policy_url"
        case keyPrefix = "key_prefix"
        case dataTerms = "data_terms"
        case cheapModel = "cheap_model"
    }
}

/// What saving a key found out (`whimbrel_key_check`, core/api::hosted_models::KeyCheck),
/// decoded off its `status` tag.
enum CoreKeyCheck: Equatable {
    /// `prices` is filled for OpenRouter only.
    case valid(models: [String], prices: [CoreOpenRouterModel])
    case refused(message: String)
    case unreachable(message: String)
    case other(message: String)

    /// The one line the key row shows for this outcome.
    var message: String {
        switch self {
        case let .valid(models, _):
            return "Saved. \(models.count) model\(models.count == 1 ? "" : "s") available."
        case let .refused(message), let .unreachable(message), let .other(message):
            return message
        }
    }

    /// Whether the line is a problem the user has to act on (a refused key) rather than
    /// a note (saved, or offline for now).
    var isError: Bool {
        if case .refused = self { return true }
        if case .other = self { return true }
        return false
    }
}

extension CoreKeyCheck: Decodable {
    private enum CodingKeys: String, CodingKey { case status, models, prices, message }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let status = try container.decode(String.self, forKey: .status)
        switch status {
        case "valid":
            self = .valid(
                models: try container.decodeIfPresent([String].self, forKey: .models) ?? [],
                prices: try container.decodeIfPresent([CoreOpenRouterModel].self, forKey: .prices) ?? []
            )
        case "refused":
            self = .refused(message: try container.decode(String.self, forKey: .message))
        case "unreachable":
            self = .unreachable(message: try container.decode(String.self, forKey: .message))
        default:
            self = .other(message: try container.decodeIfPresent(String.self, forKey: .message) ?? "Couldn't check the key.")
        }
    }
}

/// JSON shape matching `whimbrel_llm_spend_today`'s return value (FR-CHAT-2
/// governance): today's spend so far, in cents, using the same UTC-midnight day
/// boundary as `check_spend_cap`.
struct CoreSpendToday: Codable {
    let userCents: Int64
    let globalCents: Int64

    enum CodingKeys: String, CodingKey {
        case userCents = "user_cents"
        case globalCents = "global_cents"
    }
}

/// Discriminated result of an LLM-governed FFI call (`whimbrel_llm_answer`,
/// `whimbrel_llm_daily_digest`, `whimbrel_llm_summarize_meeting`) — LB0-5. A `nil`
/// return on the enclosing `Result<T, CoreLLMError>?` (see `askQuestion`, etc.) is
/// unrelated to this type — it means genuinely invalid usage (null/invalid handle,
/// malformed argument JSON), not a well-formed error. `CoreLLMError` only ever
/// represents a real `{"ok":false,"error":{...}}` envelope from a call that reached
/// the Rust core.
enum CoreLLMError: Error, Equatable, Decodable {
    case spendCap(scope: Scope, spentCents: Int64, capCents: Int64)
    case providerFailure
    case noTranscript

    enum Scope: String, Decodable {
        case user
        case global
    }

    private enum CodingKeys: String, CodingKey {
        case kind, scope
        case spentCents = "spent_cents"
        case capCents = "cap_cents"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(String.self, forKey: .kind)
        switch kind {
        case "spend_cap":
            let scope = try container.decode(Scope.self, forKey: .scope)
            let spentCents = try container.decode(Int64.self, forKey: .spentCents)
            let capCents = try container.decode(Int64.self, forKey: .capCents)
            self = .spendCap(scope: scope, spentCents: spentCents, capCents: capCents)
        case "no_transcript":
            self = .noTranscript
        default:
            // "provider_failure", or any future kind this build doesn't recognize yet
            // -- falls back to the generic case rather than failing to decode,
            // matching this codebase's forward-compat convention for unrecognized
            // string enums (e.g. core/memory's MeetingTrigger::from_str).
            self = .providerFailure
        }
    }

    /// User-facing text for the Ask-chat and meeting-summary-regenerate UIs (LB0-5) —
    /// kept here, not duplicated per call site, so both surfaces stay in sync.
    var userFacingMessage: String {
        switch self {
        case .spendCap(.user, let spentCents, let capCents):
            return "Daily spend cap reached (\(spentCents)¢/\(capCents)¢). Try again after it resets, or raise the cap in Settings."
        case .spendCap(.global, let spentCents, let capCents):
            return "The app-wide daily spend limit was reached (\(spentCents)¢/\(capCents)¢). This isn't specific to you — try again later."
        case .providerFailure:
            return "Couldn't reach the AI provider. Try again."
        case .noTranscript:
            return "There is no transcript for this meeting to ask about."
        }
    }
}

/// The full envelope shape `whimbrel_llm_answer`/`whimbrel_llm_daily_digest`/
/// `whimbrel_llm_summarize_meeting` now return (LB0-5) — `data`'s type varies per
/// call site, so this is generic over it.
struct CoreLLMEnvelope<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: CoreLLMError?

    private static var log: Logger { Logger(subsystem: "com.mpointer.kestrel", category: "core-bridge") }

    /// The envelope at `json` as a `Result`; an undecodable one is `.providerFailure`.
    static func result(_ json: UnsafeMutablePointer<CChar>) -> Result<T, CoreLLMError> {
        let data = Data(String(cString: json).utf8)
        guard let envelope = try? JSONDecoder().decode(CoreLLMEnvelope<T>.self, from: data) else {
            log.error("decodeLLMResult: failed to decode envelope JSON")
            return .failure(.providerFailure)
        }
        if envelope.ok, let value = envelope.data {
            return .success(value)
        }
        return .failure(envelope.error ?? .providerFailure)
    }
}

/// JSON shape matching core/transcribe's `TranscriptSegment` (FR-MTG-4). Timestamps are
/// centiseconds (10s of milliseconds), as whisper.cpp reports them.
struct CoreTranscriptSegment: Codable {
    let startCs: Int64
    let endCs: Int64
    let text: String

    enum CodingKeys: String, CodingKey {
        case startCs = "start_cs"
        case endCs = "end_cs"
        case text
    }
}

/// JSON shape matching core/diarize's `SpeakerSegment`.
struct CoreSpeakerSegment: Codable {
    let startCs: Int64
    let endCs: Int64
    let speakerId: Int

    enum CodingKeys: String, CodingKey {
        case startCs = "start_cs"
        case endCs = "end_cs"
        case speakerId = "speaker_id"
    }
}

/// JSON shape matching core/diarize's `LabeledSegment`. `speakerId` is nil when no
/// diarization speaker segment overlapped this transcript segment's midpoint — callers
/// treat that as "keep the pre-diarization label," not an error.
struct CoreLabeledSegment: Codable {
    let startCs: Int64
    let endCs: Int64
    let text: String
    let speakerId: Int?

    enum CodingKeys: String, CodingKey {
        case startCs = "start_cs"
        case endCs = "end_cs"
        case text
        case speakerId = "speaker_id"
    }
}

/// One speaker's contiguous turn in a transcript, derived by
/// `CoreMemoryStore.parseTranscriptTurns` from the flat `"[HH:mm:ss] Speaker: text"`
/// transcript string for display only. The flat string stays the source of truth
/// `regenerate()` sends to the LLM. JSON shape matches
/// core/transcribe's `postprocess::Turn`.
struct TranscriptTurn: Codable, Equatable {
    let speaker: String
    let timestamp: String
    let text: String
}

/// JSON shape of `whimbrel_transcript_postprocess`'s result.
private struct CoreTranscriptPostprocessResult: Decodable {
    let lines: [TranscriptionService.Line]
    let micLinesKept: Int

    enum CodingKeys: String, CodingKey {
        case lines
        case micLinesKept = "mic_lines_kept"
    }
}

/// JSON shape matching core/memory's `MeetingTrigger` (FR-MTG-1/2).
enum CoreMeetingTrigger: String, Codable {
    case calendar
    case detected
    case manual
    case away
}

/// JSON shape matching core/memory's `AwayOutcome`: how an Away Mode meeting went.
enum AwayOutcome: String, Codable {
    case attended
    case chatNotPosted = "chat_not_posted"
    case removed
    case waitingRoom = "waiting_room"
    case passcodeRequired = "passcode_required"
    case signInRequired = "sign_in_required"
    case notMuted = "not_muted"
    case screenLocked = "screen_locked"
    case appFailed = "app_failed"
    case missedOverlap = "missed_overlap"
}

/// JSON shape matching core/memory's `MeetingStatus`.
enum CoreMeetingStatus: String, Codable {
    case scheduled
    case recording
    case completed
    case cancelled
}

/// Mirrors core/memory's `MeetingPart`: one recording session of a (possibly continued) meeting.
struct CoreMeetingPart: Decodable, Equatable {
    let partIndex: Int64
    let startUnixMs: Int64
    let endUnixMs: Int64?
    let audioPath: String?
    let transcriptPath: String?
    let transcribed: Bool

    enum CodingKeys: String, CodingKey {
        case partIndex = "part_index"
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case audioPath = "audio_path"
        case transcriptPath = "transcript_path"
        case transcribed
    }
}

/// Mirrors core/api's `media_retention::MediaStep`.
enum CoreMediaStep: String, Codable, Equatable {
    case moveAudio = "move_audio"
    case moveSnapshots = "move_snapshots"
    case compress
    case deleteAudio = "delete_audio"
    case shrinkImages = "shrink_images"
}

/// Mirrors core/api's `media_retention::MeetingWork`.
struct CoreMeetingWork: Codable, Equatable {
    let meetingId: Int64
    let steps: [CoreMediaStep]
    let audioPaths: [String]
    let shrinkSnapshotIds: [Int64]

    enum CodingKeys: String, CodingKey {
        case meetingId = "meeting_id"
        case steps
        case audioPaths = "audio_paths"
        case shrinkSnapshotIds = "shrink_snapshot_ids"
    }
}

/// Mirrors core/api's `media_retention::MediaUsage`.
struct CoreMediaUsage: Codable, Equatable {
    let audioFullBytes: Int64
    let audioCompressedBytes: Int64
    let imageBytes: Int64
    /// What deleting audio would free now (at the preview age while deletion is off).
    var audioDeletableBytes: Int64 = 0

    enum CodingKeys: String, CodingKey {
        case audioFullBytes = "audio_full_bytes"
        case audioCompressedBytes = "audio_compressed_bytes"
        case imageBytes = "image_bytes"
        case audioDeletableBytes = "audio_deletable_bytes"
    }
}

/// Mirrors core/api's `media_retention::MediaPlan`.
struct CoreMediaPlan: Codable, Equatable {
    let work: [CoreMeetingWork]
    let disconnectedMeetings: Int
    let usage: CoreMediaUsage

    enum CodingKeys: String, CodingKey {
        case work
        case disconnectedMeetings = "disconnected_meetings"
        case usage
    }
}

/// Mirrors core/memory's `SplitResult`.
struct CoreSplitResult: Decodable, Equatable {
    let newMeetingId: Int64
    /// Snapshot files now owned by the new meeting; the caller moves them on disk and then
    /// calls `setMeetingSnapshotPaths`.
    let movedSnapshotPaths: [String]

    enum CodingKeys: String, CodingKey {
        case newMeetingId = "new_meeting_id"
        case movedSnapshotPaths = "moved_snapshot_paths"
    }
}

/// JSON shape sent to `whimbrel_memory_insert_meeting`, matching core/memory's `NewMeeting`.
struct CoreNewMeeting: Encodable {
    let deviceId: String
    let startUnixMs: Int64
    let endUnixMs: Int64?
    let title: String
    let attendees: [String]
    let trigger: CoreMeetingTrigger
    let audioPath: String?
    let status: CoreMeetingStatus

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case title, attendees, trigger
        case audioPath = "audio_path"
        case status
    }
}

/// Mirrors core/memory's `MeetingSnapshot`: one slide image saved from a meeting.
/// A slide-OCR search hit (`memory::MeetingSnapshotHit`); `id` is the items row id.
struct CoreMeetingSnapshotHit: Codable, Identifiable, Equatable {
    let id: Int64
    let meetingId: Int64
    let meetingTitle: String
    let text: String
    let filePath: String

    enum CodingKeys: String, CodingKey {
        case id
        case meetingId = "meeting_id"
        case meetingTitle = "meeting_title"
        case text
        case filePath = "file_path"
    }
}

struct CoreMeetingSnapshot: Codable, Identifiable, Equatable {
    let id: Int64
    let meetingId: Int64
    let capturedUnixMs: Int64
    let filePath: String
    let width: Int
    let height: Int
    let ocrText: String

    enum CodingKeys: String, CodingKey {
        case id
        case meetingId = "meeting_id"
        case capturedUnixMs = "captured_unix_ms"
        case filePath = "file_path"
        case width, height
        case ocrText = "ocr_text"
    }
}

/// JSON shape matching core/memory's `Meeting` (FR-MTG-1/2/5). `Identifiable` (via the
/// existing `id`) so Timeline's meeting detail view can be presented with
/// `.sheet(item:)`.
/// Another recording of the same call as the meeting it's listed under.
struct CoreMeetingCopy: Codable, Identifiable, Equatable {
    let id: Int64
    let deviceId: String
    let title: String

    enum CodingKeys: String, CodingKey {
        case id
        case deviceId = "device_id"
        case title
    }
}

struct CoreMeeting: Codable, Identifiable {
    let id: Int64
    let deviceId: String
    let startUnixMs: Int64
    let endUnixMs: Int64?
    let title: String
    let attendees: [String]
    let trigger: CoreMeetingTrigger
    let audioPath: String?
    let status: CoreMeetingStatus
    let transcriptPath: String?
    let notes: String?
    let summaryTldr: String?
    let summaryDecisions: [String]
    let summaryActionItems: [String]
    let summaryOpenQuestions: [String]
    let summaryProvider: String?
    let summaryModel: String?
    let estimatedSpeakerCount: Int64?
    /// Other recordings of the same call; only set by `recentMeetingsGrouped`.
    var alsoRecorded: [CoreMeetingCopy]? = nil
    /// Away Mode outcome; nil on a non-away meeting, and on an away one that was attended.
    var awayOutcome: AwayOutcome? = nil
    var awayOutcomeAtUnixMs: Int64? = nil
    /// "Keep media": exempt from the retention compress/shrink/delete steps. Stored
    /// optional so a payload without the key decodes (as false) instead of failing.
    var keepMediaStored: Bool? = nil
    var keepMedia: Bool { keepMediaStored ?? false }

    /// This device's own recording of the call: the meeting itself, or its copy in
    /// `alsoRecorded` when the lead is a peer's. nil when this device has none.
    func ownCopyId(deviceId: String) -> Int64? {
        if self.deviceId == deviceId { return id }
        return alsoRecorded?.first { $0.deviceId == deviceId }?.id
    }
    /// When Whimbrel was removed from the call (`.removed` only).
    var awayOutcomeAt: Date? {
        awayOutcomeAtUnixMs.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) }
    }

    enum CodingKeys: String, CodingKey {
        case id
        case deviceId = "device_id"
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case title, attendees, trigger
        case alsoRecorded = "also_recorded"
        case audioPath = "audio_path"
        case status
        case transcriptPath = "transcript_path"
        case notes
        case summaryTldr = "summary_tldr"
        case summaryDecisions = "summary_decisions"
        case summaryActionItems = "summary_action_items"
        case summaryOpenQuestions = "summary_open_questions"
        case summaryProvider = "summary_provider"
        case summaryModel = "summary_model"
        case estimatedSpeakerCount = "estimated_speaker_count"
        case awayOutcome = "away_outcome"
        case awayOutcomeAtUnixMs = "away_outcome_at_unix_ms"
        case keepMediaStored = "keep_media"
    }
}

/// JSON shape matching core/memory's `MeetingNoteDraft`.
struct CoreMeetingNoteDraft: Codable {
    let meetingId: Int64
    let summaryEdited: String?
    let decisionsEdited: [String]?
    let openQuestionsEdited: [String]?
    let personalNotes: String?
    let lastEditedUnixMs: Int64
    let lastRegeneratedSeenUnixMs: Int64?

    enum CodingKeys: String, CodingKey {
        case meetingId = "meeting_id"
        case summaryEdited = "summary_edited"
        case decisionsEdited = "decisions_edited"
        case openQuestionsEdited = "open_questions_edited"
        case personalNotes = "personal_notes"
        case lastEditedUnixMs = "last_edited_unix_ms"
        case lastRegeneratedSeenUnixMs = "last_regenerated_seen_unix_ms"
    }
}

/// JSON shape matching core/memory's `MeetingActionItem`.
struct CoreMeetingActionItem: Codable, Identifiable, Equatable {
    let id: Int64
    let meetingId: Int64
    let text: String
    let completed: Bool
    let orderIndex: Int64
    let source: String
    let createdUnixMs: Int64
    let updatedUnixMs: Int64

    enum CodingKeys: String, CodingKey {
        case id
        case meetingId = "meeting_id"
        case text, completed
        case orderIndex = "order_index"
        case source
        case createdUnixMs = "created_unix_ms"
        case updatedUnixMs = "updated_unix_ms"
    }
}

/// JSON shape matching core/memory's `DeletedMeetingFiles`; round-trips to JSON so it can be
/// handed back to `CoreMemoryStore.deleteAllMeetingFiles`.
struct CoreDeletedMeetingFiles: Codable, Equatable {
    let audioPath: String?
    let transcriptPath: String?
    /// Nil when the meeting kept its snapshots or another meeting still has snapshots in the folder.
    let snapshotsDir: String?
    /// Every part's audio / transcript path (a continued meeting has several).
    let audioPaths: [String]
    let transcriptPaths: [String]

    enum CodingKeys: String, CodingKey {
        case audioPath = "audio_path"
        case transcriptPath = "transcript_path"
        case snapshotsDir = "snapshots_dir"
        case audioPaths = "audio_paths"
        case transcriptPaths = "transcript_paths"
    }
}

/// JSON shape matching core/memory's `RetentionResult` (FR-MEM-4).
struct CoreRetentionResult: Codable {
    let captureItemsDeleted: Int
    let meetingsDeleted: Int
    let deletedMeetingFiles: [CoreDeletedMeetingFiles]
    let conversationsDeleted: Int

    enum CodingKeys: String, CodingKey {
        case captureItemsDeleted = "capture_items_deleted"
        case meetingsDeleted = "meetings_deleted"
        case deletedMeetingFiles = "deleted_meeting_files"
        case conversationsDeleted = "conversations_deleted"
    }
}

/// JSON shape of `whimbrel_retention_scan_orphans`'s result: full paths of the orphaned
/// recording files and their summed on-disk size.
struct CoreOrphanScan: Codable {
    let orphaned: [String]
    let totalBytes: Int64

    enum CodingKeys: String, CodingKey {
        case orphaned
        case totalBytes = "total_bytes"
    }
}

/// JSON shape of `whimbrel_models_download_poll`'s result (core/api::models).
struct CoreModelDownloadProgress: Decodable {
    let fraction: Double
    let status: String
    let done: Bool
    let cancelled: Bool
    let error: String?
}

/// JSON shape of `whimbrel_update_check_manifest`'s result (core/api::update).
struct CoreUpdateManifestCheck: Decodable {
    let manifest: UpdateChecker.VersionManifest
    let newer: Bool
}

/// One built-in Ask starter prompt (core/api::prompts), surfaced as a chip in Ask mode.
struct LibraryPrompt: Identifiable, Decodable, Equatable {
    let id: String
    let title: String
    let query: String
}

/// JSON shape matching `whimbrel_llm_summarize_meeting`'s return value and
/// `core/llm::MeetingSummary` (FR-MTG-5).
struct CoreMeetingSummary: Codable {
    let tldr: String
    let decisions: [String]
    let actionItems: [String]
    let openQuestions: [String]
    let provider: String?
    let model: String?

    enum CodingKeys: String, CodingKey {
        case tldr
        case decisions
        case actionItems = "action_items"
        case openQuestions = "open_questions"
        case provider
        case model
    }
}

/// Thin Swift wrapper over core/ffi's C-ABI (NFR-PRT-1). Not thread-safe — the owner
/// must serialize access to a given instance, matching the FFI's own contract.
final class CoreMemoryStore {
    /// FR-MTG-5: every background FFI call into ANY `CoreMemoryStore` instance must
    /// serialize through this single queue rather than the generic concurrent
    /// `DispatchQueue.global` — the class's own contract above requires serialized
    /// access, and using the generic global queue independently at each call site does
    /// NOT provide that: two calls from two different call sites (e.g. the at-a-glance
    /// panel's digest generation and a background meeting-summarize pipeline) could
    /// otherwise run truly concurrently on different threads against the same handle.
    static let accessQueue = DispatchQueue(label: "com.mpointer.kestrel.corestore-access", qos: .userInitiated)
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "core-bridge")

    private let handle: OpaquePointer

    init?(path: URL, modelsDir: URL?) {
        let handle: OpaquePointer? = path.path.withCString { pathC in
            if let modelsDir {
                return modelsDir.path.withCString { modelsDirC in
                    whimbrel_memory_open(pathC, modelsDirC)
                }
            } else {
                return whimbrel_memory_open(pathC, nil)
            }
        }
        guard let handle else {
            return nil
        }
        self.handle = handle
    }

    /// The slow lane (docs/adr/0001, M3.3 Decision 2): calls that hold a store across an
    /// LLM request run here, on `AppState.slowStore`, so `accessQueue` and every UI read
    /// never wait behind a model.
    static let slowLaneQueue = DispatchQueue(label: "com.mpointer.kestrel.corestore-slow", qos: .utility)

    /// `text` with credentials replaced (core `memory::redact`); unchanged on failure.
    static func redactSecrets(_ text: String) -> String {
        guard let out = text.withCString({ whimbrel_redact_secrets($0) }) else { return text }
        defer { whimbrel_core_free_string(out) }
        return String(cString: out)
    }

    /// A second connection to `path` sharing `primary`'s embedder, for `slowLaneQueue`.
    /// Must be called on `accessQueue`, since it reads `primary`.
    init?(secondaryTo primary: CoreMemoryStore, path: URL) {
        guard let handle = path.path.withCString({ whimbrel_memory_open_secondary(primary.handle, $0) }) else {
            return nil
        }
        self.handle = handle
    }

    deinit {
        whimbrel_memory_close(handle)
    }

    /// docs/adr/0001 slice 2: registers this device's own sync id so meeting and
    /// transcript writes enqueue outbox rows for rows this device authored. Called once
    /// from `AppState.init`, before any other store work is queued.
    @discardableResult
    func setDeviceId(_ deviceId: String) -> Bool {
        deviceId.withCString { whimbrel_memory_set_device_id(handle, $0) }
    }

    /// Re-embeds up to `maxRows` pre-existing rows (Task 2's `Store::reindex_batch`).
    /// Returns the count actually processed (0 = fully caught up), or -1 on error.
    func reindexBatch(maxRows: Int) -> Int64 {
        whimbrel_memory_reindex_batch(handle, UInt64(maxRows))
    }

    /// Runs up to `maxJobs` pending jobs (see `Store::run_jobs_batch`). Returns the
    /// count actually processed (0 = nothing eligible), or -1 on error.
    func runJobsBatch(maxJobs: Int) -> Int64 {
        whimbrel_memory_run_jobs_batch(handle, UInt64(maxJobs))
    }

    /// Runs up to `maxJobs` pending `episode_segment` jobs only, independent of
    /// `runJobsBatch`'s shared `reindex_sweep`/`thread_state_sync` queue (see
    /// `Store::run_episode_segment_batch`). Returns the count actually processed
    /// (0 = nothing eligible), or -1 on error.
    func runEpisodeSegmentBatch(maxJobs: Int) -> Int64 {
        whimbrel_memory_run_episode_segment_batch(handle, UInt64(maxJobs))
    }

    /// Runs up to `maxJobs` pending episode-summarize jobs against the given local
    /// Ollama `model` (see `Store::claim_episode_summarize_jobs`). Returns the count
    /// actually processed (0 = nothing eligible), or -1 on error.
    func runEpisodeSummarizeBatch(model: String, maxJobs: Int, customInstructions: String) -> Int64 {
        model.withCString { modelPtr in
            customInstructions.withCString { instructionsPtr in
                whimbrel_episodes_run_summarize_batch(handle, modelPtr, UInt64(maxJobs), instructionsPtr)
            }
        }
    }

    /// Runs up to `maxJobs` pending thread-summarize jobs against the given local
    /// Ollama `model` (see `Store::claim_thread_summarize_jobs`). Returns the count
    /// actually processed (0 = nothing eligible), or -1 on error.
    func runThreadSummarizeBatch(model: String, maxJobs: Int, customInstructions: String) -> Int64 {
        model.withCString { modelPtr in
            customInstructions.withCString { instructionsPtr in
                whimbrel_threads_run_summarize_batch(handle, modelPtr, UInt64(maxJobs), instructionsPtr)
            }
        }
    }

    /// Runs up to `maxJobs` pending thread-priority jobs against TypeSafe (Reply
    /// Priorities). Returns the count processed (0 = nothing eligible), -1 on a null
    /// handle or key, or -2 when TypeSafe rejected the key.
    func runThreadPriorityBatch(apiKey: String, maxJobs: Int) -> Int64 {
        apiKey.withCString { keyPtr in
            whimbrel_threads_run_priority_batch(handle, keyPtr, UInt64(maxJobs))
        }
    }

    /// Queues a priority job for every thread currently waiting on the user. Returns the
    /// number queued, or -1 on error.
    func seedThreadPriorityJobs() -> Int64 {
        whimbrel_threads_seed_priority_jobs(handle)
    }

    /// Backfills transcript_segment rows for pre-feature meetings (LB0-6's
    /// `Store::backfill_transcript_segments_batch`). Returns the count actually
    /// processed (0 = fully caught up), or -1 on a null handle.
    func backfillTranscriptSegments(maxMeetings: Int) -> Int64 {
        whimbrel_memory_backfill_transcript_segments_batch(handle, UInt64(maxMeetings))
    }

    /// Whether the embedding model is currently loaded and usable.
    func embeddingModelLoaded() -> Bool {
        whimbrel_memory_embedding_loaded(handle)
    }

    func ingestJSONL(path: URL, deviceId: String) -> CoreIngestStats? {
        guard let json = path.path.withCString({ jsonlPath in
            deviceId.withCString { device in
                whimbrel_memory_ingest_jsonl(handle, jsonlPath, device)
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    func searchKeyword(_ query: String, limit: Int = 20) -> [CoreCaptureItem] {
        guard let json = query.withCString({ whimbrel_memory_search_keyword(handle, $0, limit) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    func searchSemantic(_ query: String, limit: Int = 20) -> [CoreCaptureItem] {
        guard let json = query.withCString({ whimbrel_memory_search_semantic(handle, $0, limit) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    /// FR-CHAT-1 follow-on (keyword half): FTS5 search over transcript segments.
    func searchTranscriptKeyword(_ query: String, limit: Int = 20) -> [CoreMeetingTranscriptSegment] {
        guard let json = query.withCString({ whimbrel_memory_search_transcript_keyword(handle, $0, limit) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    /// FR-CHAT-1 follow-on (semantic half): nearest-neighbor search over transcript
    /// segments.
    func searchTranscriptSemantic(_ query: String, limit: Int = 20) -> [CoreMeetingTranscriptSegment] {
        guard let json = query.withCString({ whimbrel_memory_search_transcript_semantic(handle, $0, limit) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    func searchSnapshotKeyword(_ query: String, limit: Int = 20) -> [CoreMeetingSnapshotHit] {
        guard let json = query.withCString({ whimbrel_memory_search_snapshot_keyword(handle, $0, limit) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    func searchSnapshotSemantic(_ query: String, limit: Int = 20) -> [CoreMeetingSnapshotHit] {
        guard let json = query.withCString({ whimbrel_memory_search_snapshot_semantic(handle, $0, limit) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    /// FR-MEM-1: most recent items, newest first — the default "browse the timeline"
    /// view with no search query.
    func recent(limit: Int = 50) -> [CoreCaptureItem] {
        guard let json = whimbrel_memory_recent(handle, limit) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    /// FR-CHAT-1: retrieval-grounded chat with citations. `model` is a local Ollama
    /// model name (e.g. "gemma4:12b") — always used for Ollama's own link in the
    /// provider chain, regardless of `routing.preferredProvider`. `history` is prior
    /// conversation turns, oldest first — pass the previous question/answer pairs for a
    /// follow-up question; empty for a fresh conversation. Note retrieval of context for
    /// this call still uses `query` alone, not `history` (see core/llm's
    /// `answer_with_citations` docs). `routing` carries Settings-sourced keys/caps/
    /// local-only-mode plus this conversation's provider+model pick. Returns `nil` only
    /// on genuinely invalid usage (null handle, malformed args) — otherwise (LB0-5) a
    /// `Result` distinguishing success from `.spendCap`/`.providerFailure`.
    func askQuestion(
        _ query: String, model: String, contextLimit: Int = 5, history: [CoreChatTurn] = [], routing: CoreRoutingConfig
    ) -> Result<CoreChatAnswer, CoreLLMError>? {
        guard let historyJSON = try? JSONEncoder().encode(history),
              let historyString = String(data: historyJSON, encoding: .utf8) else { return nil }
        guard let routingJSON = try? JSONEncoder().encode(routing),
              let routingString = String(data: routingJSON, encoding: .utf8) else { return nil }
        guard let json = query.withCString({ q in
            model.withCString { m in
                historyString.withCString { h in
                    routingString.withCString { r in
                        whimbrel_llm_answer(handle, q, m, contextLimit, h, r)
                    }
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decodeLLMResult(json)
    }

    /// #74: the assembly half of `askQuestion`, for a caller that streams the actual
    /// model call via `InferenceStreamClient` instead of blocking on it — see
    /// `whimbrel_llm_prepare_ask`'s doc comment. Same retrieval/spend-cap/history/routing
    /// semantics as `askQuestion`, just split so the (fast, DB-only) assembly and the
    /// (slow, network) generation can run through different execution paths.
    func prepareAsk(
        _ query: String, contextLimit: Int = 5, history: [CoreChatTurn] = [], routing: CoreRoutingConfig
    ) -> Result<CorePrepareAskData, CoreLLMError>? {
        guard let historyJSON = try? JSONEncoder().encode(history),
              let historyString = String(data: historyJSON, encoding: .utf8) else { return nil }
        guard let routingJSON = try? JSONEncoder().encode(routing),
              let routingString = String(data: routingJSON, encoding: .utf8) else { return nil }
        guard let json = query.withCString({ q in
            historyString.withCString { h in
                routingString.withCString { r in
                    whimbrel_llm_prepare_ask(handle, q, contextLimit, h, r)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decodeLLMResult(json)
    }

    /// The meeting-scoped counterpart of `prepareAsk`: the prompt is built from one
    /// meeting's transcript. Finish with `finalizeAsk` / `discardAsk` as for `prepareAsk`.
    func prepareMeetingAsk(
        meetingId: Int64, query: String, history: [CoreChatTurn] = [], routing: CoreRoutingConfig
    ) -> Result<CorePrepareMeetingAskData, CoreLLMError>? {
        guard let historyJSON = try? JSONEncoder().encode(history),
              let historyString = String(data: historyJSON, encoding: .utf8) else { return nil }
        guard let routingJSON = try? JSONEncoder().encode(routing),
              let routingString = String(data: routingJSON, encoding: .utf8) else { return nil }
        guard let json = query.withCString({ q in
            historyString.withCString { h in
                routingString.withCString { r in
                    whimbrel_llm_prepare_meeting_ask(handle, meetingId, q, h, r)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decodeLLMResult(json)
    }

    /// #74: the citation-extraction half, called once a streaming run started from
    /// `prepareAsk`'s prompt reaches a `.completed` terminal event — see
    /// `whimbrel_llm_finalize_ask`'s doc comment. Pass -1 for either token count to omit
    /// it. Returns `[]` (not `nil`) for an unknown/already-finalized `prepareId` — see
    /// that function's doc comment for why. `nil` only on genuinely invalid usage.
    func finalizeAsk(prepareId: String, finalText: String, model: String, inputTokens: Int64, outputTokens: Int64) -> [CoreContextItemRef]? {
        guard let json = prepareId.withCString({ p in
            finalText.withCString { t in
                model.withCString { m in
                    whimbrel_llm_finalize_ask(handle, p, t, m, inputTokens, outputTokens)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data: CoreFinalizeAskData? = decode(json)
        return data?.citations
    }

    /// #74: discards a prepared ask's stashed context without extracting citations —
    /// call this on a cancelled or failed-with-no-output streaming run, since
    /// `finalizeAsk` (the only other remover) never runs on those paths. See
    /// `whimbrel_llm_discard_ask`'s doc comment.
    func discardAsk(prepareId: String) {
        prepareId.withCString { p in
            whimbrel_llm_discard_ask(p)
        }
    }

    /// FR-MEM-3: end-of-day digest over a time range (e.g. today's start/end in local
    /// time). `model` is a local Ollama model name. `routing` — see `askQuestion`'s doc
    /// comment. Returns `nil` only on genuinely invalid usage (null handle, malformed
    /// args) — otherwise (LB0-5) a `Result` distinguishing success from
    /// `.spendCap`/`.providerFailure`.
    func dailyDigest(model: String, startUnixMs: Int64, endUnixMs: Int64, routing: CoreRoutingConfig) -> Result<CoreDailyDigest, CoreLLMError>? {
        guard let routingJSON = try? JSONEncoder().encode(routing),
              let routingString = String(data: routingJSON, encoding: .utf8) else { return nil }
        guard let json = model.withCString({ m in
            routingString.withCString { r in
                whimbrel_llm_daily_digest(handle, m, startUnixMs, endUnixMs, r)
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decodeLLMResult(json)
    }

    /// FR-RTN-1..4: called once per `RoutineScheduler` tick (Task 6). `model` is a local
    /// Ollama model name, only actually used if some routine is due this call. Returns
    /// the routines that ran and completed THIS call (usually empty). Returns `nil` only
    /// on genuinely invalid usage (null handle, malformed args); a well-formed empty
    /// result is `[]`, not `nil`.
    func routinesTick(
        nowUnixMs: Int64, ollamaModel: String, routing: CoreRoutingConfig, todayEvents: [CoreCalendarEventSummary],
        gmailConnected: Bool, recipientEmail: String?, gate: RoutineGate
    ) -> [CoreRoutineRun]? {
        guard let routingJSON = try? JSONEncoder().encode(routing),
              let routingString = String(data: routingJSON, encoding: .utf8) else { return nil }
        guard let eventsJSON = try? JSONEncoder().encode(todayEvents),
              let eventsString = String(data: eventsJSON, encoding: .utf8) else { return nil }
        guard let json = ollamaModel.withCString({ m in
            routingString.withCString { r in
                eventsString.withCString { e in
                    if let recipientEmail {
                        return recipientEmail.withCString { re in
                            whimbrel_routines_tick(handle, nowUnixMs, m, r, e, gmailConnected, re, gate.rawValue)
                        }
                    }
                    return whimbrel_routines_tick(handle, nowUnixMs, m, r, e, gmailConnected, nil, gate.rawValue)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        guard let envelope: CoreLLMEnvelope<[CoreRoutineRun]> = decode(json), envelope.ok else { return nil }
        return envelope.data ?? []
    }

    /// Prepares briefs for upcoming meetings whose lead window has opened (Meeting Prep).
    /// Returns the routine runs it created (empty when the routine is off or nothing is
    /// due), or nil on a malformed call.
    func meetingPrepRun(
        nowUnixMs: Int64, leadMinutes: Double, ollamaModel: String, routing: CoreRoutingConfig, meetings: [QualifyingMeeting],
        gate: RoutineGate
    ) -> [CoreRoutineRun]? {
        guard let routingJSON = try? JSONEncoder().encode(routing), let routingString = String(data: routingJSON, encoding: .utf8),
              let meetingsJSON = try? JSONEncoder().encode(meetings), let meetingsString = String(data: meetingsJSON, encoding: .utf8)
        else { return nil }
        guard let json = ollamaModel.withCString({ m in
            routingString.withCString { r in
                meetingsString.withCString { u in
                    whimbrel_meeting_prep_run(handle, nowUnixMs, leadMinutes, m, r, u, gate.rawValue)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        guard let envelope: CoreLLMEnvelope<[CoreRoutineRun]> = decode(json), envelope.ok else { return nil }
        return envelope.data ?? []
    }

    /// Away Mode could not attend a meeting: records the to-do and, when the organizer is known and
    /// is not the user, files the organizer email as a pending approval. Nil on a malformed call
    /// or a database error.
    func recordAwayFollowup(_ req: AwayFollowupRequest) -> AwayFollowupResult? {
        guard let body = try? JSONEncoder().encode(req), let bodyString = String(data: body, encoding: .utf8),
              let json = bodyString.withCString({ whimbrel_away_record_followup(handle, $0) })
        else { return nil }
        defer { whimbrel_core_free_string(json) }
        guard let envelope: CoreLLMEnvelope<AwayFollowupResult> = decode(json), envelope.ok else { return nil }
        return envelope.data
    }

    /// Follow-ups not yet done, oldest first.
    func openFollowups() -> [CoreMeetingFollowup] {
        guard let json = whimbrel_memory_open_followups(handle) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    func completeFollowup(id: Int64) -> Bool {
        whimbrel_memory_complete_followup(handle, id, Int64(Date().timeIntervalSince1970 * 1000))
    }

    /// The stored brief for a calendar event id, or nil.
    func meetingPrep(eventId: String) -> CoreMeetingPrep? {
        guard let json = eventId.withCString({ whimbrel_meeting_prep_get(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// FR-RTN-5..7 (Phase B approval gate): every routine run currently sitting in
    /// `PendingApproval`, newest first, each with its routine's name joined in (bare
    /// JSON, not an LB0-5 envelope). Note the underlying JSON also carries
    /// `routine_name` (`whimbrel_routines_pending_approvals`'s own doc comment) --
    /// this decode target drops it since `CoreRoutineRun` has no such field; a future
    /// caller that wants the name should decode into `CoreRoutineRunWithName` instead.
    /// Returns `nil` on a null/invalid handle or a DB error.
    func pendingApprovals() -> [CoreRoutineRun]? {
        guard let json = whimbrel_routines_pending_approvals(handle) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// M3.0 Task 7: atomically claims `runId`'s drafted action and builds the connector
    /// call (core/api::write_action::plan). Make the connector call yourself, off the
    /// store queue, then `settleWriteAction`. Nil on a null handle or DB error.
    func planWriteAction(runId: Int64) -> CoreWriteActionPlanResult? {
        guard let json = whimbrel_routines_plan_write_action(handle, runId) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Envelope: Decodable {
            let status: String
            let plan: CoreWriteActionPlan?
        }
        guard let envelope: Envelope = decode(json) else { return nil }
        switch envelope.status {
        case "not_claimable": return .notClaimable
        case "undecodable": return .undecodable
        case "planned":
            guard let plan = envelope.plan else { return nil }
            return .planned(plan)
        default: return nil
        }
    }

    /// Records the connector call's outcome for a planned run: `success` and
    /// `terminalFailure` finalize it, `retryableFailure` and `noToken` release it back to
    /// pending approval so the draft survives. Nil on a null handle, malformed JSON or
    /// DB error.
    func settleWriteAction(runId: Int64, plan: CoreWriteActionPlan, outcome: CoreWriteActionOutcome) -> CoreWriteActionSettled? {
        guard let planData = try? JSONEncoder().encode(plan),
              let planJSON = String(data: planData, encoding: .utf8) else { return nil }
        guard let json = planJSON.withCString({ p in
            outcome.json.withCString { o in whimbrel_routines_settle_write_action(handle, runId, p, o) }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Called by Swift after it has actually executed a pending write action (or the
    /// execution genuinely failed) -- `success` distinguishes which. Returns `false` on
    /// a null/invalid handle or DB error.
    func finalizeRoutineRun(id: Int64, success: Bool, outputSummary: String, outputJson: String) -> Bool {
        outputSummary.withCString { s in
            outputJson.withCString { j in
                whimbrel_routines_finalize_run(handle, id, success, s, j)
            }
        }
    }

    /// A user's deliberate "no" on a pending write action -- discards the draft, no
    /// execution ever attempted. Returns `false` on a null/invalid handle or DB error.
    func rejectRoutineRun(id: Int64) -> Bool {
        whimbrel_routines_reject_run(handle, id)
    }

    /// Reads back one pending run's draft write-action payload (the JSON Swift actually
    /// executes against Calendar/Gmail) WITHOUT claiming it, for display only. Returns
    /// `nil` if the handle is null/invalid, the run doesn't exist, or it has no pending
    /// action (already finalized/rejected).
    func pendingActionJson(forRunId id: Int64) -> String? {
        guard let json = whimbrel_routines_pending_action_json(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return String(cString: json)
    }

    /// Toggles whether a routine's write-step drafts are auto-approved (skipping the
    /// approvals inbox). Returns `false` on a null/invalid handle or DB error.
    func setRoutineAutoApprove(id: Int64, autoApprove: Bool) -> Bool {
        whimbrel_routines_set_auto_approve(handle, id, autoApprove)
    }

    /// FR-RTN-1..4 Settings support: lists every routine (bare JSON, not an LB0-5
    /// envelope). Returns `nil` only on a null/invalid handle or a DB error.
    func listRoutines() -> [CoreRoutine]? {
        guard let json = whimbrel_routines_list(handle) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// FR-RTN-8 final-review fix: the most recent routine runs across every routine,
    /// newest first, each with its routine's name joined in (bare JSON, not an LB0-5
    /// envelope). This is `RoutinesInboxView`'s real persisted-history source — unlike
    /// `RoutineScheduler.recentRuns` (in-memory only), this survives an app relaunch.
    /// Returns `nil` only on a null/invalid handle or a DB error.
    func recentRoutineRuns(limit: Int) -> [CoreRoutineRunWithName]? {
        guard let json = whimbrel_routines_recent_runs(handle, UInt64(limit)) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Up to `limit` most recent episodes, newest first (see `Store::recent_episodes`).
    /// Returns `nil` only on a null/invalid handle or a DB error.
    func recentEpisodes(limit: Int) -> [CoreEpisode]? {
        guard let json = whimbrel_memory_recent_episodes(handle, UInt64(limit)) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Fetches one episode by id, fresh from the store — used by the per-episode
    /// detail window instead of relying on a value passed in at window-open time.
    func getEpisode(id: Int64) -> CoreEpisode? {
        guard let json = whimbrel_memory_get_episode(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Shows/hides one episode from the default episode list without deleting
    /// anything. Returns whether the update succeeded.
    @discardableResult
    func setEpisodeHidden(id: Int64, hidden: Bool) -> Bool {
        whimbrel_memory_set_episode_hidden(handle, id, hidden, Int64(Date().timeIntervalSince1970 * 1000))
    }

    /// Permanently deletes the episode's underlying source items and the episode
    /// itself. Returns whether a row existed and was deleted.
    @discardableResult
    func forgetEpisodeActivity(id: Int64) -> Bool {
        whimbrel_memory_forget_episode_activity(handle, id)
    }

    /// Resets the episode's existing `episode_summarize` job back to `pending` so
    /// it's picked up again. Returns whether a matching job row existed and was reset.
    @discardableResult
    func regenerateEpisodeSummary(id: Int64) -> Bool {
        whimbrel_memory_regenerate_episode_summary(handle, id, Int64(Date().timeIntervalSince1970 * 1000))
    }

    /// Cross-device hidden-episode marks, already in settings-sync field shape (keys
    /// `episode.hidden.*`) so `SettingsSyncCoordinator` merges them straight into its
    /// push map. Empty on a null/invalid handle or a DB error.
    func hiddenEpisodeSyncFields() -> [String: SyncSettingsField] {
        guard let json = whimbrel_memory_hidden_episode_sync_fields(handle) else { return [:] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? [:]
    }

    /// Applies pulled `episode.hidden.*` fields; the core does its own last-writer-wins
    /// per mark. Returns the number applied, or -1 on failure.
    @discardableResult
    func applyHiddenEpisodeSyncFields(_ fields: [String: SyncSettingsField]) -> Int64 {
        guard let data = try? JSONEncoder().encode(fields),
              let json = String(data: data, encoding: .utf8) else { return -1 }
        return json.withCString { whimbrel_memory_apply_hidden_episode_sync_fields(handle, $0) }
    }

    /// Ids of every hidden episode, regardless of which device captured it. Hidden state
    /// syncs across devices by episode identity (see `hiddenEpisodeSyncFields`).
    /// `deviceId` is accepted for ABI stability but ignored by the core. Returns an empty array on a
    /// null/invalid handle, malformed input, or a DB error.
    func listHiddenEpisodeIds(deviceId: String) -> [Int64] {
        guard let json = deviceId.withCString({ whimbrel_memory_list_hidden_episodes(handle, $0) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    /// Up to `limit` threads needing attention (waiting_on_me/waiting_on_them only —
    /// see `Store::threads_needing_attention`), most recently active first. Returns
    /// `nil` only on a null/invalid handle or a DB error.
    func threadsNeedingAttention(limit: Int) -> [CoreThread]? {
        guard let json = whimbrel_threads_needing_attention(handle, UInt64(limit)) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Fetches one thread by id, fresh from the store — used by the per-thread detail
    /// window instead of relying on a value passed in at window-open time.
    func getThread(id: Int64) -> CoreThread? {
        guard let json = whimbrel_memory_get_thread(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// FR-RTN-1..4 Settings support: enables/disables a routine by id. Returns `false`
    /// on a null/invalid handle, unknown id, or DB error.
    func setRoutineEnabled(id: Int64, enabled: Bool) -> Bool {
        whimbrel_routines_set_enabled(handle, id, enabled)
    }

    /// FR-RTN-1..4 Settings support: updates a routine's HH:MM schedule by id. Returns
    /// `false` on a null/invalid handle, unknown id, or DB error.
    func updateRoutineSchedule(id: Int64, scheduleHhmm: String) -> Bool {
        scheduleHhmm.withCString { whimbrel_routines_set_schedule(handle, id, $0) }
    }

    /// Sets which model chain a routine's runs use: `default`, `cheapest`, or
    /// `<provider>/<model>`. Returns `false` on a null/invalid handle, a value outside
    /// those shapes, or a DB error.
    func setRoutineModelPref(id: Int64, pref: String) -> Bool {
        pref.withCString { whimbrel_routines_set_model_pref(handle, id, $0) }
    }

    /// The Model usage screen's totals (core/api::usage::totals). Nil on a null/invalid
    /// handle.
    func usageTotals() -> CoreUsageTotals? {
        guard let json = whimbrel_usage_totals(handle) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// The newest `limit` calls with `since <= ts < until` (core/api::usage::list),
    /// newest first. Nil on a null/invalid handle.
    func usageList(sinceUnixMs: Int64, untilUnixMs: Int64, limit: UInt32) -> [CoreUsageRow]? {
        guard let json = whimbrel_usage_list(handle, sinceUnixMs, untilUnixMs, limit) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// FR-CHAT-2 governance: today's spend-so-far. `userId` may be nil (Whimbrel's
    /// typical single-device "anonymous" identity); `globalCents` always reflects total
    /// spend across all identities. Returns nil on a null/invalid handle.
    func spendToday(userId: String? = nil) -> CoreSpendToday? {
        let json: UnsafeMutablePointer<CChar>?
        if let userId {
            json = userId.withCString { whimbrel_llm_spend_today(handle, $0) }
        } else {
            json = whimbrel_llm_spend_today(handle, nil)
        }
        guard let json else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// FR-MTG-1/2: records a meeting (a qualifying calendar event or a heuristically
    /// detected one). Returns the new row's id, or nil on failure.
    func insertMeeting(_ meeting: CoreNewMeeting) -> Int64? {
        guard let json = try? JSONEncoder().encode(meeting), let jsonString = String(data: json, encoding: .utf8) else {
            return nil
        }
        guard let resultJSON = jsonString.withCString({ whimbrel_memory_insert_meeting(handle, $0) }) else {
            return nil
        }
        defer { whimbrel_core_free_string(resultJSON) }
        struct IdResult: Decodable { let id: Int64 }
        return decode(resultJSON).map { (result: IdResult) in result.id }
    }

    /// Updates a meeting's status only. Returns whether the update succeeded.
    @discardableResult
    func updateMeetingStatus(id: Int64, status: CoreMeetingStatus) -> Bool {
        status.rawValue.withCString { whimbrel_memory_update_meeting_status(handle, id, $0) }
    }

    /// Re-roots stored meeting audio/transcript paths after the data folder moved.
    /// Returns the number of meetings changed, or -1 on failure.
    func replaceMeetingPathPrefix(old: String, new: String) -> Int64 {
        old.withCString { o in new.withCString { n in whimbrel_memory_replace_meeting_path_prefix(handle, o, n) } }
    }

    /// Sets a meeting's title (synced). Returns whether it was updated.
    @discardableResult
    func setMeetingTitle(id: Int64, title: String) -> Bool {
        title.withCString { whimbrel_memory_set_meeting_title(handle, id, $0) }
    }

    /// Sets one meeting's estimated_speaker_count (the heuristic speaker-count hint).
    /// Returns whether the update succeeded.
    @discardableResult
    func setMeetingEstimatedSpeakerCount(id: Int64, count: Int) -> Bool {
        whimbrel_memory_set_meeting_estimated_speaker_count(handle, id, Int64(count))
    }

    /// Records an Away Mode outcome on a meeting. `at` is stored only for `.removed`.
    @discardableResult
    func setMeetingAwayOutcome(meetingId: Int64, outcome: AwayOutcome, at: Date) -> Bool {
        outcome.rawValue.withCString {
            whimbrel_memory_set_meeting_away_outcome(handle, meetingId, $0, Int64(at.timeIntervalSince1970 * 1000))
        }
    }

    /// Permanently deletes one meeting's database row. Returns whether a row existed
    /// and was deleted — the caller is responsible for deleting the meeting's
    /// audio/transcript files separately (see `deleteMeetingFiles`); this method only
    /// touches the Rust core's metadata store, never the filesystem.
    @discardableResult
    func deleteMeeting(id: Int64) -> Bool {
        whimbrel_memory_delete_meeting(handle, id)
    }

    // MARK: Meeting parts (continue a meeting within the window)

    /// Adds the next part and returns its 1-based index. The first part also sets the
    /// meeting's continue key.
    func beginMeetingPart(meetingId: Int64, startUnixMs: Int64, continueKey: String?) -> Int64? {
        let json = Self.withOptionalCString(continueKey) { whimbrel_meeting_begin_part(handle, meetingId, startUnixMs, $0) }
        guard let json else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct R: Decodable { let part_index: Int64 }
        return decode(json).map { (r: R) in r.part_index }
    }

    /// The completed meeting with `continueKey` that ended within the continue window (core's
    /// 10 minutes, or `windowMs`), if any.
    func findContinuableMeeting(continueKey: String, nowUnixMs: Int64, windowMs: Int64? = nil) -> Int64? {
        guard let json = continueKey.withCString({ key in
            windowMs.map { whimbrel_meeting_find_continuable_within(handle, key, nowUnixMs, $0) }
                ?? whimbrel_meeting_find_continuable(handle, key, nowUnixMs)
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct R: Decodable { let id: Int64? }
        return decode(json).flatMap { (r: R) in r.id }
    }

    /// Puts a completed meeting back to `recording`.
    @discardableResult
    func reopenMeeting(id: Int64) -> Bool {
        whimbrel_meeting_reopen(handle, id)
    }

    /// Undoes `reopenMeeting` and the part begun after it (never part 1): the meeting is
    /// completed again with its earlier end.
    @discardableResult
    func undoMeetingReopen(meetingId: Int64, partIndex: Int64) -> Bool {
        whimbrel_meeting_undo_reopen(handle, meetingId, partIndex)
    }

    @discardableResult
    func completeMeetingPart(meetingId: Int64, partIndex: Int64, endUnixMs: Int64, audioPath: String?) -> Bool {
        Self.withOptionalCString(audioPath) { whimbrel_meeting_complete_part(handle, meetingId, partIndex, endUnixMs, $0) }
    }

    /// Writes only the meeting's notes (nil clears them).
    @discardableResult
    func setMeetingNotes(meetingId: Int64, notes: String?) -> Bool {
        Self.withOptionalCString(notes) { whimbrel_meeting_set_notes(handle, meetingId, $0) }
    }

    @discardableResult
    func setMeetingPartTranscript(meetingId: Int64, partIndex: Int64, transcriptPath: String) -> Bool {
        transcriptPath.withCString { whimbrel_meeting_set_part_transcript(handle, meetingId, partIndex, $0) }
    }

    /// Parts in order. A legacy meeting with no part rows yields one synthesized part.
    func meetingParts(meetingId: Int64) -> [CoreMeetingPart]? {
        guard let json = whimbrel_meeting_parts(handle, meetingId) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Replaces this device's segments for one part (`start_cs` is part-relative). Returns
    /// the number inserted, or nil on failure.
    func replacePartTranscriptSegments(meetingId: Int64, partIndex: Int64, segments: [NewTranscriptSegmentInput]) -> Int? {
        guard let data = try? JSONEncoder().encode(segments) else { return nil }
        guard let json = String(decoding: data, as: UTF8.self).withCString({ whimbrel_meeting_replace_part_segments(handle, meetingId, partIndex, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct R: Decodable { let inserted: Int }
        return decode(json).map { (r: R) in r.inserted }
    }

    /// The whole-meeting transcript text, with a divider before each part after the first.
    func meetingTranscriptText(meetingId: Int64) -> String? {
        guard let text = whimbrel_meeting_transcript_text(handle, meetingId) else { return nil }
        defer { whimbrel_core_free_string(text) }
        return String(cString: text)
    }

    /// Splits a completed meeting at `fromPartIndex` (>= 2) into a new meeting.
    func splitMeeting(meetingId: Int64, fromPartIndex: Int64) -> CoreSplitResult? {
        guard let json = whimbrel_meeting_split(handle, meetingId, fromPartIndex) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// After moving snapshot files on disk: repoints the meeting's snapshot rows and records their folder.
    @discardableResult
    func setMeetingSnapshotPaths(meetingId: Int64, moves: [(old: String, new: String)], snapshotsDir: String) -> Bool {
        guard let data = try? JSONEncoder().encode(moves.map { [$0.old, $0.new] }) else { return false }
        return String(decoding: data, as: UTF8.self).withCString { movesPtr in
            snapshotsDir.withCString { whimbrel_meeting_set_snapshot_paths(handle, meetingId, movesPtr, $0) }
        }
    }

    /// After moving or compressing audio on disk: rewrites `meeting.audio_path` and every
    /// part's that equals an `old`. Local only. False, with nothing changed, when any `old`
    /// matches no stored path: the caller must then keep the originals.
    func setMeetingAudioPaths(meetingId: Int64, moves: [(old: String, new: String)]) -> Bool {
        guard let data = try? JSONEncoder().encode(moves.map { [$0.old, $0.new] }) else { return false }
        return String(decoding: data, as: UTF8.self).withCString { whimbrel_meeting_set_audio_paths(handle, meetingId, $0) }
    }

    /// After a path rewrite: true only once the commit is on stable storage (FULL WAL
    /// checkpoint with F_FULLFSYNC). On false the rows may already point at the new files,
    /// so delete nothing: neither the originals nor the new files.
    func checkpointDurable() -> Bool {
        whimbrel_store_checkpoint(handle)
    }

    /// Storage retention: what to move, compress, delete or shrink (core `media_retention::plan`).
    /// `busy` meetings get no work. Nil on error.
    func mediaPlan(policy: CoreMediaPolicy, nowUnixMs: Int64, mediaRoot: URL, busy: Set<Int64>) -> CoreMediaPlan? {
        guard let policyData = try? JSONEncoder().encode(policy),
              let busyData = try? JSONEncoder().encode(busy.sorted()) else { return nil }
        let json = String(decoding: policyData, as: UTF8.self).withCString { policyPtr in
            mediaRoot.path.withCString { rootPtr in
                String(decoding: busyData, as: UTF8.self).withCString { whimbrel_media_plan(handle, policyPtr, nowUnixMs, rootPtr, $0) }
            }
        }
        guard let json else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Deletes a meeting's row and returns every part's files plus the snapshot folder
    /// (nil if another meeting still has snapshots in it). Nil when there was no such
    /// meeting. Delete the files with `deleteAllMeetingFiles`.
    func deleteMeetingReturningFiles(id: Int64) -> CoreDeletedMeetingFiles? {
        guard let json = whimbrel_meeting_delete_returning_files(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    // MARK: Meeting snapshots (slide images; device-local)

    /// Returns the new snapshot id, or nil on failure or a missing meeting.
    func insertMeetingSnapshot(meetingId: Int64, capturedUnixMs: Int64, filePath: String, width: Int, height: Int, ocrText: String) -> Int64? {
        struct In: Encodable {
            let meetingId, capturedUnixMs: Int64
            let filePath: String
            let width, height: Int
            let ocrText: String
            enum CodingKeys: String, CodingKey {
                case meetingId = "meeting_id", capturedUnixMs = "captured_unix_ms", filePath = "file_path", width, height, ocrText = "ocr_text"
            }
        }
        guard let json = try? JSONEncoder().encode(In(meetingId: meetingId, capturedUnixMs: capturedUnixMs, filePath: filePath, width: width, height: height, ocrText: ocrText)) else { return nil }
        let id = String(decoding: json, as: UTF8.self).withCString { whimbrel_memory_insert_meeting_snapshot(handle, $0) }
        return id < 0 ? nil : id
    }

    @discardableResult
    func replaceMeetingSnapshot(id: Int64, filePath: String, width: Int, height: Int, ocrText: String) -> Bool {
        let obj: [String: Any] = ["file_path": filePath, "width": width, "height": height, "ocr_text": ocrText]
        guard let json = try? JSONSerialization.data(withJSONObject: obj) else { return false }
        return String(decoding: json, as: UTF8.self).withCString { whimbrel_memory_replace_meeting_snapshot(handle, id, $0) }
    }

    /// Deletes the row and returns its file path (the caller removes the file), or nil if no such row.
    @discardableResult
    func deleteMeetingSnapshot(id: Int64) -> String? {
        guard let p = whimbrel_memory_delete_meeting_snapshot(handle, id) else { return nil }
        defer { whimbrel_core_free_string(p) }
        return String(cString: p)
    }

    /// Slide OCR text for the summary prompt: non-blank texts joined by blank lines.
    /// Truncation is the core's job (`cap_slides`). Touches the store synchronously, so
    /// call it on the queue that owns this handle.
    func meetingSlidesText(meetingId: Int64) -> String {
        listMeetingSnapshots(meetingId: meetingId)
            .map { $0.ocrText.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n\n")
    }

    /// A meeting's snapshots, oldest first. Rows whose image file no longer exists are
    /// dropped and deleted so the strip never shows broken slides, but only when the file's
    /// folder is still there: a missing folder (unmounted volume, unreachable root) says
    /// nothing about the file, so those rows are kept as they are.
    func listMeetingSnapshots(meetingId: Int64) -> [CoreMeetingSnapshot] {
        guard let json = whimbrel_memory_list_meeting_snapshots(handle, meetingId) else { return [] }
        defer { whimbrel_core_free_string(json) }
        let all: [CoreMeetingSnapshot] = decode(json) ?? []
        let fm = FileManager.default
        return all.filter { snap in
            let file = URL(fileURLWithPath: snap.filePath)
            if fm.fileExists(atPath: file.path) || !fm.fileExists(atPath: file.deletingLastPathComponent().path) { return true }
            deleteMeetingSnapshot(id: snap.id)
            return false
        }
    }

    func setMeetingSnapshotsDir(meetingId: Int64, dir: String) {
        _ = dir.withCString { whimbrel_memory_set_meeting_snapshots_dir(handle, meetingId, $0) }
    }

    func setKeepSnapshots(meetingId: Int64, keep: Bool) {
        _ = whimbrel_memory_set_keep_snapshots(handle, meetingId, keep)
    }

    /// Sets the synced "Keep media" flag. Returns whether a row changed.
    func setKeepMedia(meetingId: Int64, keep: Bool) -> Bool {
        whimbrel_memory_set_keep_media(handle, meetingId, keep)
    }

    /// nil when the meeting does not exist.
    func meetingSnapshotsInfo(meetingId: Int64) -> (dir: String?, keep: Bool)? {
        guard let json = whimbrel_memory_meeting_snapshots_info(handle, meetingId) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Info: Decodable { let dir: String?; let keep: Bool }
        return decode(json).map { (i: Info) in (i.dir, i.keep) }
    }

    /// Marks a meeting completed with its actual end time and (if captured) the audio
    /// file path. Returns whether the update succeeded.
    @discardableResult
    func completeMeeting(id: Int64, endUnixMs: Int64, audioPath: String?) -> Bool {
        if let audioPath {
            return audioPath.withCString { whimbrel_memory_complete_meeting(handle, id, endUnixMs, $0) }
        }
        return whimbrel_memory_complete_meeting(handle, id, endUnixMs, nil)
    }

    /// FR-MTG-5: records the transcript file path and captured notes for a completed
    /// meeting. `notes` may be nil (no notes were taken). Returns whether the update
    /// succeeded.
    @discardableResult
    func setMeetingTranscript(id: Int64, transcriptPath: String, notes: String?) -> Bool {
        transcriptPath.withCString { pathPtr in
            if let notes {
                return notes.withCString { notesPtr in
                    whimbrel_memory_set_meeting_transcript(handle, id, pathPtr, notesPtr)
                }
            }
            return whimbrel_memory_set_meeting_transcript(handle, id, pathPtr, nil)
        }
    }

    /// Fetches one meeting's editable-overlay draft, or nil if nothing has been edited yet.
    func getMeetingNoteDraft(meetingId: Int64) -> CoreMeetingNoteDraft? {
        guard let json = whimbrel_memory_get_meeting_note_draft(handle, meetingId) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Autosaves the During-state live note field. Returns whether the write succeeded.
    @discardableResult
    func savePersonalNotes(meetingId: Int64, notes: String, nowUnixMs: Int64) -> Bool {
        notes.withCString { whimbrel_memory_save_meeting_personal_notes(handle, meetingId, $0, nowUnixMs) }
    }

    /// Writes structured overrides for Summary/Decisions/Open-questions. Pass nil for
    /// any section not being edited right now -- it's left untouched. Returns whether
    /// the write succeeded.
    @discardableResult
    func saveNoteOverrides(meetingId: Int64, summaryEdited: String?, decisionsEdited: [String]?, openQuestionsEdited: [String]?, nowUnixMs: Int64) -> Bool {
        struct Overrides: Encodable {
            let summaryEdited: String?
            let decisionsEdited: [String]?
            let openQuestionsEdited: [String]?
            enum CodingKeys: String, CodingKey {
                case summaryEdited = "summary_edited"
                case decisionsEdited = "decisions_edited"
                case openQuestionsEdited = "open_questions_edited"
            }
        }
        let overrides = Overrides(summaryEdited: summaryEdited, decisionsEdited: decisionsEdited, openQuestionsEdited: openQuestionsEdited)
        guard let json = try? JSONEncoder().encode(overrides), let jsonString = String(data: json, encoding: .utf8) else { return false }
        return jsonString.withCString { whimbrel_memory_save_meeting_note_overrides(handle, meetingId, $0, nowUnixMs) }
    }

    /// "Replace editable draft" -- discards structured overrides, keeps personal notes.
    /// Returns whether the write succeeded.
    @discardableResult
    func resetMeetingNoteDraft(meetingId: Int64, nowUnixMs: Int64) -> Bool {
        whimbrel_memory_reset_meeting_note_draft(handle, meetingId, nowUnixMs)
    }

    /// Records that the user has reconciled with the generation produced at
    /// `regeneratedUnixMs`. Returns whether the write succeeded.
    @discardableResult
    func markMeetingRegenerationSeen(meetingId: Int64, regeneratedUnixMs: Int64) -> Bool {
        whimbrel_memory_mark_meeting_regeneration_seen(handle, meetingId, regeneratedUnixMs)
    }

    /// All action items for a meeting, ordered for display.
    func listMeetingActionItems(meetingId: Int64) -> [CoreMeetingActionItem] {
        guard let json = whimbrel_memory_list_meeting_action_items(handle, meetingId) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    /// Seeds generated action items from a fresh summary. Idempotent. Returns whether
    /// the write succeeded.
    @discardableResult
    func seedMeetingActionItems(meetingId: Int64, texts: [String], nowUnixMs: Int64) -> Bool {
        guard let json = try? JSONEncoder().encode(texts), let jsonString = String(data: json, encoding: .utf8) else { return false }
        return jsonString.withCString { whimbrel_memory_seed_meeting_action_items(handle, meetingId, $0, nowUnixMs) }
    }

    /// Adds a user-authored action item. Returns the new row's id, or nil on failure.
    func insertMeetingActionItem(meetingId: Int64, text: String, orderIndex: Int64, nowUnixMs: Int64) -> Int64? {
        guard let resultJSON = text.withCString({ whimbrel_memory_insert_meeting_action_item(handle, meetingId, $0, orderIndex, nowUnixMs) }) else { return nil }
        defer { whimbrel_core_free_string(resultJSON) }
        struct IdResult: Decodable { let id: Int64 }
        return decode(resultJSON).map { (result: IdResult) in result.id }
    }

    @discardableResult
    func updateMeetingActionItemText(id: Int64, text: String, nowUnixMs: Int64) -> Bool {
        text.withCString { whimbrel_memory_update_meeting_action_item_text(handle, id, $0, nowUnixMs) }
    }

    @discardableResult
    func setMeetingActionItemCompleted(id: Int64, completed: Bool, nowUnixMs: Int64) -> Bool {
        whimbrel_memory_set_meeting_action_item_completed(handle, id, completed, nowUnixMs)
    }

    @discardableResult
    func deleteMeetingActionItem(id: Int64) -> Bool {
        whimbrel_memory_delete_meeting_action_item(handle, id)
    }

    /// Applies a new display order. `orderedIds` must contain exactly the meeting's
    /// current item ids, in their new order.
    @discardableResult
    func reorderMeetingActionItems(meetingId: Int64, orderedIds: [Int64], nowUnixMs: Int64) -> Bool {
        guard let json = try? JSONEncoder().encode(orderedIds), let jsonString = String(data: json, encoding: .utf8) else { return false }
        return jsonString.withCString { whimbrel_memory_reorder_meeting_action_items(handle, meetingId, $0, nowUnixMs) }
    }

    /// "Replace editable draft" for action items -- drops stale generated items, keeps
    /// user-added ones, appends fresh generated items. Returns whether the write succeeded.
    @discardableResult
    func replaceMeetingActionItemsFromGenerated(meetingId: Int64, texts: [String], nowUnixMs: Int64) -> Bool {
        guard let json = try? JSONEncoder().encode(texts), let jsonString = String(data: json, encoding: .utf8) else { return false }
        return jsonString.withCString { whimbrel_memory_replace_meeting_action_items_from_generated(handle, meetingId, $0, nowUnixMs) }
    }

    /// FR-CHAT-1 follow-on: inserts a batch of transcript segments for one meeting.
    /// Returns whether the insert succeeded.
    @discardableResult
    func insertTranscriptSegments(meetingId: Int64, segments: [NewTranscriptSegmentInput]) -> Bool {
        guard let segmentsJSON = try? JSONEncoder().encode(segments),
              let segmentsString = String(data: segmentsJSON, encoding: .utf8) else { return false }
        return segmentsString.withCString { whimbrel_memory_insert_transcript_segments(handle, meetingId, $0) }
    }

    /// docs/superpowers/specs/2026-08-07-items-schema-migration-design.md: upserts
    /// calendar-event items, keyed on (source, external_id) so repeated polling doesn't
    /// duplicate rows. Returns whether the upsert succeeded.
    @discardableResult
    func upsertCalendarEventItems(_ items: [NewCalendarEventItemInput]) -> Bool {
        guard !items.isEmpty else { return true }
        guard let itemsJSON = try? JSONEncoder().encode(items),
              let itemsString = String(data: itemsJSON, encoding: .utf8) else { return false }
        return itemsString.withCString { whimbrel_memory_upsert_calendar_event_items(handle, $0) }
    }

    /// FR-MTG-5: generates a meeting summary by blending `notes` with `transcript`,
    /// through the same governance pipeline as `askQuestion`/`dailyDigest`. On success
    /// this also updates the meeting row's summary fields on the Rust side — this
    /// return value is for immediately reflecting the result in the UI without a
    /// separate re-fetch. Returns `nil` only on genuinely invalid usage (null handle,
    /// malformed args) — otherwise (LB0-5) a `Result` distinguishing success from
    /// `.spendCap`/`.providerFailure` (a cap rejection can happen either before the
    /// first provider is tried or mid-operation on a later chunk).
    func summarizeMeeting(meetingId: Int64, notes: String, transcript: String, slides: String, model: String, routing: CoreRoutingConfig, customInstructions: String) -> Result<CoreMeetingSummary, CoreLLMError>? {
        guard let routingJSON = try? JSONEncoder().encode(routing),
              let routingString = String(data: routingJSON, encoding: .utf8) else { return nil }
        guard let json = notes.withCString({ n in
            transcript.withCString { t in
                slides.withCString { sl in
                    model.withCString { m in
                        routingString.withCString { r in
                            customInstructions.withCString { ci in
                                whimbrel_llm_summarize_meeting(handle, meetingId, n, t, sl, m, r, ci)
                            }
                        }
                    }
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decodeLLMResult(json)
    }

    /// Most recent meetings, newest first.
    /// `recentMeetings` for display: copies of one call collapse into one entry whose
    /// `alsoRecorded` lists the others.
    func recentMeetingsGrouped(limit: Int = 20) -> [CoreMeeting] {
        guard let json = whimbrel_meetings_recent_grouped(handle, limit) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return decode(json) ?? []
    }

    func recentMeetings(limit: Int = 20) -> [CoreMeeting] {
        guard let json = whimbrel_memory_recent_meetings(handle, limit) else {
            Self.log.log("recentMeetings: FFI call returned nil")
            return []
        }
        defer { whimbrel_core_free_string(json) }
        let result: [CoreMeeting]? = decode(json)
        if result == nil {
            let raw = String(cString: json)
            Self.log.log("recentMeetings: decode FAILED, raw json prefix: \(String(raw.prefix(500)), privacy: .private)")
        } else {
            Self.log.log("recentMeetings: decoded \(result?.count ?? -1, privacy: .public) meetings")
        }
        return result ?? []
    }

    /// Captures per device over `[start, end)`, busiest first. Empty on any failure.
    func captureActivityByDevice(startUnixMs: Int64, endUnixMs: Int64) -> [CoreDeviceActivity] {
        guard let json = whimbrel_memory_capture_activity_by_device(handle, startUnixMs, endUnixMs) else { return [] }
        defer { whimbrel_core_free_string(json) }
        let result: [CoreDeviceActivity]? = decode(json)
        return result ?? []
    }

    /// Fetches one meeting by id, fresh from the store — used by the per-meeting
    /// detail window instead of relying on a value passed in at window-open time.
    /// Returns nil if the meeting doesn't exist or on any DB error (indistinguishable,
    /// matching this file's existing null-on-any-failure convention).
    func getMeeting(id: Int64) -> CoreMeeting? {
        guard let json = whimbrel_memory_get_meeting(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Fetches one capture item by id, fresh from the store — used by the per-item
    /// detail window instead of relying on a value passed in at window-open time
    /// (matches `getMeeting`'s exact rationale/pattern). Returns nil if the item
    /// doesn't exist or on any DB error.
    func getCaptureItem(id: Int64) -> CoreCaptureItem? {
        guard let json = whimbrel_memory_get_capture_item(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// A page of a meeting's transcript, ordered by start time — the source of truth for
    /// transcript display on any device. `transcript_segment` rows sync independently of
    /// a meeting's own `transcriptPath` (a local file path that never transfers between
    /// devices), so a device that only ever pulled this meeting via sync still has real
    /// transcript content available here even with no local file. Returns nil on a
    /// null/invalid handle or a DB error.
    func getTranscriptSegmentsPage(meetingId: Int64, offset: Int, limit: Int) -> CoreTranscriptSegmentsPage? {
        guard let json = whimbrel_memory_get_transcript_segments_page(handle, meetingId, offset, limit) else {
            return nil
        }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// FR-MEM-4: per-data-class retention sweep plus file cleanup (core/api::retention,
    /// M3.0 Task 2). `nil` for a day count means "keep that class forever." After the
    /// rows are deleted, each deleted meeting's audio (both mic/system siblings inside
    /// `recordingsDir`) and transcript files are removed best-effort. Returns nil on a
    /// DB error.
    func applyRetention(captureItemDays: Int?, meetingDays: Int?, conversationDays: Int?, nowUnixMs: Int64, recordingsDir: URL) -> CoreRetentionResult? {
        guard let json = recordingsDir.path.withCString({ dir in
            whimbrel_retention_apply(
                handle, Int64(captureItemDays ?? -1), Int64(meetingDays ?? -1), Int64(conversationDays ?? -1), nowUnixMs, dir
            )
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Finds recording files in `recordingsDir` and, when non-nil, `mediaAudioDir` (the
    /// media folder's `Audio`) that no meeting row claims, directly or via the mic/system
    /// sibling rule (core/api::retention::scan_orphans_in). `liveWriteURLs`
    /// are the files a currently-active recording is writing right now, snapshotted by
    /// the caller on the main actor before hopping to `accessQueue`. Deletes nothing:
    /// pass `orphaned` to `deleteOrphanedRecordings` after any confirmation. Returns
    /// nil on a DB error.
    func scanOrphanedRecordings(recordingsDir: URL, mediaAudioDir: URL?, excluding liveWriteURLs: Set<URL>) -> CoreOrphanScan? {
        guard let excludedData = try? JSONEncoder().encode(liveWriteURLs.map(\.path)),
              let json = recordingsDir.path.withCString({ dir in
                  Self.withOptionalCString(mediaAudioDir?.path) { media in
                      String(decoding: excludedData, as: UTF8.self).withCString { excluded in
                          whimbrel_retention_scan_orphans(handle, dir, media, excluded)
                      }
                  }
              }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Permanently deletes `paths` (normally a `CoreOrphanScan.orphaned` array). Returns
    /// how many files were actually removed.
    static func deleteOrphanedRecordings(_ paths: [String]) -> Int {
        guard let data = try? JSONEncoder().encode(paths) else { return 0 }
        return Int(String(decoding: data, as: UTF8.self).withCString { whimbrel_retention_delete_orphans($0) })
    }

    /// Best-effort deletion of one meeting's on-disk files after its row is gone: the
    /// audio's mic/system siblings (resolved inside `recordingsDir`), the transcript and,
    /// when `snapshotsDir` is non-nil, the marked snapshot folder.
    /// A missing file is never an error (it may already be gone, or never existed for a
    /// mic-only degraded session, FR-MTG-10).
    static func deleteMeetingFiles(audioPath: String?, transcriptPath: String?, snapshotsDir: String?, recordingsDir: URL) {
        recordingsDir.path.withCString { dir in
            withOptionalCString(audioPath) { audio in
                withOptionalCString(transcriptPath) { transcript in
                    withOptionalCString(snapshotsDir) { snaps in
                        whimbrel_retention_delete_meeting_files(audio, transcript, snaps, dir)
                    }
                }
            }
        }
    }

    /// Best-effort deletion of every part's on-disk files (audio siblings, transcripts) and the
    /// guarded snapshot folder, from `deleteMeetingReturningFiles`.
    static func deleteAllMeetingFiles(_ files: CoreDeletedMeetingFiles, recordingsDir: URL) {
        guard let data = try? JSONEncoder().encode(files) else { return }
        String(decoding: data, as: UTF8.self).withCString { json in
            recordingsDir.path.withCString { whimbrel_retention_delete_all_meeting_files(json, $0) }
        }
    }

    private static func withOptionalCString<R>(_ value: String?, _ body: (UnsafePointer<CChar>?) -> R) -> R {
        guard let value else { return body(nil) }
        return value.withCString { body($0) }
    }

    /// Mirrors Rust's `DuplicateMeetingGroup` — JSON keys are exact Rust field names
    /// (this file's `decode` uses a plain `JSONDecoder()`, no snake_case conversion).
    struct DuplicateMeetingGroup: Decodable {
        let deviceId: String
        let title: String
        let startUnixMs: Int64
        let endUnixMs: Int64?
        /// Sorted ascending by id — the last entry is the one to KEEP.
        let meetingIds: [Int64]

        enum CodingKeys: String, CodingKey {
            case deviceId = "device_id"
            case title
            case startUnixMs = "start_unix_ms"
            case endUnixMs = "end_unix_ms"
            case meetingIds = "meeting_ids"
        }
    }

    /// Peer-sourced meeting rows still duplicated from before the sync fix in
    /// docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md.
    /// `ownDeviceId` excludes this device's own meetings, which cannot self-duplicate.
    /// Returns nil on a DB error.
    func findDuplicateMeetingGroups(ownDeviceId: String) -> [DuplicateMeetingGroup]? {
        guard let json = ownDeviceId.withCString({ whimbrel_memory_find_duplicate_meeting_groups(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// One meeting with no recorded audio at all (id, title, start time) — for the
    /// Settings "Clean Up Meetings Without Audio" scan/confirm flow.
    struct MeetingWithoutAudioSummary: Decodable, Identifiable {
        let id: Int64
        let title: String
        let startUnixMs: Int64

        enum CodingKeys: String, CodingKey {
            case id, title
            case startUnixMs = "start_unix_ms"
        }
    }

    /// This device's own meetings with no recorded audio at all — excludes meetings still
    /// actively recording (their audio_path legitimately hasn't been stamped yet) and
    /// meetings synced from other devices (they never have local audio). Returns nil on a
    /// DB error.
    func findMeetingsWithoutAudio(ownDeviceId: String) -> [MeetingWithoutAudioSummary]? {
        guard let json = ownDeviceId.withCString({ whimbrel_memory_find_meetings_without_audio(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    struct ConversationSummary: Decodable, Identifiable {
        let id: Int64
        let title: String
        let starred: Bool
        let updatedAtUnixMs: Int64

        enum CodingKeys: String, CodingKey {
            case id, title, starred
            case updatedAtUnixMs = "updated_at_unix_ms"
        }
    }

    struct ConversationExchangeRow: Decodable, Identifiable {
        let id: Int64
        let query: String
        let answer: String
        let citationsJson: String
        let createdAtUnixMs: Int64
        let model: String?

        enum CodingKeys: String, CodingKey {
            case id, query, answer, model
            case citationsJson = "citations_json"
            case createdAtUnixMs = "created_at_unix_ms"
        }
    }

    struct ConversationDetail: Decodable {
        let id: Int64
        let title: String
        let starred: Bool
        let createdAtUnixMs: Int64
        let updatedAtUnixMs: Int64
        let exchanges: [ConversationExchangeRow]

        enum CodingKeys: String, CodingKey {
            case id, title, starred, exchanges
            case createdAtUnixMs = "created_at_unix_ms"
            case updatedAtUnixMs = "updated_at_unix_ms"
        }
    }

    /// FR-CHAT-6: creates a new conversation, returning its id, or nil on a null
    /// handle/DB error. `{"id": ...}` is decoded via a private helper struct rather than
    /// a full model type, since this is the only place that specific shape is needed.
    func createConversation(firstQuery: String) -> Int64? {
        guard let json = firstQuery.withCString({ whimbrel_conversations_create(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct IdResponse: Decodable { let id: Int64 }
        let response: IdResponse? = decode(json)
        return response?.id
    }

    func appendConversationExchange(conversationId: Int64, query: String, answer: String, citationsJson: String, model: String?) -> Bool {
        query.withCString { q in
            answer.withCString { a in
                citationsJson.withCString { c in
                    guard let model else { return whimbrel_conversations_append_exchange(handle, conversationId, q, a, c, nil) }
                    return model.withCString { m in whimbrel_conversations_append_exchange(handle, conversationId, q, a, c, m) }
                }
            }
        }
    }

    func listConversations() -> [ConversationSummary]? {
        guard let json = whimbrel_conversations_list(handle) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    func getConversation(id: Int64) -> ConversationDetail? {
        guard let json = whimbrel_conversations_get(handle, id) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    func renameConversation(id: Int64, newTitle: String) -> Bool {
        newTitle.withCString { whimbrel_conversations_rename(handle, id, $0) }
    }

    func setConversationTitleIfPlaceholder(id: Int64, generatedTitle: String) -> Bool {
        generatedTitle.withCString { whimbrel_conversations_set_title_if_placeholder(handle, id, $0) }
    }

    func setConversationStarred(id: Int64, starred: Bool) -> Bool {
        whimbrel_conversations_set_starred(handle, id, starred)
    }

    func deleteConversation(id: Int64) -> Bool {
        whimbrel_conversations_delete(handle, id)
    }

    /// Returns the generated title on success, or nil on any failure (provider
    /// failure, spend cap, malformed response) — the caller's job is to leave the
    /// existing placeholder title untouched on nil, never retry.
    func generateConversationTitle(model: String, query: String, answer: String, routing: CoreRoutingConfig) -> String? {
        guard let routingJson = try? JSONEncoder().encode(routing), let routingJsonString = String(data: routingJson, encoding: .utf8) else { return nil }
        guard let json = whimbrel_llm_generate_conversation_title(handle, model, query, answer, routingJsonString) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Envelope: Decodable { let ok: Bool; let data: TitleData?; struct TitleData: Decodable { let title: String } }
        guard let envelope: Envelope = decode(json), envelope.ok, let title = envelope.data?.title else { return nil }
        return title
    }

    static var coreVersion: String {
        guard let ptr = whimbrel_core_version() else { return "unknown" }
        let version = String(cString: ptr)
        whimbrel_core_free_string(UnsafeMutablePointer(mutating: ptr))
        return version
    }

    /// Encrypts already-serialized payload bytes under the group key. `aad`
    /// (e.g. "account_id||device_id||sequence_number") is authenticated but
    /// not encrypted, and must be supplied unchanged to
    /// `decryptSyncPayload` for this same ciphertext. Returns the ciphertext's
    /// base64 encoding, or nil on malformed input.
    static func encryptSyncPayload(groupKeyBase64: String, plaintextBase64: String, aad: String) -> String? {
        guard let json = groupKeyBase64.withCString({ key in
            plaintextBase64.withCString { plaintext in
                aad.withCString { aad in
                    whimbrel_sync_encrypt_payload(key, plaintext, aad)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Response: Decodable {
            let ciphertextBase64: String
            enum CodingKeys: String, CodingKey { case ciphertextBase64 = "ciphertext_base64" }
        }
        let data = Data(String(cString: json).utf8)
        let response: Response? = try? JSONDecoder().decode(Response.self, from: data)
        return response?.ciphertextBase64
    }

    /// Decrypts a blob produced by `encryptSyncPayload`. `aad` must exactly
    /// match what was passed to `encryptSyncPayload` for this blob. Returns
    /// the plaintext's base64 encoding, or nil on malformed input or a
    /// decrypt failure.
    static func decryptSyncPayload(groupKeyBase64: String, ciphertextBase64: String, aad: String) -> String? {
        guard let json = groupKeyBase64.withCString({ key in
            ciphertextBase64.withCString { ciphertext in
                aad.withCString { aad in
                    whimbrel_sync_decrypt_payload(key, ciphertext, aad)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Response: Decodable {
            let plaintextBase64: String
            enum CodingKeys: String, CodingKey { case plaintextBase64 = "plaintext_base64" }
        }
        let data = Data(String(cString: json).utf8)
        let response: Response? = try? JSONDecoder().decode(Response.self, from: data)
        return response?.plaintextBase64
    }

    /// One field of a settings-sync push/pull payload — mirrors
    /// `core/sync-engine::SettingsField`'s JSON shape exactly (snake_case
    /// on the wire via `CodingKeys`, camelCase in Swift).
    struct SyncSettingsField: Codable {
        let value: String
        let updatedAtUnixMs: Int64
        let isDeleted: Bool

        enum CodingKeys: String, CodingKey {
            case value
            case updatedAtUnixMs = "updated_at_unix_ms"
            case isDeleted = "is_deleted"
        }
    }

    /// One local setting with its "last locally modified" timestamp, as gathered for
    /// `syncSettingsIteration` (`api::sync_ops::coordinators::LocalField`).
    struct SyncLocalField: Codable {
        let value: String
        let updatedAtUnixMs: Int64

        enum CodingKeys: String, CodingKey {
            case value
            case updatedAtUnixMs = "updated_at_unix_ms"
        }
    }

    /// One provider API key as this device sees it. `key` is only consulted while
    /// `syncEnabled` is true; the core decides what (if anything) to push and with
    /// which timestamp — see `whimbrel_sync_settings_iteration` in whimbrel_core.h.
    struct SyncApiKeyState: Codable {
        let fieldKey: String
        let syncEnabled: Bool
        let key: String?
        let keyUpdatedAtUnixMs: Int64
        let toggleUpdatedAtUnixMs: Int64

        enum CodingKeys: String, CodingKey {
            case fieldKey = "field_key"
            case syncEnabled = "sync_enabled"
            case key
            case keyUpdatedAtUnixMs = "key_updated_at_unix_ms"
            case toggleUpdatedAtUnixMs = "toggle_updated_at_unix_ms"
        }
    }

    /// Everything the shell gathers for one settings-sync iteration.
    struct SyncLocalSettings: Codable {
        var fields: [String: SyncLocalField] = [:]
        var apiKeys: [SyncApiKeyState] = []
        var hiddenEpisodeMarks: [String: SyncSettingsField] = [:]

        enum CodingKeys: String, CodingKey {
            case fields
            case apiKeys = "api_keys"
            case hiddenEpisodeMarks = "hidden_episode_marks"
        }
    }

    struct SyncSettingsIteration: Codable {
        let pushedCount: Int
        let pulled: [String: SyncSettingsField]

        enum CodingKeys: String, CodingKey {
            case pushedCount = "pushed_count"
            case pulled
        }
    }

    /// One pulled field the shell should apply (`whimbrel_sync_settings_apply_plan`).
    struct SyncPulledField: Codable {
        let fieldKey: String
        let value: String
        let updatedAtUnixMs: Int64
        let isDeleted: Bool

        enum CodingKeys: String, CodingKey {
            case fieldKey = "field_key"
            case value
            case updatedAtUnixMs = "updated_at_unix_ms"
            case isDeleted = "is_deleted"
        }
    }

    struct SyncSettingsApplyPlan: Codable {
        let fields: [SyncPulledField]
        let hiddenEpisodeMarks: [String: SyncSettingsField]

        enum CodingKeys: String, CodingKey {
            case fields
            case hiddenEpisodeMarks = "hidden_episode_marks"
        }
    }

    /// One settings-sync iteration: the core plans the push from `local` (including
    /// the API-key tombstone timestamp rule), runs the round trip against the
    /// account's Turso data-plane database and returns the pulled delta. Blocking
    /// network call — run it off the main actor. Returns nil on malformed input or a
    /// sync failure (network error, wrong group key, etc. — no detail, matching the
    /// Rust side).
    static func syncSettingsIteration(
        tursoURL: String,
        tursoToken: String,
        groupKeyBase64: String,
        local: SyncLocalSettings
    ) -> SyncSettingsIteration? {
        guard let localData = try? JSONEncoder().encode(local),
              let localJSON = String(data: localData, encoding: .utf8) else {
            return nil
        }
        guard let json = tursoURL.withCString({ url in
            tursoToken.withCString { token in
                groupKeyBase64.withCString { key in
                    localJSON.withCString { local in
                        whimbrel_sync_settings_iteration(url, token, key, local)
                    }
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(SyncSettingsIteration.self, from: Data(String(cString: json).utf8))
    }

    /// Pure: filters a pulled delta down to what should be applied, given this
    /// device's per-field timestamps read AFTER the round trip and its API-key sync
    /// toggles. Returns nil on malformed input.
    static func syncSettingsApplyPlan(
        pulled: [String: SyncSettingsField],
        localTimestamps: [String: Int64],
        apiKeySyncEnabled: [String: Bool]
    ) -> SyncSettingsApplyPlan? {
        guard let pulledData = try? JSONEncoder().encode(pulled),
              let pulledJSON = String(data: pulledData, encoding: .utf8),
              let timestampsData = try? JSONEncoder().encode(localTimestamps),
              let timestampsJSON = String(data: timestampsData, encoding: .utf8),
              let togglesData = try? JSONEncoder().encode(apiKeySyncEnabled),
              let togglesJSON = String(data: togglesData, encoding: .utf8) else {
            return nil
        }
        guard let json = pulledJSON.withCString({ pulled in
            timestampsJSON.withCString { timestamps in
                togglesJSON.withCString { toggles in
                    whimbrel_sync_settings_apply_plan(pulled, timestamps, toggles)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(SyncSettingsApplyPlan.self, from: Data(String(cString: json).utf8))
    }

    /// Runs one content-sync round trip against the account's Turso data-plane
    /// database: pushes this device's unpushed capture_item/meeting/transcript_segment
    /// rows, then pulls and applies any peer devices' new rows directly into this
    /// local store. Unlike `syncSettingsIteration`, there is no delta to apply afterward —
    /// everything happens inside the Rust call. Returns `false` on any sync failure
    /// (network error, wrong group key, etc. — no detail, matching `syncSettingsIteration`).
    func syncContent(tursoURL: String, tursoToken: String, groupKeyBase64: String, deviceId: String) -> Bool {
        tursoURL.withCString { url in
            tursoToken.withCString { token in
                groupKeyBase64.withCString { key in
                    deviceId.withCString { device in
                        whimbrel_sync_content(handle, url, token, key, device)
                    }
                }
            }
        }
    }

    /// After a fresh start: every local row (and skin) goes up again to the new, empty sync
    /// database. Call on `accessQueue`.
    @discardableResult
    func resetSyncPushCursor() -> Bool {
        whimbrel_sync_reset_push_cursor(handle)
    }

    /// Skins Task 16: queues the user skin as `skinsRoot` now holds it (a missing folder sends a deletion).
    /// Call on `accessQueue` with `UserSkinStore.writing` held; see `SkinSync`.
    @discardableResult
    func skinSyncEnqueue(skinsRoot: URL, skinId: String) -> Bool {
        skinsRoot.path.withCString { root in skinId.withCString { whimbrel_skin_sync_enqueue(handle, root, $0) } }
    }

    /// Queues every skin in `skinsRoot` again (`SkinReconcileReport.pushAll`). As `skinSyncEnqueue`.
    @discardableResult
    func skinSyncEnqueueAll(skinsRoot: URL) -> Bool {
        skinsRoot.path.withCString { whimbrel_skin_sync_enqueue_all(handle, $0) }
    }

    /// Writes the skins pulled from other devices into `skinsRoot` and takes the first-run and re-push
    /// flags; nil on a malformed reply. As `skinSyncEnqueue`.
    func skinSyncReconcile(skinsRoot: URL) -> SkinReconcileReport? {
        guard let json = skinsRoot.path.withCString({ whimbrel_skin_sync_reconcile(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? CoreLLMEnvelope<SkinReconcileReport>.result(json).get()
    }

    /// Why the most recent `syncContent` failed and in which phase; nil if it succeeded.
    static func lastSyncContentError() -> String? {
        guard let msg = whimbrel_sync_content_last_error() else { return nil }
        defer { whimbrel_core_free_string(msg) }
        return String(cString: msg)
    }

    /// FR-MTG-4: on-device transcription via whisper.cpp. Stateless — not tied to a
    /// store handle, so this is a static function rather than an instance method.
    /// `wavPath` must already be 16kHz PCM (see AudioConversion.swift). Returns nil on
    /// a missing/bad model, non-16kHz WAV, or any other whisper.cpp failure.
    static func transcribeWav(modelPath: URL, wavPath: URL) -> [CoreTranscriptSegment]? {
        guard let json = modelPath.path.withCString({ model in
            wavPath.path.withCString { wav in
                whimbrel_transcribe_wav(model, wav)
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        return try? JSONDecoder().decode([CoreTranscriptSegment].self, from: data)
    }

    /// FR-MTG-4 follow-on: runs speaker diarization on a WAV file alone (no transcript
    /// needed) — call this concurrently with `transcribeWav` on the same file, not
    /// sequentially after it, then combine both results via `labelSpeakers` once both
    /// finish. `expectedSpeakerCount`, when non-nil and positive, forces clustering to
    /// produce exactly that many speakers (e.g. a calendar meeting's known attendee
    /// count) — pass `nil` when no reliable count is known, falling back to
    /// threshold-based auto-clustering. Returns nil on any failure (missing/corrupt
    /// models, sample-rate mismatch, processing failure) — callers must degrade to
    /// their pre-diarization behavior on nil, never lose the transcript over this.
    static func diarizeWav(segmentationModelPath: URL, embeddingModelPath: URL, wavPath: URL, expectedSpeakerCount: Int?) -> [CoreSpeakerSegment]? {
        guard let json = segmentationModelPath.path.withCString({ seg in
            embeddingModelPath.path.withCString { emb in
                wavPath.path.withCString { wav in
                    whimbrel_diarize_wav(seg, emb, wav, Int32(expectedSpeakerCount ?? -1))
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        return try? JSONDecoder().decode([CoreSpeakerSegment].self, from: data)
    }

    /// Merges transcript segments with speaker segments (from `transcribeWav`/
    /// `diarizeWav`, run concurrently) into one labeled list. Pure/cheap — call only
    /// after both of the above have completed. `transcriptSegmentsJSON`/
    /// `speakerSegmentsJSON` are raw JSON strings (encode the arrays yourself via
    /// `JSONEncoder` before calling this).
    static func labelSpeakers(transcriptSegmentsJSON: String, speakerSegmentsJSON: String) -> [CoreLabeledSegment]? {
        guard let json = transcriptSegmentsJSON.withCString({ t in
            speakerSegmentsJSON.withCString { s in
                whimbrel_label_speakers(t, s)
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        return try? JSONDecoder().decode([CoreLabeledSegment].self, from: data)
    }

    /// M3.0 Task 1: the write-time transcript pipeline (core/transcribe::postprocess).
    /// Drops "Me" lines that are bleed-through of a system-audio line, interleaves both
    /// channels chronologically, merges adjacent same-speaker lines. Pass an empty
    /// `systemLines` for a mic-only session (then this is just the same-speaker merge).
    /// Pure, stateless. Returns the merged lines plus how many mic lines survived the
    /// bleed filter (for the caller's diagnostics log); nil only if JSON round-tripping
    /// fails, which would be a bug on either side of the bridge.
    static func postprocessTranscript(micLines: [TranscriptionService.Line], systemLines: [TranscriptionService.Line]) -> (lines: [TranscriptionService.Line], micLinesKept: Int)? {
        guard let micJSON = try? JSONEncoder().encode(micLines),
              let systemJSON = try? JSONEncoder().encode(systemLines),
              let json = String(decoding: micJSON, as: UTF8.self).withCString({ m in
                  String(decoding: systemJSON, as: UTF8.self).withCString { s in
                      whimbrel_transcript_postprocess(m, s)
                  }
              }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        guard let result = try? JSONDecoder().decode(CoreTranscriptPostprocessResult.self, from: data) else { return nil }
        return (lines: result.lines, micLinesKept: result.micLinesKept)
    }

    /// FR-MTG-2: which meeting service a browser address-bar value is on (core's
    /// exact-host check, shared with Windows). Nil when it isn't a meeting URL.
    static func meetingURLProvider(_ address: String) -> MeetingProvider? {
        guard let out = address.withCString({ whimbrel_meeting_url_provider($0) }) else { return nil }
        defer { whimbrel_core_free_string(out) }
        return MeetingProvider(rawValue: String(cString: out))
    }

    /// FR-MTG-5: one poll of core's meeting end rules (api::meetings::end_rule), so the
    /// Mac and Windows end meetings by the same rules. Pure, stateless: the caller
    /// carries `state` between polls. Nil only if JSON round-tripping fails, which
    /// would be a bug on either side of the bridge.
    static func evaluateMeetingEnd(_ signals: CoreMeetingEndSignals, state: CoreMeetingEndState) -> (reason: String?, state: CoreMeetingEndState)? {
        guard let signalsJSON = try? JSONEncoder().encode(signals),
              let stateJSON = try? JSONEncoder().encode(state),
              let json = String(decoding: signalsJSON, as: UTF8.self).withCString({ sig in
                  String(decoding: stateJSON, as: UTF8.self).withCString { st in
                      whimbrel_meeting_end_evaluate(sig, st)
                  }
              }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Result: Decodable { let reason: String?; let state: CoreMeetingEndState }
        guard let result = try? JSONDecoder().decode(Result.self, from: Data(String(cString: json).utf8)) else { return nil }
        return (result.reason, result.state)
    }

    /// Parses a flat `"[HH:mm:ss] Speaker: text"`-per-line transcript into display
    /// turns (core/transcribe::postprocess::parse_turns). A line that doesn't match the
    /// expected shape is skipped, not fatal; the caller falls back to rendering `raw`
    /// unparsed when this returns an empty array.
    static func parseTranscriptTurns(from raw: String) -> [TranscriptTurn] {
        guard let json = raw.withCString({ whimbrel_transcript_parse_turns($0) }) else { return [] }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        return (try? JSONDecoder().decode([TranscriptTurn].self, from: data)) ?? []
    }

    /// M3.0 Task 3: starts downloading whichever of the three model files are missing
    /// (core/api::models). Returns the job id to poll, or nil if the bridge rejected the
    /// request. The work runs on the core's own thread; poll from a `Task`, never from
    /// `accessQueue`.
    static func startModelDownload(whisperModelPath: URL, segmentationModelPath: URL, embeddingModelPath: URL) -> String? {
        let destinations: [String: String] = [
            "whisper_model_path": whisperModelPath.path,
            "segmentation_model_path": segmentationModelPath.path,
            "embedding_model_path": embeddingModelPath.path,
        ]
        guard let data = try? JSONEncoder().encode(destinations),
              let json = String(decoding: data, as: UTF8.self).withCString({ whimbrel_models_download_start($0) })
        else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Started: Decodable { let jobId: String; enum CodingKeys: String, CodingKey { case jobId = "job_id" } }
        return (try? JSONDecoder().decode(Started.self, from: Data(String(cString: json).utf8)))?.jobId
    }

    /// Latest progress for a model-download job, or nil if the job is unknown.
    static func pollModelDownload(jobId: String) -> CoreModelDownloadProgress? {
        guard let json = jobId.withCString({ whimbrel_models_download_poll($0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(CoreModelDownloadProgress.self, from: Data(String(cString: json).utf8))
    }

    /// Requests cancellation; `cancelled` then arrives through `pollModelDownload`.
    @discardableResult
    static func cancelModelDownload(jobId: String) -> Bool {
        jobId.withCString { whimbrel_models_download_cancel($0) }
    }

    /// Frees the job's registry entry. Safe to call twice.
    static func releaseModelDownload(jobId: String) {
        jobId.withCString { whimbrel_models_download_release($0) }
    }

    /// M3.0 Task 4: decodes a version manifest and compares it against
    /// `currentVersion` (core/api::update). Nil if the manifest is not valid JSON or
    /// lacks `version`/`url`; a malformed `currentVersion` yields `newer == false`.
    static func checkUpdateManifest(json: Data, currentVersion: String) -> CoreUpdateManifestCheck? {
        guard let manifestJSON = String(data: json, encoding: .utf8),
              let out = manifestJSON.withCString({ m in
                  currentVersion.withCString { v in whimbrel_update_check_manifest(m, v) }
              }) else { return nil }
        defer { whimbrel_core_free_string(out) }
        return try? JSONDecoder().decode(CoreUpdateManifestCheck.self, from: Data(String(cString: out).utf8))
    }

    /// True if the file at `path` hashes to `expectedSha256` (SHA-256, streamed,
    /// case-insensitive). A nil `expectedSha256` is the legacy no-hash manifest and
    /// passes. Blocking (the update zip is ~260 MB): call off the main actor.
    static func verifyUpdateSha256(path: URL, expectedSha256: String?) -> Bool {
        path.path.withCString { p in
            withOptionalCString(expectedSha256) { e in whimbrel_update_verify_sha256(p, e) }
        }
    }

    /// M3.0 Task 5: the built-in Ask starter prompts (core/api::prompts).
    static func starterPrompts() -> [LibraryPrompt] {
        guard let json = whimbrel_prompts_starters() else { return [] }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode([LibraryPrompt].self, from: Data(String(cString: json).utf8))) ?? []
    }

    /// Whether an Ask-mode question needs a fast response rather than tolerating a
    /// slower local model (core/api::prompts::is_time_critical).
    static func isTimeCritical(_ query: String) -> Bool {
        query.withCString { whimbrel_ask_is_time_critical($0) }
    }

    /// Fetches the sorted model ids for `provider` ("anthropic", "openai", "google",
    /// "ollama") from that provider's own listing endpoint (core/api::hosted_models).
    /// Blocking, up to 30 s: call off the main actor. `baseURLOverride` is only for
    /// Ollama's configurable host.
    static func fetchHostedModels(provider: String, apiKey: String?, baseURLOverride: String? = nil) -> Result<[String], HostedModelsService.FetchError> {
        struct Reply: Decodable { let models: [String]?; let error: String? }
        guard let json = provider.withCString({ p in
            withOptionalCString(apiKey) { k in
                withOptionalCString(baseURLOverride) { b in whimbrel_hosted_models_fetch(p, k, b) }
            }
        }) else { return .failure(HostedModelsService.FetchError(message: "Unknown model provider \(provider).")) }
        defer { whimbrel_core_free_string(json) }
        guard let reply = try? JSONDecoder().decode(Reply.self, from: Data(String(cString: json).utf8)) else {
            return .failure(HostedModelsService.FetchError(message: "Model list response could not be decoded."))
        }
        if let models = reply.models { return .success(models) }
        return .failure(HostedModelsService.FetchError(message: reply.error ?? "Model list request failed."))
    }

    /// Validates a pasted key against the provider's model list (core/api::hosted_models::
    /// validate_key). Blocking, up to 30 s: call off the main actor. Nil only on an
    /// unknown provider or a decode failure.
    static func keyCheck(provider: String, apiKey: String, baseURLOverride: String? = nil) -> CoreKeyCheck? {
        guard let json = provider.withCString({ p in
            apiKey.withCString { k in
                withOptionalCString(baseURLOverride) { b in whimbrel_key_check(p, k, b) }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(CoreKeyCheck.self, from: Data(String(cString: json).utf8))
    }

    /// The key on-ramp's provider table (core/api::providers::PROVIDERS), in core's order.
    static func providers() -> [CoreProviderInfo] {
        guard let json = whimbrel_providers() else { return [] }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode([CoreProviderInfo].self, from: Data(String(cString: json).utf8))) ?? []
    }

    /// The one-line warning when `key` does not look like `provider`'s
    /// (core/api::providers::format_hint); nil when it does.
    static func keyFormatHint(provider: String, key: String) -> String? {
        guard let hint = provider.withCString({ p in key.withCString { k in whimbrel_key_format_hint(p, k) } }) else { return nil }
        defer { whimbrel_core_free_string(hint) }
        return String(cString: hint)
    }

    /// M3.0 Task 6: the Before/During/After/Cancelled decision for one meeting
    /// (core/api::meeting_workspace::derive_state). Pure, no store: the caller passes the
    /// rows it already holds and the in-process signals. Nil only on a JSON round-trip
    /// failure, which would be a bug on either side of the bridge.
    static func meetingWorkspaceState(
        meeting: CoreMeeting, draft: CoreMeetingNoteDraft?, actionItems: [CoreMeetingActionItem],
        activeMeetingId: Int64?, processingMeetingId: Int64?, processingStage: String?, nowUnixMs: Int64
    ) -> MeetingWorkspaceState? {
        struct Signals: Encodable {
            let activeMeetingId: Int64?
            let processingMeetingId: Int64?
            let processingStage: String?
            let nowUnixMs: Int64
            enum CodingKeys: String, CodingKey {
                case activeMeetingId = "active_meeting_id"
                case processingMeetingId = "processing_meeting_id"
                case processingStage = "processing_stage"
                case nowUnixMs = "now_unix_ms"
            }
        }
        let encoder = JSONEncoder()
        guard let meetingData = try? encoder.encode(meeting),
              let draftData = try? encoder.encode(draft),
              let itemsData = try? encoder.encode(actionItems),
              let signalsData = try? encoder.encode(Signals(activeMeetingId: activeMeetingId, processingMeetingId: processingMeetingId, processingStage: processingStage, nowUnixMs: nowUnixMs))
        else { return nil }
        guard let json = String(decoding: meetingData, as: UTF8.self).withCString({ m in
            String(decoding: draftData, as: UTF8.self).withCString { d in
                String(decoding: itemsData, as: UTF8.self).withCString { a in
                    String(decoding: signalsData, as: UTF8.self).withCString { s in
                        whimbrel_meeting_workspace_state(m, d, a, s)
                    }
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(MeetingWorkspaceState.self, from: Data(String(cString: json).utf8))
    }

    // MARK: - Connector sync (core/api::connectors, M3.0 Task 8)

    /// Phase 1 of one account's Google Calendar sync: the `sync_events` call carrying
    /// the account's stored sync token (or none for a baseline). Nil on a DB error.
    /// Discards one account's stored sync token, so the next request is a full sync.
    @discardableResult
    func clearCalendarSyncToken(accountEmail: String) -> Bool {
        accountEmail.withCString { whimbrel_memory_clear_calendar_sync_token(handle, $0) }
    }

    func calendarSyncRequest(accountEmail: String) -> CoreConnectorToolCall? {
        guard let json = accountEmail.withCString({ whimbrel_connector_calendar_request(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Phase 2: applies one `sync_events` result (cancelled events deleted from items,
    /// token advanced only after that succeeds) and folds it into `existing`, the
    /// account's known meetings. `needsFullResync` means the token was cleared and the
    /// caller should re-request once. Nil on a DB error.
    func applyCalendarSync(accountEmail: String, resultJSON: String, existing: [QualifyingMeeting], nowUnixMs: Int64) -> CoreCalendarApplyOutcome? {
        guard let existingData = try? JSONEncoder().encode(existing) else { return nil }
        guard let json = accountEmail.withCString({ e in
            resultJSON.withCString { r in
                String(decoding: existingData, as: UTF8.self).withCString { x in
                    whimbrel_connector_calendar_apply(handle, e, r, x, nowUnixMs)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Phase 1 of one account's Gmail sync: the `sync_messages` call carrying the
    /// stored historyId (or none for a baseline). Nil on a DB error.
    func gmailSyncRequest(accountEmail: String) -> CoreConnectorToolCall? {
        guard let json = accountEmail.withCString({ whimbrel_connector_gmail_request(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Phase 2: applies one `sync_messages` result (deletions, upserts stamped with
    /// `deviceId`, historyId advanced only if both succeeded). Nil on a DB error.
    func applyGmailSync(accountEmail: String, deviceId: String, resultJSON: String) -> CoreGmailApplyOutcome? {
        guard let json = accountEmail.withCString({ e in
            deviceId.withCString { d in
                resultJSON.withCString { r in whimbrel_connector_gmail_apply(handle, e, d, r) }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// Merges EventKit's meetings with the OAuth connector's, deduping by exact start
    /// time + case-insensitive title (the OAuth copy wins); sorted by start time.
    static func mergeMeetings(eventKit: [QualifyingMeeting], oauth: [QualifyingMeeting]) -> [QualifyingMeeting] {
        guard let ekData = try? JSONEncoder().encode(eventKit), let oauthData = try? JSONEncoder().encode(oauth),
              let json = String(decoding: ekData, as: UTF8.self).withCString({ e in
                  String(decoding: oauthData, as: UTF8.self).withCString { o in whimbrel_calendar_merge_meetings(e, o) }
              }) else { return oauth + eventKit }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode([QualifyingMeeting].self, from: Data(String(cString: json).utf8))) ?? (oauth + eventKit)
    }

    /// The items-table calendar-event upsert input for a calendar-sourced meeting; nil
    /// for a detected meeting (no calendar entry).
    static func calendarEventItem(for meeting: QualifyingMeeting, deviceId: String) -> NewCalendarEventItemInput? {
        guard let data = try? JSONEncoder().encode(meeting),
              let json = String(decoding: data, as: UTF8.self).withCString({ m in
                  deviceId.withCString { d in whimbrel_calendar_event_item(m, d) }
              }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(NewCalendarEventItemInput?.self, from: Data(String(cString: json).utf8)) ?? nil
    }

    /// The first recognized conferencing link in `text` (Zoom, Google Meet, Teams,
    /// Webex), or nil.
    static func detectConferenceLink(in text: String) -> DetectedConferenceLink? {
        guard let json = text.withCString({ whimbrel_meeting_link_detect($0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(DetectedConferenceLink?.self, from: Data(String(cString: json).utf8)) ?? nil
    }

    // MARK: - Connector account registry (core/api::connectors::accounts, M3.0 Task 9)

    /// The connected accounts for `connector` ("calendar", "gmail", "outlook365") in
    /// connection order, plus the default. Nil on a DB error.
    func connectorAccounts(connector: String) -> CoreConnectorAccounts? {
        guard let json = connector.withCString({ whimbrel_connector_accounts_list(handle, $0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    /// 1 if added (default automatically if first), 0 if already connected, -1 on error.
    func addConnectorAccount(connector: String, email: String, nowUnixMs: Int64) -> Int32 {
        connector.withCString { c in email.withCString { e in whimbrel_connector_accounts_add(handle, c, e, nowUnixMs) } }
    }

    /// Removes `email`, promoting the next remaining account if it was the default.
    func removeConnectorAccount(connector: String, email: String) -> Bool {
        connector.withCString { c in email.withCString { e in whimbrel_connector_accounts_remove(handle, c, e) } }
    }

    /// Makes `email` the default; a no-op if it isn't connected.
    func setDefaultConnectorAccount(connector: String, email: String) -> Bool {
        connector.withCString { c in email.withCString { e in whimbrel_connector_accounts_set_default(handle, c, e) } }
    }

    // MARK: - OAuth (core/api::oauth, M3.0 Task 10)

    /// Binds the loopback listener (127.0.0.1 only) and builds the consent URL. Nil on
    /// an unknown provider or a bind failure.
    static func oauthBegin(config: CoreOAuthConfig) -> CoreOAuthSession? {
        guard let data = try? JSONEncoder().encode(config),
              let json = String(decoding: data, as: UTF8.self).withCString({ whimbrel_oauth_begin($0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return try? JSONDecoder().decode(CoreOAuthSession.self, from: Data(String(cString: json).utf8))
    }

    private struct OAuthOutcome: Decodable {
        let tokens: OAuthStoredTokens?
        let error: String?
        let reason: String?
    }

    private static func decodeOAuthOutcome(_ json: UnsafeMutablePointer<CChar>) -> Result<OAuthStoredTokens, CoreOAuthError> {
        defer { whimbrel_core_free_string(json) }
        guard let outcome = try? JSONDecoder().decode(OAuthOutcome.self, from: Data(String(cString: json).utf8)) else {
            return .failure(.listenerFailed)
        }
        if let tokens = outcome.tokens { return .success(tokens) }
        return .failure(CoreOAuthError(code: outcome.error ?? "", reason: outcome.reason))
    }

    /// BLOCKING (up to `timeoutMs`): waits for the provider's redirect, checks `state`,
    /// exchanges the code. Consumes the session. Call off the main actor and never on
    /// `accessQueue`.
    static func oauthWait(sessionId: String, timeoutMs: UInt64, nowUnixMs: Int64) -> Result<OAuthStoredTokens, CoreOAuthError> {
        guard let json = sessionId.withCString({ whimbrel_oauth_wait($0, timeoutMs, nowUnixMs) }) else {
            return .failure(.listenerFailed)
        }
        return decodeOAuthOutcome(json)
    }

    /// Asks a pending `oauthWait` to return `.cancelled` (the auth window was closed).
    @discardableResult
    static func oauthCancel(sessionId: String) -> Bool {
        sessionId.withCString { whimbrel_oauth_cancel($0) }
    }

    /// BLOCKING: the refresh grant, returning the merged Keychain blob.
    static func oauthRefresh(config: CoreOAuthConfig, stored: OAuthStoredTokens, nowUnixMs: Int64) -> Result<OAuthStoredTokens, CoreOAuthError> {
        guard let configData = try? JSONEncoder().encode(config), let storedData = try? JSONEncoder().encode(stored),
              let json = String(decoding: configData, as: UTF8.self).withCString({ c in
                  String(decoding: storedData, as: UTF8.self).withCString { s in whimbrel_oauth_refresh(c, s, nowUnixMs) }
              }) else { return .failure(.tokenRefreshFailed) }
        return decodeOAuthOutcome(json)
    }

    /// Whether `granted` (nil: unknown, never sufficient) covers every scope in `required`.
    static func oauthScopeSatisfies(granted: String?, required: String) -> Bool {
        required.withCString { r in
            withOptionalCString(granted) { g in whimbrel_oauth_scope_satisfies(g, r) }
        }
    }

    /// The unverified payload claims of a JWT, or nil if it isn't one.
    static func oauthJwtClaims(_ jwt: String) -> [String: Any]? {
        guard let json = jwt.withCString({ whimbrel_oauth_jwt_claims($0) }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: json).utf8))) as? [String: Any]
    }

    /// Decodes one of the three LB0-5-enveloped LLM call results. `json` is assumed
    /// non-null already (the caller's own `guard let json = ... else { return nil }`
    /// handles genuinely-invalid-usage before this is reached) — a malformed envelope
    /// at this point would mean the Rust side produced invalid JSON, which shouldn't
    /// happen in practice; falls back to `.providerFailure` defensively rather than
    /// crashing.
    private func decodeLLMResult<T: Decodable>(_ json: UnsafeMutablePointer<CChar>) -> Result<T, CoreLLMError> {
        CoreLLMEnvelope<T>.result(json)
    }

    private func decode<T: Decodable>(_ json: UnsafeMutablePointer<CChar>) -> T? {
        let data = Data(String(cString: json).utf8)
        return try? JSONDecoder().decode(T.self, from: data)
    }
}

/// Task 6's Settings-facing wrappers around Task 5's client-token FFI
/// (`whimbrel_mcp_generate_client_token`/`_list_client_tokens`/`_revoke_client_token`).
extension CoreMemoryStore {
    /// Returns the one-time plaintext token string ("kmcp_<id>.<secret>") to show
    /// the user — never retrievable again after this call returns.
    func generateMcpClientToken(name: String, scopes: [String]) -> (id: Int64, token: String)? {
        guard let scopesJson = try? JSONEncoder().encode(scopes),
              let scopesJsonString = String(data: scopesJson, encoding: .utf8) else { return nil }
        guard let json = name.withCString({ namePtr in
            scopesJsonString.withCString { scopesPtr in
                whimbrel_mcp_generate_client_token(handle, namePtr, scopesPtr)
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Result: Decodable { let id: Int64; let token: String }
        guard let result: Result = decode(json) else { return nil }
        return (result.id, result.token)
    }

    func listMcpClientTokens() -> [CoreMcpClientTokenInfo]? {
        guard let json = whimbrel_mcp_list_client_tokens(handle) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return decode(json)
    }

    @discardableResult
    func revokeMcpClientToken(id: Int64) -> Bool {
        whimbrel_mcp_revoke_client_token(handle, id)
    }
}

/// Wraps one `McpHostHandle` — a registry of live MCP connectors. Owns the
/// handle's lifetime: closes it in `deinit`, mirroring `CoreMemoryStore`'s
/// own handle-ownership pattern.
final class McpHost {
    private let handle: OpaquePointer
    /// The last `callTool` failure's message (e.g. a Google API 403), or nil.
    private(set) var lastError: String?

    init?() {
        guard let handle = whimbrel_mcp_open() else { return nil }
        self.handle = handle
    }

    deinit {
        whimbrel_mcp_close(handle)
    }

    /// Spawns `command` with `args`/`env` and performs the MCP handshake.
    /// Returns the connector id, or nil on any failure.
    func connect(command: String, args: [String], env: [(String, String)]) -> Int64? {
        guard let argsJSON = try? JSONEncoder().encode(args),
              let argsString = String(data: argsJSON, encoding: .utf8) else { return nil }
        let envPairs = env.map { [$0.0, $0.1] }
        guard let envJSON = try? JSONEncoder().encode(envPairs),
              let envString = String(data: envJSON, encoding: .utf8) else { return nil }

        let connectorId = command.withCString { c in
            argsString.withCString { a in
                envString.withCString { e in
                    whimbrel_mcp_connect(handle, c, a, e)
                }
            }
        }
        return connectorId >= 0 ? connectorId : nil
    }

    struct ToolInfo: Decodable {
        let name: String
        let description: String?
    }

    /// Lists the tools the connector at `connectorId` exposes. Returns nil
    /// on any failure.
    func listTools(connectorId: Int64) -> [ToolInfo]? {
        guard let json = whimbrel_mcp_list_tools(handle, connectorId) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        return try? JSONDecoder().decode([ToolInfo].self, from: data)
    }

    /// Calls `toolName` on the connector at `connectorId` with
    /// `argumentsJSON` (a JSON object string). Returns the tool's raw text
    /// result, or nil on any failure. When the tool call itself (or the MCP
    /// protocol) failed, `lastError` holds the connector's message.
    func callTool(connectorId: Int64, toolName: String, argumentsJSON: String) -> String? {
        lastError = nil
        guard let json = toolName.withCString({ t in
            argumentsJSON.withCString { a in
                whimbrel_mcp_call_tool(handle, connectorId, t, a)
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        let data = Data(String(cString: json).utf8)
        if let result = try? JSONDecoder().decode(String.self, from: data) { return result }
        lastError = (try? JSONDecoder().decode([String: String].self, from: data))?["error"]
        return nil
    }

    /// Disconnects the connector at `connectorId`. Returns whether it
    /// succeeded.
    @discardableResult
    func disconnect(connectorId: Int64) -> Bool {
        whimbrel_mcp_disconnect(handle, connectorId)
    }
}

/// Mirrors `api::meetings::end_rule::EndSignals`. `captureSessionActive` is nil unless
/// a detected native meeting app's own audio use is known (#84).
struct CoreMeetingEndSignals: Encodable {
    let nowUnixMs: Int64
    let recordingStartedUnixMs: Int64
    let appRunning: Bool
    let onMeetingTab: Bool?
    let scheduledEndUnixMs: Int64?
    let audioActive: Bool
    let captureSessionActive: Bool?

    enum CodingKeys: String, CodingKey {
        case nowUnixMs = "now_unix_ms"
        case recordingStartedUnixMs = "recording_started_unix_ms"
        case appRunning = "app_running"
        case onMeetingTab = "on_meeting_tab"
        case scheduledEndUnixMs = "scheduled_end_unix_ms"
        case audioActive = "audio_active"
        case captureSessionActive = "capture_session_active"
    }
}

/// Mirrors `api::meetings::end_rule::EndState`: the clocks carried between polls.
struct CoreMeetingEndState: Codable, Equatable {
    var silenceStartedUnixMs: Int64?
    var browserTabGoneStartedUnixMs: Int64?
    var captureReleasedStartedUnixMs: Int64?

    enum CodingKeys: String, CodingKey {
        case silenceStartedUnixMs = "silence_started_unix_ms"
        case browserTabGoneStartedUnixMs = "browser_tab_gone_started_unix_ms"
        case captureReleasedStartedUnixMs = "capture_released_started_unix_ms"
    }
}

/// How long work waits on `CoreMemoryStore.accessQueue` before it runs. Every call into
/// the store shares that one serial queue, and some (meeting summaries, routine runs,
/// episode and thread summaries, title generation) hold it while waiting on an AI
/// provider. Once a second this enqueues an empty job and records how long it waited;
/// once a minute it logs `[queue-wait] jobs= p50_ms= p95_ms= max_ms=`, the same shape as
/// Windows' `store queue wait` line. That data decides whether the Mac needs a second
/// store connection for slow calls, as Windows added in #164. A probe still waiting
/// isn't re-enqueued, so a long hold shows up as one large wait, not a pile of probes.
enum AccessQueueWaitProbe {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "core-bridge")
    private static let queue = DispatchQueue(label: "com.mpointer.kestrel.queue-wait-probe", qos: .utility)
    private static var timer: DispatchSourceTimer?
    private static var waitsMs: [Double] = []
    private static var inFlight = false
    private static var lastLog = Date()

    static func start() {
        queue.async {
            guard timer == nil else { return }
            let source = DispatchSource.makeTimerSource(queue: queue)
            source.schedule(deadline: .now() + 1, repeating: 1)
            source.setEventHandler(handler: tick)
            source.resume()
            timer = source
        }
    }

    private static func tick() {
        if !inFlight {
            inFlight = true
            let enqueued = DispatchTime.now()
            CoreMemoryStore.accessQueue.async {
                let waited = Double(DispatchTime.now().uptimeNanoseconds - enqueued.uptimeNanoseconds) / 1_000_000
                queue.async {
                    waitsMs.append(waited)
                    inFlight = false
                }
            }
        }
        guard Date().timeIntervalSince(lastLog) >= 60 else { return }
        lastLog = Date()
        if let s = summarize(waitsMs) {
            log.notice("[queue-wait] jobs=\(s.jobs, privacy: .public) p50_ms=\(s.p50, privacy: .public) p95_ms=\(s.p95, privacy: .public) max_ms=\(s.max, privacy: .public)")
        }
        waitsMs.removeAll()
    }

    /// Nearest-rank percentiles, in whole milliseconds. Nil for no samples.
    static func summarize(_ waitsMs: [Double]) -> (jobs: Int, p50: Int, p95: Int, max: Int)? {
        guard !waitsMs.isEmpty else { return nil }
        let sorted = waitsMs.sorted()
        func rank(_ p: Double) -> Int { Int(sorted[min(sorted.count - 1, Int((p * Double(sorted.count)).rounded(.up)) - 1)].rounded()) }
        return (sorted.count, rank(0.5), rank(0.95), Int(sorted.last!.rounded()))
    }
}

// MARK: - Live transcription (docs/superpowers/specs/2026-10-06-live-transcription-design.md)

/// A provisional transcript row written during a recording (`whimbrel_live_list_segments`).
struct CoreLiveSegment: Decodable, Identifiable {
    let id: Int64
    let meetingId: Int64
    let channel: String
    let chunkIndex: Int64
    let startCs: Int64
    let endCs: Int64
    let speakerLabel: String
    let text: String

    enum CodingKeys: String, CodingKey {
        case id, channel, text
        case meetingId = "meeting_id"
        case chunkIndex = "chunk_index"
        case startCs = "start_cs"
        case endCs = "end_cs"
        case speakerLabel = "speaker_label"
    }
}

/// A whisper model kept loaded for the length of a meeting.
final class CoreLiveTranscriber {
    private let handle: OpaquePointer

    init?(modelPath: String) {
        guard let h = modelPath.withCString({ whimbrel_live_transcriber_new($0) }) else { return nil }
        handle = h
    }

    deinit { whimbrel_live_transcriber_free(handle) }

    /// The segments JSON (timestamps offset by `startOffsetCs`), or nil on failure.
    func transcribe(samples: [Float], startOffsetCs: Int64, threads: Int32) -> String? {
        guard !samples.isEmpty else { return "[]" }
        guard let json = samples.withUnsafeBufferPointer({ buf in
            whimbrel_live_transcribe_chunk(handle, buf.baseAddress, buf.count, startOffsetCs, threads)
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        return String(cString: json)
    }
}

extension CoreMemoryStore {
    /// Where to cut a growing 16 kHz mono buffer, or nil while it is too early.
    static func liveNextCut(samples: [Float], sampleRate: UInt32) -> (at: Int, overlap: Int)? {
        guard !samples.isEmpty else { return nil }
        guard let json = samples.withUnsafeBufferPointer({ buf in
            whimbrel_live_next_cut(buf.baseAddress, buf.count, sampleRate)
        }) else { return nil }
        defer { whimbrel_core_free_string(json) }
        struct Cut: Decodable { let at: Int; let overlap: Int }
        guard let cut = try? JSONDecoder().decode(Cut.self, from: Data(String(cString: json).utf8)) else { return nil }
        return (cut.at, cut.overlap)
    }

    /// Stores one transcribed chunk as provisional live rows (local only).
    func insertLiveSegments(meetingId: Int64, channel: String, chunkIndex: Int64, segmentsJSON: String, speakerLabel: String) -> Bool {
        channel.withCString { c in
            segmentsJSON.withCString { s in
                speakerLabel.withCString { l in
                    whimbrel_live_insert_segments(handle, meetingId, c, chunkIndex, s, l)
                }
            }
        }
    }

    func listLiveSegments(meetingId: Int64) -> [CoreLiveSegment] {
        guard let json = whimbrel_live_list_segments(handle, meetingId) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode([CoreLiveSegment].self, from: Data(String(cString: json).utf8))) ?? []
    }

    @discardableResult
    func clearLiveSegments(meetingId: Int64) -> Int {
        Int(whimbrel_live_clear_segments(handle, meetingId))
    }

    @discardableResult
    func purgeStaleLiveSegments(nowUnixMs: Int64) -> Int {
        Int(whimbrel_live_purge_stale(handle, nowUnixMs))
    }
}

// MARK: - Profile suggestions

/// A fact Whimbrel proposes for the user's Profile, from a day's meeting summaries. Pending until
/// the user adds or dismisses it. See docs/superpowers/specs/2026-10-06-profile-design.md.
struct CoreProfileSuggestion: Decodable, Identifiable, Equatable {
    let id: Int64
    let text: String
    let sourceMeetingId: Int64?

    enum CodingKeys: String, CodingKey {
        case id, text, sourceMeetingId = "source_meeting_id"
    }
}

extension CoreMemoryStore {
    func listProfileSuggestions() -> [CoreProfileSuggestion] {
        guard let json = whimbrel_profile_list_suggestions(handle) else { return [] }
        defer { whimbrel_core_free_string(json) }
        return (try? JSONDecoder().decode([CoreProfileSuggestion].self, from: Data(String(cString: json).utf8))) ?? []
    }

    @discardableResult
    func dismissProfileSuggestion(id: Int64) -> Bool { whimbrel_profile_dismiss_suggestion(handle, id) }

    @discardableResult
    func deleteProfileSuggestion(id: Int64) -> Bool { whimbrel_profile_delete_suggestion(handle, id) }

    /// Runs the once-a-day suggestion job (core owns the day gate and every failure rule).
    /// Returns how many suggestions were added, or nil when the call failed.
    func profileSuggestRun(nowUnixMs: Int64, ollamaModel: String, routing: CoreRoutingConfig, facts: [String], gate: RoutineGate) -> Int? {
        guard let routingJSON = try? JSONEncoder().encode(routing), let routingString = String(data: routingJSON, encoding: .utf8),
              let factsJSON = try? JSONEncoder().encode(facts), let factsString = String(data: factsJSON, encoding: .utf8)
        else { return nil }
        guard let out = ollamaModel.withCString({ m in
            routingString.withCString { r in
                factsString.withCString { f in
                    whimbrel_profile_suggest_run(handle, nowUnixMs, m, r, f, gate.rawValue)
                }
            }
        }) else { return nil }
        defer { whimbrel_core_free_string(out) }
        struct Reply: Decodable { let added: Int }
        return (try? JSONDecoder().decode(Reply.self, from: Data(String(cString: out).utf8)))?.added
    }
}

/// Skins Release 2 (core/ffi `whimbrel_skin_*`). Each call opens its own store at `dbPath` (except
/// the fetch) and blocks on the network, so call them off the main thread. `routing` carries the
/// spend caps (R6). A bare-NULL return (malformed arguments) and an undecodable envelope are both
/// `.providerFailure`; image bytes arrive base64-encoded and are decoded here.
enum CoreSkinMaker {
    private static func call<T: Decodable>(_ json: UnsafeMutablePointer<CChar>?) -> Result<T, CoreLLMError> {
        guard let json else { return .failure(.providerFailure) }
        defer { whimbrel_core_free_string(json) }
        return CoreLLMEnvelope<T>.result(json)
    }

    private static func bytes(_ result: Result<String, CoreLLMError>) -> Result<Data, CoreLLMError> {
        result.flatMap { Data(base64Encoded: $0).map { .success($0) } ?? .failure(.providerFailure) }
    }

    private static func json(_ routing: CoreRoutingConfig) -> String {
        (try? JSONEncoder().encode(routing)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }

    static func turn(dbPath: String, provider: String, model: String, apiKey: String?, input: Data, routing: CoreRoutingConfig)
        -> Result<SkinTurnReplyDTO, CoreLLMError> {
        let inputString = String(decoding: input, as: UTF8.self)
        let out: UnsafeMutablePointer<CChar>? = dbPath.withCString { db in
            provider.withCString { p in
                model.withCString { m in
                    inputString.withCString { i in
                        json(routing).withCString { r in
                            if let apiKey { return apiKey.withCString { k in whimbrel_skin_turn(db, p, m, k, i, r) } }
                            return whimbrel_skin_turn(db, p, m, nil, i, r)
                        }
                    }
                }
            }
        }
        return call(out)
    }

    static func searchImages(dbPath: String, apiKey: String, query: String, routing: CoreRoutingConfig) -> Result<[ImageCandidateDTO], CoreLLMError> {
        call(dbPath.withCString { db in
            apiKey.withCString { k in query.withCString { q in json(routing).withCString { r in whimbrel_skin_image_search(db, k, q, r) } } }
        })
    }

    static func generateImage(dbPath: String, provider: String, apiKey: String, prompt: String, routing: CoreRoutingConfig) -> Result<Data, CoreLLMError> {
        bytes(call(dbPath.withCString { db in
            provider.withCString { p in
                apiKey.withCString { k in prompt.withCString { q in json(routing).withCString { r in whimbrel_skin_image_generate(db, p, k, q, r) } } }
            }
        }))
    }

    /// `data` at most 1600 px on its long side, same format (unchanged when it fits); nil when core
    /// can't decode it. CPU-bound: call off the main thread.
    static func downscale(_ data: Data) -> Data? {
        try? bytes(call(data.base64EncodedString().withCString { whimbrel_skin_downscale($0) })).get()
    }

    /// The guarded download (https:443, public addresses only, image bytes only); no charge.
    static func fetchImage(_ url: String) -> Result<Data, CoreLLMError> {
        bytes(call(url.withCString { whimbrel_skin_image_fetch($0) }))
    }
}

/// What `whimbrel_skin_sync_reconcile` changed in the skins folder, and whether every skin should be sent again.
struct SkinReconcileReport: Decodable, Equatable {
    var changed: [String] = []
    var deleted: [String] = []
    var pushAll = false
    /// Why the reconcile failed (the flags were still taken); nil when it worked.
    var error: String?

    enum CodingKeys: String, CodingKey { case changed, deleted, pushAll = "push_all", error }
}
