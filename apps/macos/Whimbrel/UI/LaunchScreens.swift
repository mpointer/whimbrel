import AppKit
import SwiftUI

// The launch window (X4 Task 11b-2): the screens `LaunchCoordinator` stops on, the new-install
// recovery setup and the migration progress. The presenter turns the coordinator's
// `present`/`runOnboarding` calls into one observable `stage` and the user's buttons back
// into answers. Every blocking screen offers Quit and nothing that starts fresh, creates a
// new database or deletes one. Copy comes from the typed values only: `LaunchDiagnostic`
// text is never shown.

/// What the launch window shows now.
enum LaunchStage {
    /// Nothing to show yet (the plain path never leaves this).
    case starting
    case onboarding(EncryptionSetupModel)
    case migrating(Double)
    case recovery(EncryptionRecoveryModel)
    case keyUnavailable(EncryptionKeyUnavailableModel)
    /// Any other screen, drawn by `LaunchBlockingScreenView`.
    case screen(LaunchScreen)
}

@MainActor
final class LaunchUIPresenter: ObservableObject, LaunchPresenter {
    @Published private(set) var stage: LaunchStage = .starting
    /// Set by the first screen, onboarding or migration progress: the launch needs a window.
    @Published private(set) var needsWindow = false
    /// The user answered and the coordinator is working: the screen's buttons are disabled.
    @Published private(set) var busy = false

    /// Makes the setup's collaborators (live: `EncryptionSetupModel.Actions.live`).
    private let setupActions: (_ existingKey: Data?) -> EncryptionSetupModel.Actions
    /// Makes the recovery screen's collaborators around this presenter's `useKey`.
    private let recoveryActions: (_ useKey: @escaping (Data) async -> EncryptionRecoveryModel.UseKeyResult)
        -> EncryptionRecoveryModel.Actions

    /// A blocking screen or the onboarding waits for the user (the menu bar says Whimbrel
    /// needs attention, and the launch window comes forward).
    @Published private(set) var waitingForUser = false
    /// Quit was pressed in the setup after its recovery file or words were made: ask first.
    @Published private(set) var confirmingQuit = false
    /// The line above the onboarding when this launch follows Delete everything
    /// (`EncryptionCopy.deletedLibraryLine`, Task 13c item 1); nil otherwise.
    @Published private(set) var onboardingNotice: String?
    /// Reads and removes the one-shot Delete everything marker (`FreshLibraryMarker`). Set by
    /// `AppLaunch` on its environment's defaults; none by default.
    var consumeFreshLibraryMarker: () -> Bool = { false }

    private var answer: CheckedContinuation<LaunchScreenResponse, Never>? { didSet { updateWaiting() } }
    private var onboarding: CheckedContinuation<Data?, Never>? { didSet { updateWaiting() } }
    /// A restored key handed to the coordinator: answered by the next screen (rejected) or
    /// by the open (opened).
    private var useKeyResult: CheckedContinuation<EncryptionRecoveryModel.UseKeyResult, Never>?
    /// The key-unavailable screen's Retry: answered by the next screen or the open.
    private var retryResult: CheckedContinuation<Bool, Never>?

    init(setupActions: @escaping (Data?) -> EncryptionSetupModel.Actions,
         recoveryActions: @escaping (@escaping (Data) async -> EncryptionRecoveryModel.UseKeyResult) -> EncryptionRecoveryModel.Actions) {
        self.setupActions = setupActions
        self.recoveryActions = recoveryActions
    }

    private func updateWaiting() {
        let now = answer != nil || onboarding != nil
        if waitingForUser != now { waitingForUser = now }
    }

    // MARK: LaunchPresenter

    func present(_ screen: LaunchScreen) async -> LaunchScreenResponse {
        needsWindow = true
        busy = false
        settlePending(nextScreen: screen)
        switch screen {
        case let .recovery(reason):
            // A wrong restored key shows the same screen again: keep its model (the user sees
            // "does not open this database" on it, from `useKey`).
            if case .recovery = stage {} else {
                let model = EncryptionRecoveryModel(reason: reason == .needsKey ? .keyMissing : .keyRejected,
                                                    actions: recoveryActions { [weak self] key in await self?.useKey(key) ?? .failed })
                stage = .recovery(model)
            }
        case .keyUnavailable:
            if case .keyUnavailable = stage {} else {
                // Never the diagnostic text: the detail line stays empty.
                stage = .keyUnavailable(EncryptionKeyUnavailableModel(detail: "") { [weak self] in await self?.retryKey() ?? false })
            }
        default:
            stage = .screen(screen)
        }
        return await withCheckedContinuation { answer = $0 }
    }

