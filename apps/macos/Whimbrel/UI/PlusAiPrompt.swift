import SwiftUI

/// The one-time "use Whimbrel AI?" question for a Plus account (design:
/// docs/superpowers/specs/2026-11-19-plus-whimbrel-ai-first-use-design.md). Mac-only on
/// purpose: `SyncBilling.swift` is shared with the iPhone app, whose string catalog must not
/// gain these keys.
enum PlusAiPrompt {
    /// "plus" only for a plan `AccountPlan.isPlus` accepts (Cloud and tier Plus), else nil.
    static func planTier(of plan: AccountPlan?) -> String? { plan?.isPlus == true ? "plus" : nil }

    /// Whether to ask now. Every condition must hold; each one alone blocks the sheet.
    /// `hasWhimbrelCredentials` is what Whimbrel AI needs to run (`RoutingSettings.hasKey(for: .whimbrel)`).
    static func shouldShow(
        planTier: String?, isSignedIn: Bool, hasWhimbrelCredentials: Bool,
        preferredProvider: RoutingSettings.Provider, localOnlyMode: Bool,
        answered: Bool, isRecording: Bool, hasDataNote: Bool
    ) -> Bool {
        // No data note, no sheet: accepting must never happen without the disclosure.
        planTier == "plus" && isSignedIn && hasWhimbrelCredentials && hasDataNote
            && preferredProvider != .whimbrel && !localOnlyMode
            && !answered && !isRecording
    }

    /// The per-device "answered" flag. Not synced: a second Mac asks once, unless the
    /// provider already synced over as Whimbrel AI.
    struct Answered {
        /// In the whimbrel namespace (never the old app name; `scripts/rename-whimbrel.py --check`).
        static let key = "com.mpointer.whimbrel.plusAiPromptAnswered"
        private let defaults: UserDefaults
        init(defaults: UserDefaults = .standard) { self.defaults = defaults }
        var isSet: Bool {
            get { defaults.bool(forKey: Self.key) }
            nonmutating set { defaults.set(newValue, forKey: Self.key) }
        }
    }

    /// "Use Whimbrel AI": writes the provider through the same setter the Settings picker
    /// uses (so it syncs like any user change), then records the answer.
    @MainActor
    static func accept(routing: RoutingSettings, answered: Answered) {
        routing.preferredProvider = .whimbrel
        answered.isSet = true
    }

    /// "Keep my current setup": only records the answer. The provider is not written.
    static func decline(answered: Answered) {
        answered.isSet = true
    }

    enum Choice { case use, keep, notNow }

    /// What a button does. "Not now" records nothing (asked again next launch).
    @MainActor
    static func handle(_ choice: Choice, routing: RoutingSettings, answered: Answered) {
        switch choice {
        case .use: accept(routing: routing, answered: answered)
        case .keep: decline(answered: answered)
        case .notNow: break
        }
    }

    /// The inputs the host reads from live state, so the mapping is testable without a view.
    @MainActor
    static func wanted(
        plan: AccountPlan?, isSignedIn: Bool, routing: RoutingSettings,
        answered: Answered, isRecording: Bool, dataNote: String?
    ) -> Bool {
        shouldShow(
            planTier: planTier(of: plan), isSignedIn: isSignedIn,
            hasWhimbrelCredentials: routing.hasKey(for: .whimbrel),
            preferredProvider: routing.preferredProvider, localOnlyMode: routing.localOnlyMode,
            answered: answered.isSet, isRecording: isRecording, hasDataNote: dataNote != nil)
    }

    /// A Plus user who is already on Whimbrel AI is never asked, and is marked answered
    /// silently so switching away later does not start asking them.
    static func markAnsweredIfAlreadyOnWhimbrel(
        planTier: String?, preferredProvider: RoutingSettings.Provider, answered: Answered
    ) {
        if planTier == "plus", preferredProvider == .whimbrel, !answered.isSet { answered.isSet = true }
    }

