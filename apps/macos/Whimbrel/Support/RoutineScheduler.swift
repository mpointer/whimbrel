import Foundation
import OSLog
import UserNotifications

/// FR-RTN-1: wakes every 60 seconds and calls `whimbrel_routines_tick` (Task 4) via
/// `CoreMemoryStore.routinesTick`, which does all the real due-checking and execution
/// work in Rust. Structurally mirrors `RetentionScheduler` (a `Timer`-driven
/// `ObservableObject`), but ticks every 60s rather than once-then-24h -- daily-brief-
/// style wall-clock scheduling needs real per-minute granularity (RetentionScheduler's
/// "just re-run every 24h from launch" approach has no wall-clock awareness and
/// deliberately doesn't need it; a routine's `schedule_hhmm` does). 60s is also fine
/// grained enough to serve Phase B's pre-meeting-prep trigger later without changing
/// this interval.
///
/// Started unconditionally at launch (`AppState.init`, matching `retentionScheduler`,
/// not `mcpServerProcess`) -- there's no scheduler-level enable/disable toggle; the
/// toggle lives per-routine (Task 8's Settings section), on `enabled` in the `routine`
/// table itself. An empty/all-disabled routine list just means every tick's
/// `whimbrel_routines_tick` call quickly returns `[]`.
///
/// Task 9 adds the real write-action execution round trip for standing-authorized
/// (auto-approved) routines: after a tick's `whimbrel_routines_tick` call returns a run
/// left `PendingApproval` (Focus Time's calendar-block draft, or EOD Recap's email
/// draft), this scheduler checks whether that run's owning routine has
/// `autoApproveWriteActions` set and, if so, actually executes it against the real
/// Calendar/Gmail MCP connector -- see `executeWriteAction(runId:store:calendarToken:
/// gmailToken:)` below for the auth-failure-vs-terminal-failure distinction this
/// requires.
@MainActor
final class RoutineScheduler: ObservableObject {
    @Published private(set) var recentRuns: [CoreRoutineRun] = []
    /// FR-RTN-5..7 (Phase B approval gate): every routine run currently sitting in
    /// `PendingApproval`, refreshed after every tick and after every real write-action
    /// execution attempt -- the Pending Approvals UI's (Task 10) real data source.
    @Published private(set) var pendingApprovals: [CoreRoutineRun] = []