    func runOnboarding(existingKey: Data?) async -> Data? {
        needsWindow = true
        busy = false
        settlePending(nextScreen: nil)
        // The first onboarding screen after Delete everything says the library is new: the
        // marker is consumed when it shows (a second onboarding in this launch keeps the line).
        if consumeFreshLibraryMarker() { onboardingNotice = EncryptionCopy.deletedLibraryLine }
        let model = EncryptionSetupModel(actions: setupActions(existingKey)) { [weak self] key in
            self?.finishOnboarding(key)
        }
        stage = .onboarding(model)
        return await withCheckedContinuation { onboarding = $0 }
    }

    nonisolated func migrationProgress(_ progress: CoreEncryptionMigration.Progress) {
        let fraction = progress.overall
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.showMigration(fraction) }
        }
    }

    // MARK: Answers from the window

    /// The user's answer to the screen on show. Ignored when nothing is waiting.
    func respond(_ response: LaunchScreenResponse) {
        guard let a = answer else { return }
        answer = nil
        busy = true
        a.resume(returning: response)
    }

    /// Quitting now would lose a fresh key's recovery file or words already made (the next
    /// launch would make new words, and the saved ones would stop working).
    var quitNeedsConfirmation: Bool {
        guard onboarding != nil, case let .onboarding(model) = stage else { return false }
        return model.material != nil && !model.actions.reusesKey
    }

    /// The setup's Quit (and Cmd-Q or the menu bar's Quit, through `AppTerminationGuard`):
    /// asks first when `quitNeedsConfirmation`.
    func requestQuit() {
        if quitNeedsConfirmation {
            confirmingQuit = true
        } else {
            quit()
        }
    }

    func cancelQuit() { confirmingQuit = false }

    /// Quit from any blocking screen or the onboarding.
    func quit() {
        confirmingQuit = false
        if let o = onboarding {
            onboarding = nil
            o.resume(returning: nil)
        }
        respond(.quit)
    }

    /// The launch ended (opened or quit): a recovery or retry still waiting learns the result.
    func launchFinished(opened: Bool) {
        // Opened without an onboarding (the keys could not all be deleted): the marker must
        // never show at a later onboarding.
        if opened { _ = consumeFreshLibraryMarker() }
        useKeyResult.map { $0.resume(returning: opened ? .opened : .failed) }
        useKeyResult = nil
        retryResult.map { $0.resume(returning: opened) }
        retryResult = nil
        busy = false
    }

    // MARK: Private

    private func finishOnboarding(_ key: Data) {
        guard let o = onboarding else { return }
        onboarding = nil
        busy = true
        stage = .starting
        o.resume(returning: key)
    }

    private func showMigration(_ fraction: Double) {
        needsWindow = true
        if case let .migrating(old) = stage, old > fraction { return }
        stage = .migrating(fraction)
    }

    /// The recovery screen's unlock: hand the key over, then wait for what it did.
    private func useKey(_ key: Data) async -> EncryptionRecoveryModel.UseKeyResult {
        guard answer != nil else { return .failed }
        return await withCheckedContinuation { c in
            useKeyResult = c
            respond(.restoredKey(key))
        }
    }

    /// The key-unavailable screen's Retry: true when the launch moved past that screen.
    private func retryKey() async -> Bool {
        guard answer != nil else { return false }
        return await withCheckedContinuation { c in
            retryResult = c
            respond(.retry)
        }
    }

    /// A new screen (or the onboarding) arrived: answer what the previous screen still waits for.
    private func settlePending(nextScreen: LaunchScreen?) {
        if let u = useKeyResult {
            useKeyResult = nil
            if case .recovery? = nextScreen { u.resume(returning: .keyRejected) } else { u.resume(returning: .failed) }
        }
        if let r = retryResult {
            retryResult = nil
            if case .keyUnavailable? = nextScreen { r.resume(returning: false) } else { r.resume(returning: true) }
        }
    }
}

// MARK: - Views