    static let title = String(localized: "Plus includes a monthly Whimbrel AI allowance. Use it for your AI features?")
    static let footer = String(localized: "You can change this any time in Settings → AI. To keep a particular client's material away from hosted models, use Exclude from hosted models on that client's page.")
    /// The provider data note from core, the same text Settings → AI Providers shows.
    static func dataNote() -> String? {
        CoreMemoryStore.providers().first(where: { $0.id == "whimbrel" })?.dataTerms?.localizedText
    }
}

/// The sheet: "Use Whimbrel AI" is the default button. Closing it without choosing records
/// nothing, so it is asked again next launch.
struct PlusAiPromptSheet: View {
    let dataNote: String?
    let onUse: () -> Void
    let onKeep: () -> Void
    let onNotNow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text(PlusAiPrompt.title)
                .font(.whimbrelBody(.headline))
                .foregroundStyle(Color.whimbrelTextPrimary)
                .fixedSize(horizontal: false, vertical: true)
            if let dataNote {
                Text(dataNote)
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text(PlusAiPrompt.footer)
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(String(localized: "Not now"), action: onNotNow)
                    .keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier("plus-ai-not-now")
                Spacer()
                Button(String(localized: "Keep my current setup"), action: onKeep)
                    .accessibilityIdentifier("plus-ai-keep")
                Button(String(localized: "Use Whimbrel AI"), action: onUse)
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("plus-ai-use")
            }
        }
        .padding(Spacing.lg)
        .frame(width: 440)
        .background(Color.whimbrelBackgroundPage)
        .accessibilityIdentifier("plus-ai-prompt")
    }
}

/// Presents the sheet once the plan read has landed and every condition holds. It observes
/// the sync state and routing settings, so it reacts when the plan, sign-in, credentials,
/// provider, Local-only mode or recording state changes.
struct PlusAiPromptHost: ViewModifier {
    @EnvironmentObject private var appState: AppState
    @ObservedObject var syncState: SyncState
    @ObservedObject var routing: RoutingSettings
    @State private var showing = false
    /// Closed without choosing: not asked again until the next launch.
    @State private var closedThisSession = false
    @State private var dataNote: String?
    private let answered = PlusAiPrompt.Answered()

    private var wanted: Bool {
        var signedIn = false
        if case .ready = syncState.status { signedIn = true }
        return PlusAiPrompt.wanted(
            plan: syncState.plan, isSignedIn: signedIn, routing: routing, answered: answered,
            isRecording: appState.isRecordingNow, dataNote: dataNote)
    }

    func body(content: Content) -> some View {
        content
            .onAppear { dataNote = PlusAiPrompt.dataNote(); evaluate() }
            .onChange(of: wanted) { _, _ in evaluate() }
            .onChange(of: routing.preferredProvider) { _, _ in evaluate() }
            .sheet(isPresented: $showing, onDismiss: {
                // Closed without choosing ("Not now", Esc, or closed by the host when a recording
                // starts): not answered, so it is asked again next launch.
                if !answered.isSet { closedThisSession = true }
            }, content: {
                PlusAiPromptSheet(
                    dataNote: dataNote,
                    onUse: { PlusAiPrompt.handle(.use, routing: routing, answered: answered); showing = false },
                    onKeep: { PlusAiPrompt.handle(.keep, routing: routing, answered: answered); showing = false },
                    onNotNow: { PlusAiPrompt.handle(.notNow, routing: routing, answered: answered); showing = false })
            })
    }

    private func evaluate() {
        PlusAiPrompt.markAnsweredIfAlreadyOnWhimbrel(
            planTier: PlusAiPrompt.planTier(of: syncState.plan), preferredProvider: routing.preferredProvider, answered: answered)
        if showing, !wanted, !answered.isSet { showing = false; return }
        if wanted, !closedThisSession, !showing { showing = true }
    }
}

extension View {
    func plusAiPrompt(syncState: SyncState, routing: RoutingSettings) -> some View {
        modifier(PlusAiPromptHost(syncState: syncState, routing: routing))
    }
}