    private static let tickInterval: TimeInterval = 60
    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "routine-scheduler")
    private var timer: Timer?

    /// Read for the default-account lookups below (`resolveAccessTokens`'s
    /// auto-approve path, `resolvedRecipientEmail`) -- this scheduler never drives
    /// either connector's own connect/add-account/remove-account flow.
    private weak var calendarConnectorState: CalendarConnectorState?
    private weak var gmailConnectorState: GmailConnectorState?

    init() {}

    /// The specific WRITE scope each connector's write action actually needs, checked
    /// against what Google really granted before the action runs (see
    /// `fetchAccessToken`). Deliberately just the write scope, not the connector's full
    /// requested scope string -- a read scope like `calendar.readonly` being absent
    /// wouldn't stop `create_event` from working, so requiring it would block a
    /// perfectly capable grant.
    private static let calendarWriteScope = "https://www.googleapis.com/auth/calendar.events"
    private static let gmailWriteScope = "https://www.googleapis.com/auth/gmail.send"

    func start(
        store: CoreMemoryStore?, slowStore: CoreMemoryStore?, meetingsProvider: @escaping () -> [QualifyingMeeting], routing: RoutingSettings,
        promptLibrary: PromptLibrarySettings, meetingPrepLeadMinutes: @escaping () -> Double,
        calendarConnectorState: CalendarConnectorState, gmailConnectorState: GmailConnectorState,
        profile: ProfileSettings, profileSuggestions: ProfileSuggestionsModel,
        gate: @escaping () -> RoutineGate
    ) {
        self.calendarConnectorState = calendarConnectorState
        self.gmailConnectorState = gmailConnectorState
        stop()
        timer = Timer.scheduledTimer(withTimeInterval: Self.tickInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick(store: store, slowStore: slowStore, meetingsProvider: meetingsProvider, routing: routing, promptLibrary: promptLibrary, leadMinutes: meetingPrepLeadMinutes(), profile: profile, profileSuggestions: profileSuggestions, gate: gate()) }
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    private func tick(store: CoreMemoryStore?, slowStore: CoreMemoryStore?, meetingsProvider: () -> [QualifyingMeeting], routing: RoutingSettings, promptLibrary: PromptLibrarySettings, leadMinutes: Double, profile: ProfileSettings, profileSuggestions: ProfileSuggestionsModel, gate: RoutineGate) {
        guard let store, let slowStore else { return }
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let upcomingMeetings = meetingsProvider()
        let todayEvents = upcomingMeetings.map {
            CoreCalendarEventSummary(title: $0.title, startUnixMs: Int64($0.startDate.timeIntervalSince1970 * 1000))
        }
        // Same pattern TimelineView's askQuestion/dailyDigest call sites use
        // (TimelineView.swift:682/700): `defaultSelection()` gives the user's current
        // provider/model pick; the Ollama link in `build_chain` always uses the Ollama
        // model specifically, regardless of which provider is preferred.
        let defaults = routing.defaultSelection()
        let preferredModel = defaults.provider == .ollama ? defaults.ollamaModel : defaults.hostedModel
        let routingConfig = routing.routingConfig(
            preferredProvider: defaults.provider, preferredModel: preferredModel,
            dailyDigestInstructions: promptLibrary.instructions[.dailyDigest] ?? "",
            weeklyDigestInstructions: promptLibrary.instructions[.weeklyDigest] ?? ""
        )
        // Read on the main actor before hopping to the slow lane.
        let profileFacts = profile.factTexts
        let suggestEnabled = profile.suggestionsActive
        let gmailConnected = gmailConnectorState?.defaultEmail != nil
        let recipientEmail = resolvedRecipientEmail()

        // The tick can call an LLM, so it runs on the slow lane; `store` stays for
        // `handleTickResult`, which works on `accessQueue`.
        CoreMemoryStore.slowLaneQueue.async { [weak self] in
            let completed = slowStore.routinesTick(
                nowUnixMs: nowMs, ollamaModel: defaults.ollamaModel, routing: routingConfig, todayEvents: todayEvents,
                gmailConnected: gmailConnected, recipientEmail: recipientEmail,
                gate: gate
            )
            // Meeting Prep is event-driven (a brief shortly before each upcoming meeting), so it
            // has its own core entry point; the runs it created are announced like any others.
            let prepRuns = slowStore.meetingPrepRun(
                nowUnixMs: nowMs, leadMinutes: leadMinutes, ollamaModel: defaults.ollamaModel,
                routing: routingConfig, meetings: upcomingMeetings, gate: gate
            ) ?? []
            // Profile suggestions: core owns the once-a-day gate and the failure backoff, so asking every tick is cheap.
            let suggested = slowStore.profileSuggestRun(
                nowUnixMs: nowMs, ollamaModel: defaults.ollamaModel, routing: routingConfig, facts: profileFacts, gate: suggestEnabled ? gate : .off
            ) ?? 0
            let routines = slowStore.listRoutines() ?? []
            let pending = slowStore.pendingApprovals() ?? []
            DispatchQueue.main.async {
                self?.handleTickResult(completed: (completed ?? []) + prepRuns, routines: routines, pending: pending, store: store)
                if suggested > 0 { Task { await profileSuggestions.reload() } }
            }
        }
    }

    /// Who EOD Recap's drafted email gets addressed to: the DEFAULT connected Calendar
    /// account's email (the direct successor to the old single-account
    /// `CalendarConnectorState.status`'s `.connected(email:)` this used to read).
    private func resolvedRecipientEmail() -> String? {
        guard let email = calendarConnectorState?.defaultEmail, email.contains("@") else {
            return nil
        }
        return email
    }

    private func handleTickResult(completed: [CoreRoutineRun]?, routines: [CoreRoutine], pending: [CoreRoutineRun], store: CoreMemoryStore) {
        pendingApprovals = pending
        if let completed, !completed.isEmpty {
            handleCompletedRuns(completed, routines: routines)
        }

        // Drawn from the full CURRENT pending set, not just the runs this tick happened
        // to create. A standing-authorized draft that hit a retryable failure (no token
        // yet, Node.js not installed, connector spawn failure) stays `PendingApproval`
        // and would otherwise never be retried -- the user would have to approve it by
        // hand, defeating the point of standing authorization. Retrying every tick is
        // safe now that claiming is atomic: `planWriteAction`'s claim guarantees only one
        // execution of a given run ever proceeds, so a retry can't double-send.
        let dueForAutoApprove = pending.filter { run in
            run.status == "pending_approval" && routines.first(where: { $0.id == run.routineId })?.autoApproveWriteActions == true
        }
        guard !dueForAutoApprove.isEmpty else { return }
        Task { [weak self] in
            guard let self else { return }
            let (calendarToken, gmailToken) = await self.resolveAccessTokens(
                calendarEmail: self.calendarConnectorState?.defaultEmail,
                gmailEmail: self.gmailConnectorState?.defaultEmail
            )
            await self.executeIfAutoApproved(dueForAutoApprove, routines: routines, store: store, calendarToken: calendarToken, gmailToken: gmailToken)
        }
    }

    private func handleCompletedRuns(_ runs: [CoreRoutineRun], routines: [CoreRoutine]) {
        recentRuns = (runs + recentRuns).prefix(50).map { $0 }
        for run in runs {
            let routine = routines.first(where: { $0.id == run.routineId })
            // A run that landed in `pending_approval` and whose routine does NOT have
            // `autoApproveWriteActions` set will sit untouched in Pending Approvals
            // (`executeIfAutoApproved` below skips it) -- the user hasn't approved
            // anything yet, so the notification must not preview the drafted content
            // (`run.outputSummary`, e.g. the drafted email/event text); it should just
            // point them at Pending Approvals. A run whose routine DOES auto-approve is
            // about to be executed for real by `executeIfAutoApproved`/
            // `executeWriteAction` moments after this tick, so it keeps the normal
            // output-summary notification.
            let needsApproval = run.status == "pending_approval" && routine?.autoApproveWriteActions != true
            postNotification(for: run, routineName: routine?.name, needsApproval: needsApproval)
        }
    }

    private func postNotification(for run: CoreRoutineRun, routineName: String?, needsApproval: Bool) {
        let content = UNMutableNotificationContent()
        // The routine's own name, not a hardcoded one -- there are three templates now
        // (Daily Brief, Focus Time, EOD Recap), so a fixed "Daily Brief" title mislabels
        // two of them, including the approval prompts this feature exists to surface.
        content.title = routineName ?? "Routines"
        content.body = needsApproval ? "Needs your approval" : run.outputSummary
        content.sound = .default
        let request = UNNotificationRequest(identifier: "routine-run-\(run.id)", content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request) { [weak self] error in
            if let error {
                self?.log.error("Failed to post routine notification: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    /// Resolves the access token for one specific connected account's helper — `email:
    /// nil` (no default set yet, or `PendingApprovalsView` was given no selection) or an
    /// email with no matching helper both resolve to `nil`, same "no usable token"
    /// signal `executeWriteAction` already treats as an auth failure to retry later.
    /// Exposed (not `private`) so the Pending Approvals UI's manual "Approve" button can
    /// resolve tokens for whichever account the user picked, via the same real path this
    /// scheduler's own auto-approve dispatch uses.
    func resolveAccessTokens(calendarEmail: String?, gmailEmail: String?) async -> (calendarToken: String?, gmailToken: String?) {
        let calendarHelper = calendarEmail.flatMap { calendarConnectorState?.oauthHelper(forEmail: $0) }
        let gmailHelper = gmailEmail.flatMap { gmailConnectorState?.oauthHelper(forEmail: $0) }
        let calendarToken = await Self.fetchAccessToken(from: calendarHelper, requiredScope: Self.calendarWriteScope, log: log)
        let gmailToken = await Self.fetchAccessToken(from: gmailHelper, requiredScope: Self.gmailWriteScope, log: log)
        return (calendarToken, gmailToken)
    }

    private static func fetchAccessToken(from helper: GoogleOAuthHelper?, requiredScope: String, log: Logger) async -> String? {
        guard let helper else { return nil }
        do {
            return try await helper.validAccessToken(requiringGrantedScope: requiredScope)
        } catch let GoogleOAuthError.insufficientScope(required, granted) {
            log.error(
                "OAuth grant is missing a required scope (need \(required, privacy: .public), have \(granted, privacy: .public)) -- reconnect this account in Settings to re-consent; drafts stay pending until then"
            )
            return nil
        } catch {
            return nil
        }
    }

    private func executeIfAutoApproved(
        _ pendingRuns: [CoreRoutineRun], routines: [CoreRoutine], store: CoreMemoryStore, calendarToken: String?, gmailToken: String?
    ) async {
        for run in pendingRuns where run.status == "pending_approval" {
            guard let routine = routines.first(where: { $0.id == run.routineId }), routine.autoApproveWriteActions else { continue }
            await executeWriteAction(runId: run.id, store: store, calendarToken: calendarToken, gmailToken: gmailToken)
        }
    }

    /// Atomically CLAIMS the pending run's drafted write-action payload, executes it
    /// against the real connector (Calendar's `create_event` for a calendar draft,
    /// Gmail's `send_email` for an email draft), and reports the real outcome back to
    /// the core. Three phases, per ADR 0001 (core/api::write_action, M3.0 Task 7):
    /// `planWriteAction` claims and builds the connector call under the store queue;
    /// the MCP round trip below runs with no store access; `settleWriteAction` records
    /// the outcome under the store queue.
    ///
    /// The claim (a single atomic conditional `UPDATE ... RETURNING` in Rust) is what
    /// makes this safe to invoke concurrently for the same run: `accessQueue` serializes
    /// the FFI calls but NOT the slow MCP round trip between them, so only the caller
    /// that wins the claim gets a plan; any other gets `.notClaimable` and returns.
    /// Shared by this scheduler's auto-approve path AND the Pending Approvals UI's
    /// "Approve" button.
    ///
    /// `calendarToken`/`gmailToken` are pre-resolved by the caller
    /// (`resolveAccessTokens`); a `nil` token is the signal that the connector call would
    /// fail because the OAuth token is expired/invalid/unavailable, and leaves the run
    /// `PendingApproval` so the draft survives to retry once the user reconnects.
    /// Likewise a failure before `callTool` (Node.js missing, the connector script
    /// missing, spawn/handshake failure) never reached Google and is retryable. Only a
    /// `callTool` failure -- Google's API itself rejected the request -- is terminal.
    /// Returns a user-facing message when the run is left pending for a reason the
    /// caller should surface; `nil` on success, on a terminal failure (already
    /// recorded), or when the run simply wasn't claimable.
    @discardableResult
    func executeWriteAction(runId: Int64, store: CoreMemoryStore, calendarToken: String?, gmailToken: String?) async -> String? {
        let plan: CoreWriteActionPlan
        switch await Self.withStoreAccess({ store.planWriteAction(runId: runId) }) {
        case .notClaimable, .none:
            log.log("executeWriteAction: run \(runId) wasn't claimable -- already claimed by another approval, finalized, or rejected")
            return nil
        case .undecodable:
            log.error("executeWriteAction: couldn't decode pending action JSON for run \(runId); finalized as failed")
            await refreshPendingApprovals(store: store)
            return nil
        case let .planned(p):
            plan = p
        }

        let token = plan.connector == "google-calendar" ? calendarToken : gmailToken
        let outcome: CoreWriteActionOutcome
        if let token {
            outcome = await Self.performToolCall(plan: plan, accessToken: token)
        } else {
            log.log("executeWriteAction: no usable \(plan.connector, privacy: .public) access token for run \(runId) (missing, unrefreshable, or granted without the required scope -- reconnect in Settings) -- leaving pending for retry")
            outcome = .noToken
        }
        if case let .retryableFailure(reason) = outcome {
            log.log("executeWriteAction: run \(runId) hit a retryable failure (\(reason, privacy: .public)) -- leaving pending for retry")
        }

        let settled = await Self.withStoreAccess { store.settleWriteAction(runId: runId, plan: plan, outcome: outcome) }
        if settled?.finalized ?? false {
            await refreshPendingApprovals(store: store)
        }
        return settled?.userMessage
    }

    private func refreshPendingApprovals(store: CoreMemoryStore) async {
        pendingApprovals = await Self.withStoreAccess { store.pendingApprovals() } ?? []
    }

    /// Every `CoreMemoryStore` FFI call must be serialized through
    /// `CoreMemoryStore.accessQueue` (its own doc comment) -- never `.sync` from this
    /// `@MainActor` scheduler, which would block the main thread on the same queue
    /// background work also uses. Bridges that queue-based access into `async`/`await`
    /// via a one-shot continuation, matching this method's callers' needs exactly.
    private static func withStoreAccess<T>(_ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async {
                continuation.resume(returning: work())
            }
        }
    }

    /// The MCP round trip for one planned action, mirroring
    /// `CalendarConnectorState.performMcpRoundTrip`'s spawn/connect/callTool/disconnect
    /// pattern. Everything before `callTool` is local (spawn, handshake, tool discovery)
    /// and a failure there is retryable; `callTool` is the one step that reaches
    /// Google, so its failure is the terminal case. No store access anywhere here.
    private static func performToolCall(plan: CoreWriteActionPlan, accessToken: String) async -> CoreWriteActionOutcome {
        await Task.detached(priority: .userInitiated) {
            let connectorLabel = plan.connector == "google-calendar" ? "Calendar" : "Gmail"
            guard let nodePath = McpNodeLocator.nodeExecutablePath() else {
                return .retryableFailure(reason: "Node.js isn't installed on this Mac.")
            }
            let scriptPath = plan.connector == "google-calendar" ? McpNodeLocator.calendarConnectorScriptPath() : McpNodeLocator.gmailConnectorScriptPath()
            guard let scriptPath else {
                return .retryableFailure(reason: "The \(connectorLabel) connector's files weren't found in this build.")
            }
            guard let host = McpHost() else {
                return .retryableFailure(reason: "Couldn't start the connector host.")
            }
            let args = McpNodeLocator.nodeNeedsEnvWrapper(resolvedPath: nodePath) ? ["node", scriptPath] : [scriptPath]
            guard let connectorId = host.connect(command: nodePath, args: args, env: [(plan.tokenEnv, accessToken)]) else {
                return .retryableFailure(reason: "Couldn't start the \(connectorLabel) connector process.")
            }
            defer { host.disconnect(connectorId: connectorId) }

            guard host.listTools(connectorId: connectorId)?.contains(where: { $0.name == plan.tool }) == true else {
                return .retryableFailure(reason: "Connected, but the connector didn't report its expected capability.")
            }
            guard let resultText = host.callTool(connectorId: connectorId, toolName: plan.tool, argumentsJSON: plan.argsJson) else {
                return .terminalFailure
            }
            return .success(resultJson: resultText)
        }.value
    }
}