/// The launch window's content for the presenter's stage.
struct LaunchWindowView: View {
    @ObservedObject var presenter: LaunchUIPresenter
    @Environment(\.redesignEnabled) private var redesign

    var body: some View {
        Group {
            switch presenter.stage {
            case .starting:
                if redesign { LaunchStartingRedesignView() } else { LaunchStartingView() }
            case let .onboarding(model):
                VStack(spacing: 0) {
                    if let notice = presenter.onboardingNotice {
                        LaunchOnboardingNotice(text: notice)
                    }
                    if redesign {
                        KeySetupRedesignView(model: model, leave: .init(title: EncryptionCopy.quitAction, isEscape: false, isEnabled: !presenter.busy, run: { presenter.requestQuit() }))
                    } else {
                        EncryptionSetupView(model: model)
                        LaunchQuitBar(disabled: presenter.busy) { presenter.requestQuit() }
                    }
                }
                .alert(EncryptionCopy.quitSetupTitle, isPresented: Binding(
                    get: { presenter.confirmingQuit }, set: { if !$0 { presenter.cancelQuit() } })) {
                    Button(EncryptionCopy.quitAction, role: .destructive) { presenter.quit() }
                    Button(EncryptionCopy.keepSettingUp, role: .cancel) { presenter.cancelQuit() }
                } message: {
                    Text(EncryptionCopy.quitSetupBody)
                }
            case let .migrating(fraction):
                if redesign { LaunchMigrationRedesignView(fraction: fraction) } else { LaunchMigrationView(fraction: fraction) }
            case let .recovery(model):
                if redesign {
                    RecoveryRedesignView(model: model, quit: { presenter.quit() })
                } else {
                    VStack(spacing: 0) {
                        EncryptionRecoveryView(model: model)
                        LaunchQuitBar(disabled: model.working) { presenter.quit() }
                    }
                }
            case let .keyUnavailable(model):
                if redesign {
                    KeyUnavailableRedesignView(model: model, quit: { presenter.quit() })
                } else {
                    VStack(spacing: 0) {
                        EncryptionKeyUnavailableView(model: model)
                        LaunchQuitBar(disabled: model.retrying) { presenter.quit() }
                    }
                }
            case let .screen(screen):
                if redesign {
                    LaunchBlockingRedesignView(screen: screen, busy: presenter.busy) { presenter.respond($0) }
                } else {
                    LaunchBlockingScreenView(screen: screen, busy: presenter.busy) { presenter.respond($0) }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.whimbrelBackgroundPage)
        // A screen that needs the user brings the app forward (the menu bar's observer also
        // opens this window when it was closed).
        .onChange(of: presenter.waitingForUser, initial: true) { _, waiting in
            if waiting { NSApp.activate(ignoringOtherApps: true) }
        }
        // No quit on disappear: `ThemedRoot` re-creates this view when the theme changes. A
        // closed launch window comes back from the menu bar ("Show Whimbrel"), which also
        // offers Quit (`LaunchMenuBarView`).
    }
}

/// One line above the launch window's onboarding (after Delete everything). Not inside
/// `EncryptionSetupView`, which the Settings opt-in shows too.
struct LaunchOnboardingNotice: View {
    let text: String

    var body: some View {
        HStack(spacing: Spacing.sm) {
            Image(systemName: "info.circle").foregroundStyle(Color.whimbrelTextSecondary)
            Text(text).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, Spacing.xl)
        .padding(.vertical, Spacing.sm)
        .background(Color.whimbrelBackgroundCard)
    }
}

/// Before anything needs the user: the page colour, then a spinner if it takes a while.
struct LaunchStartingView: View {
    @State private var slow = false

    var body: some View {
        ZStack {
            Color.whimbrelBackgroundPage
            if slow {
                VStack(spacing: Spacing.sm) {
                    ProgressView().controlSize(.small)
                    Text(EncryptionCopy.starting).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
        }
        .task {
            try? await Task.sleep(for: .seconds(1))
            slow = true
        }
    }
}

struct LaunchQuitBar: View {
    var disabled = false
    let quit: () -> Void

    var body: some View {
        HStack {
            Button(EncryptionCopy.quitAction, action: quit).buttonStyle(.whimbrelSecondary).disabled(disabled)
            Spacer()
        }
        .padding(.horizontal, Spacing.xl)
        .padding(.vertical, Spacing.md)
        .background(Color.whimbrelBackgroundPage)
    }
}

/// The migration at launch: a determinate bar and plain words, no Quit (quitting mid-copy is
/// what the progress text asks the user not to do; a crash is still safe).
struct LaunchMigrationView: View {
    let fraction: Double

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text(EncryptionCopy.migratingTitle).font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            ProgressView(value: min(max(fraction, 0), 1)).frame(maxWidth: 480)
            Text(EncryptionCopy.migratingBody).font(.whimbrelBody()).foregroundStyle(Color.whimbrelTextPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Text(EncryptionCopy.migratingInterruptHint).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
        }
        .padding(Spacing.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color.whimbrelBackgroundPage)
    }
}

/// The text and buttons of a blocking screen, from the typed screen alone.
struct LaunchScreenContent: Equatable {
    var title: String
    var paragraphs: [String]
    /// Shown smaller, under the paragraphs.
    var hints: [String] = []
    var retry = false
    /// "Put back the unencrypted copy…" (asks first).
    var restoreCopy = false
    /// "Reset encryption status…" (asks first; the confirmation says it starts empty).
    var reset = false
    var resetConfirmBody: String?
    /// "Continue without the memory database" (a failed plain open only).
    var continueWithoutStore = false
    /// The last press of a file action that did not finish.
    var lastAction: String?

    static func of(_ screen: LaunchScreen) -> LaunchScreenContent {
        switch screen {
        case .keyStorageUnavailable:
            return .init(title: EncryptionCopy.keyStorageTitle, paragraphs: [EncryptionCopy.keyStorageBody],
                         hints: [EncryptionCopy.keyStorageHint], retry: true)
        case .keyUnavailable:
            return .init(title: EncryptionCopy.unavailableTitle, paragraphs: [EncryptionCopy.unavailableBody],
                         hints: [EncryptionCopy.unavailableHint, EncryptionCopy.unavailableTouchesNothing], retry: true)
        case let .recovery(reason):
            return .init(title: EncryptionCopy.recoveryTitle,
                         paragraphs: [reason == .needsKey ? EncryptionCopy.recoveryKeyMissing : EncryptionCopy.recoveryKeyRejected,
                                      EncryptionCopy.recoveryUnchanged])
        case .migrationInterrupted:
            return .init(title: EncryptionCopy.interruptedTitle, paragraphs: [EncryptionCopy.interruptedBody], retry: true)
        case .damaged(.damaged):
            return .init(title: EncryptionCopy.recoveryTitle, paragraphs: [EncryptionCopy.damagedBody],
                         hints: [EncryptionCopy.damagedHint])
        case .damaged(.noAccess):
            return .init(title: EncryptionCopy.recoveryTitle, paragraphs: [EncryptionCopy.noAccessBody],
                         hints: [EncryptionCopy.noAccessHint], retry: true)
        case .damaged(.openElsewhere):
            return .init(title: EncryptionCopy.recoveryTitle, paragraphs: [EncryptionCopy.openElsewhereBody],
                         hints: [EncryptionCopy.openElsewhereHint], retry: true)
        case let .encryptedFileMissing(backup, copy, last):
            if backup {
                return .init(title: EncryptionCopy.missingTitle, paragraphs: [EncryptionCopy.missingWithBackupBody],
                             hints: [EncryptionCopy.missingWithBackupHint], restoreCopy: true, lastAction: last.flatMap(lastActionText))
            }
            if copy {
                // Only the unfinished encrypted copy: nothing here changes a file (support case).
                return .init(title: EncryptionCopy.missingTitle, paragraphs: [EncryptionCopy.missingEncryptedCopyOnlyBody],
                             hints: [EncryptionCopy.supportNeeded])
            }
            // Neither copy: Reset (confirmed) sets the record aside; the app then starts empty.
            return .init(title: EncryptionCopy.missingTitle, paragraphs: [EncryptionCopy.missingNoCopyResetBody],
                         hints: [EncryptionCopy.missingNoCopyHint], reset: true, resetConfirmBody: EncryptionCopy.resetConfirmBody,
                         lastAction: last.flatMap(lastActionText))
        case let .error(error):
            return .init(title: EncryptionCopy.errorTitle, paragraphs: errorParagraphs(error),
                         hints: [EncryptionCopy.errorTryAgainHint], retry: true,
                         continueWithoutStore: error == .plainOpenFailed)
        }
    }

    private static func lastActionText(_ r: LaunchFileActionResult) -> String? {
        let action: CoreEncryptionFileAction
        switch r {
        case let .restore(result): action = result.action
        case let .reset(a): return a == .done ? nil : EncryptionCopy.resetStatusFailed
        }
        switch action {
        case .done: return nil
        case .partlyDone: return EncryptionCopy.restorePartlyDone
        case .refused, .invalidArgument: return EncryptionCopy.restoreRefused
        case .failed, .unknown: return EncryptionCopy.restoreCopyFailed
        }
    }

    private static func errorParagraphs(_ e: LaunchError) -> [String] {
        switch e {
        case .filesUnreadable: return [EncryptionCopy.errorFilesUnreadable, EncryptionCopy.errorStoppedSafely]
        case .plainOpenFailed: return [EncryptionCopy.errorOpenFailed]
        case let .keyedOpenFailed(outcome):
            switch outcome {
            case .startupFailed: return [EncryptionCopy.attentionStartupFailed]
            case .unsupported: return [EncryptionCopy.attentionUnsupported]
            case .damaged: return [EncryptionCopy.attentionDamaged]
            default: return [EncryptionCopy.errorOpenFailed]
            }
        case .onboardingKeyMismatch, .freshKeyRefused: return [EncryptionCopy.errorUnexpectedKey]
        case .keyMissing: return [EncryptionCopy.errorKeyGone]
        case .databaseMissingForRecoveredKey: return [EncryptionCopy.errorFileGoneForRecoveryKey]
        case .decisionUnavailable, .tooManyDecisions, .unknownStep, .stepOutOfOrder, .migrationCallFailed:
            return [EncryptionCopy.errorStoppedSafely]
        }
    }
}

/// Every screen but recovery and key-unavailable (which have their own views).
struct LaunchBlockingScreenView: View {
    let screen: LaunchScreen
    var busy = false
    let respond: (LaunchScreenResponse) -> Void
    @State private var confirmRestore = false
    @State private var confirmReset = false

    var body: some View {
        let c = LaunchScreenContent.of(screen)
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text(c.title).font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
            ForEach(c.paragraphs, id: \.self) { p in
                Text(p).font(.whimbrelBody()).foregroundStyle(Color.whimbrelTextPrimary).fixedSize(horizontal: false, vertical: true)
            }
            ForEach(c.hints, id: \.self) { h in
                Text(h).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary).fixedSize(horizontal: false, vertical: true)
            }
            if let last = c.lastAction {
                Text(last).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError).fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            HStack {
                Button(EncryptionCopy.quitAction) { respond(.quit) }.buttonStyle(.whimbrelSecondary).disabled(busy)
                Spacer()
                if c.restoreCopy {
                    Button(EncryptionCopy.restoreCopyAction) { confirmRestore = true }.buttonStyle(.whimbrelPrimary).disabled(busy)
                }
                if c.reset {
                    Button(EncryptionCopy.resetStatusEllipsis) { confirmReset = true }.buttonStyle(.whimbrelSecondary).disabled(busy)
                }
                if c.continueWithoutStore {
                    Button(EncryptionCopy.continueWithoutStore) { respond(.continueWithoutStore) }
                        .buttonStyle(.whimbrelSecondary).disabled(busy)
                }
                if c.retry {
                    Button(busy ? EncryptionCopy.trying : EncryptionCopy.tryAgain) { respond(.retry) }
                        .buttonStyle(.whimbrelPrimary).disabled(busy)
                }
            }
        }
        .padding(Spacing.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color.whimbrelBackgroundPage)
        .alert(EncryptionCopy.restoreConfirmTitle, isPresented: $confirmRestore) {
            Button(EncryptionCopy.restoreConfirm) { respond(.restoreRollbackCopy) }
            Button(EncryptionCopy.cancel, role: .cancel) {}
        } message: {
            Text(EncryptionCopy.restoreConfirmBody)
        }
        .alert(EncryptionCopy.resetConfirmTitle, isPresented: $confirmReset) {
            Button(EncryptionCopy.resetConfirm, role: .destructive) { respond(.resetEncryptionStatus) }
            Button(EncryptionCopy.cancel, role: .cancel) {}
        } message: {
            Text(c.resetConfirmBody ?? EncryptionCopy.resetConfirmBody)
        }
    }
}
