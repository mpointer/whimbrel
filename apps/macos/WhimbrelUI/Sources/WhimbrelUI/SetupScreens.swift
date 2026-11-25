import AppKit
import SwiftUI

// The first run, launch and setup screens as plain views over value-type models. A model says what
// to show (text the app already owns arrives in it), and closures say what the buttons do. Nothing
// here knows the app: the app builds a model from its own state objects and calls its own functions
// from the closures.

// MARK: First run

/// The first run screen: the two permissions a first meeting needs, what recording looks like, and one button.
public struct WFirstRunModel {
    public var title = ""
    public var intro = ""
    public var groups: [WPermissionGroup] = []
    /// Shown under the actions.
    public var finePrint: [String] = []
    public var primaryTitle = ""
    public var onPrimary: () -> Void = {}

    public init() {}
}

public struct WFirstRunView: View {
    @Environment(\.wTheme) private var theme
    let model: WFirstRunModel

    public init(model: WFirstRunModel) { self.model = model }

    public var body: some View {
        WSetupPage(
            title: model.title, paragraph: model.intro,
            actions: [WScreenAction(id: "primary", title: model.primaryTitle, kind: .primary, run: model.onPrimary)],
            finePrint: model.finePrint
        ) {
            VStack(alignment: .leading, spacing: theme.space(20)) {
                ForEach(model.groups) { WPermissionGroupView(group: $0) }
                WRecordingPicture()
            }
        }
    }
}

/// What recording looks like, drawn with the real indicator, and the notice line. It says what to do,
/// never that anyone has been told or has agreed.
struct WRecordingPicture: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            HStack(alignment: .center, spacing: theme.space(12)) {
                WRecordingIndicator(state: .recording, size: .pill)
                Text(strings.onboarding.recordingCaption).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                WGlyph(name: "info.circle", size: 13, weight: .semibold).foregroundStyle(theme.accentPrimary)
                Text(strings.consentMessage).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
        }
        .padding(theme.space(14))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard()
    }
}

// MARK: Permission checklist

public struct WPermissionChecklistModel {
    public var title = ""
    public var intro = ""
    /// Required first, then optional.
    public var groups: [WPermissionGroup] = []
    /// "2 of 5 allowed".
    public var progress = ""
    /// Shown instead of nothing once every permission is allowed.
    public var allSet: String?
    public var refreshTitle = ""
    public var onRefresh: () -> Void = {}

    public init() {}
}

public struct WPermissionChecklistView: View {
    @Environment(\.wTheme) private var theme
    let model: WPermissionChecklistModel

    public init(model: WPermissionChecklistModel) { self.model = model }

    public var body: some View {
        WSetupPage(
            title: model.title, paragraph: model.intro,
            actions: [WScreenAction(id: "refresh", title: model.refreshTitle, kind: .secondary, run: model.onRefresh)]
        ) {
            VStack(alignment: .leading, spacing: theme.space(20)) {
                ForEach(model.groups) { WPermissionGroupView(group: $0) }
                HStack(spacing: theme.space(10)) {
                    Text(model.progress).wFont(.secondary).monospacedDigit().foregroundStyle(theme.textSecondary)
                    if let allSet = model.allSet { WSuccessLine(text: allSet) }
                }
            }
        }
    }
}

// MARK: Launch starting

/// Before anything needs the user: the page color, then a quiet bar and a line if it takes a while.
public struct WLaunchStartingView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var slow = false
    let message: String
    let delay: Duration

    public init(message: String, delay: Duration = .seconds(1)) {
        self.message = message
        self.delay = delay
    }

    public var body: some View {
        ZStack {
            theme.backgroundPage
            if slow {
                VStack(spacing: theme.space(10)) {
                    WProgressLine(label: message).frame(width: 160 * k)
                    Text(message).wFont(.secondary).foregroundStyle(theme.textSecondary)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .task {
            try? await Task.sleep(for: delay)
            slow = true
        }
    }
}

// MARK: Notice screens: blocking launch screens, migration, key unavailable, model setup

/// A screen of plain words and at most a few actions: a blocking launch screen, the migration
/// progress, the key-unavailable screen, model setup.
public struct WNoticeScreenModel {
    public var title = ""
    public var paragraphs: [String] = []
    /// Smaller, under the paragraphs.
    public var hints: [String] = []
    /// What went wrong with the last action, in plain words. Never raw system text.
    public var errorLine: String?
    /// A determinate bar when set.
    public var fraction: Double?
    /// A quiet bar under the title while the screen is working.
    public var busy = false
    public var actions: [WScreenAction] = []

    public init() {}
}

public struct WNoticeScreenView: View {
    @Environment(\.wTheme) private var theme
    let model: WNoticeScreenModel

    public init(model: WNoticeScreenModel) { self.model = model }

    public var body: some View {
        WSetupPage(title: model.title, busy: model.busy && model.fraction == nil, actions: model.actions) {
            VStack(alignment: .leading, spacing: theme.space(14)) {
                if let fraction = model.fraction { WProgressLine(fraction: fraction, label: model.title) }
                ForEach(Array(model.paragraphs.enumerated()), id: \.offset) { _, text in
                    Text(text).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                }
                ForEach(Array(model.hints.enumerated()), id: \.offset) { _, text in
                    Text(text).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
                if let error = model.errorLine { WErrorLine(text: error) }
            }
        }
    }
}

// MARK: Recovery key setup

/// Where an error line sits on the setup screen: under the passphrase, under the file step, under the test.
public enum WKeySetupErrorPlace: Equatable, Sendable { case passphrase, file, test }

/// The setup screen: choose a passphrase, make the recovery key, keep it (file, words), test, confirm.
public struct WKeySetupModel {
    public var title = ""
    public var intro = ""
    // Passphrase step
    public var passphraseHeading = ""
    public var passphraseHelp = ""
    public var passphraseLabel = ""
    public var confirmLabel = ""
    /// "4 of at least 12 characters".
    public var passphraseCount = ""
    public var meetsMinimum = false
    /// The problem with the pair, or the failure of the last create, or nil.
    public var passphraseProblem: String?
    public var passphraseLockedHint: String?
    public var passphraseLocked = false
    // The key exists
    public var hasKey = false
    public var words: [String] = []
    public var qrImage: NSImage?
    public var qrLabel = ""
    // File step
    public var fileHeading = ""
    public var fileHelp = ""
    public var saveFileTitle = ""
    public var savedLine: String?
    public var sameDiskWarning: String?
    public var saveFailedLine: String?
    // Words step
    public var wordsHeading = ""
    public var wordsNotice = ""
    public var printTitle = ""
    public var printedNote: String?
    public var printToFileWarning: String?
    public var wroteDownLabel = ""
    // Test step
    public var testHeading = ""
    public var testHelp = ""
    public var testLabel = ""
    public var testButtonTitle = ""
    public var canTest = false
    public var testPassedLine: String?
    public var testFailedLine: String?
    public var savedItLabel = ""
    public var savedItLockedHint: String?
    public var canConfirmSaved = false
    // Whole screen
    public var finePrint = ""
    /// "Create recovery key" while there is no key, the progress words while it is made.
    public var createTitle = ""
    public var canCreate = false
    public var creating = false
    public var continueTitle = ""
    public var canContinue = false
    public var startOverTitle = ""
    /// Quit or Cancel, when this screen can be left.
    public var leaveTitle: String?
    public var leaveIsEscape = false
    public var leaveEnabled = true
    public var onCreate: () -> Void = {}
    public var onSaveFile: () -> Void = {}
    public var onPrint: () -> Void = {}
    public var onTest: () -> Void = {}
    public var onStartOver: () -> Void = {}
    public var onContinue: () -> Void = {}
    public var onLeave: () -> Void = {}

    public init() {}

    /// The step the screen is on: 1 until the key exists, then 2.
    public var stepIndex: Int { hasKey ? 2 : 1 }

    /// The one primary action: Create until the key exists, then Continue.
    public var primary: WScreenAction {
        if hasKey {
            return WScreenAction(id: "continue", title: continueTitle, kind: .primary, isEnabled: canContinue, run: onContinue)
        }
        return WScreenAction(id: "create", title: createTitle, kind: .primary, isEnabled: canCreate, run: onCreate)
    }

    /// Quit or Cancel, then Start over once there is a key, then the primary (last).
    public var actions: [WScreenAction] {
        var list: [WScreenAction] = []
        if let leaveTitle {
            list.append(WScreenAction(id: "leave", title: leaveTitle, kind: .secondary, isEnabled: leaveEnabled, isCancel: leaveIsEscape, run: onLeave))
        }
        if hasKey { list.append(WScreenAction(id: "startOver", title: startOverTitle, kind: .secondary, run: onStartOver)) }
        list.append(primary)
        return list
    }

    /// Every error line the screen would show and the step it sits under.
    public var errorLines: [(place: WKeySetupErrorPlace, text: String)] {
        var lines: [(WKeySetupErrorPlace, String)] = []
        if let text = passphraseProblem { lines.append((.passphrase, text)) }
        if hasKey, let text = saveFailedLine { lines.append((.file, text)) }
        if hasKey, let text = testFailedLine { lines.append((.test, text)) }
        return lines
    }
}

public struct WKeySetupView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let model: WKeySetupModel
    let passphrase: Binding<String>
    let confirmation: Binding<String>
    let testPassphrase: Binding<String>
    let wroteDown: Binding<Bool>
    let savedConfirmed: Binding<Bool>

    public init(
        model: WKeySetupModel, passphrase: Binding<String>, confirmation: Binding<String>, testPassphrase: Binding<String>,
        wroteDown: Binding<Bool>, savedConfirmed: Binding<Bool>
    ) {
        self.model = model
        self.passphrase = passphrase
        self.confirmation = confirmation
        self.testPassphrase = testPassphrase
        self.wroteDown = wroteDown
        self.savedConfirmed = savedConfirmed
    }

    public var body: some View {
        WSetupPage(
            stepID: model.stepIndex, title: model.title, paragraph: model.intro, busy: model.creating,
            actions: model.actions, finePrint: [model.finePrint]
        ) {
            VStack(alignment: .leading, spacing: theme.space(16)) {
                passphraseStep
                if model.hasKey {
                    fileStep
                    wordsStep
                    testStep
                }
            }
        }
    }

    private func heading(_ text: String) -> some View {
        Text(text).wFont(.subtitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
    }

    private func step<Body: View>(@ViewBuilder _ body: () -> Body) -> some View {
        VStack(alignment: .leading, spacing: theme.space(12), content: body)
            .padding(theme.space(18))
            .frame(maxWidth: .infinity, alignment: .leading)
            .wCard()
            .accessibilityElement(children: .contain)
    }

    private var passphraseStep: some View {
        step {
            heading(model.passphraseHeading)
            WField(
                label: model.passphraseLabel, text: passphrase, help: model.passphraseHelp, count: model.passphraseCount,
                countIsMet: model.meetsMinimum, error: model.passphraseProblem, isSecure: true, isEnabled: !model.passphraseLocked
            )
            WField(label: model.confirmLabel, text: confirmation, isSecure: true, isEnabled: !model.passphraseLocked)
            if let hint = model.passphraseLockedHint, model.passphraseLocked {
                Text(hint).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var fileStep: some View {
        step {
            heading(model.fileHeading)
            Text(model.fileHelp).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            WButton(model.saveFileTitle, glyph: "square.and.arrow.down", kind: .secondary, action: model.onSaveFile)
            if let saved = model.savedLine {
                WSuccessLine(text: saved).lineLimit(3)
                if let warning = model.sameDiskWarning { WWarningLine(text: warning) }
            } else if let failed = model.saveFailedLine {
                WErrorLine(text: failed)
            }
        }
    }

    private var wordsStep: some View {
        step {
            heading(model.wordsHeading)
            HStack(alignment: .top, spacing: theme.space(10)) {
                WGlyph(name: "exclamationmark.triangle.fill", size: 14, weight: .semibold).foregroundStyle(theme.warning)
                Text(model.wordsNotice).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            }
            .padding(theme.space(12))
            .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.warning.opacity(0.14)))
            .overlay(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).strokeBorder(theme.warning, lineWidth: theme.isHighContrast ? 2 : 1))
            .accessibilityElement(children: .combine)
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: theme.space(16)) {
                    WWordList(words: model.words, label: model.wordsHeading)
                    qr
                }
                VStack(alignment: .leading, spacing: theme.space(12)) {
                    WWordList(words: model.words, label: model.wordsHeading)
                    qr
                }
            }
            HStack(spacing: theme.space(10)) {
                WButton(model.printTitle, glyph: "printer", kind: .secondary, action: model.onPrint)
                if let note = model.printedNote { WSuccessLine(text: note) }
            }
            if let warning = model.printToFileWarning { WWarningLine(text: warning) }
            Toggle(isOn: wroteDown) { Text(model.wroteDownLabel).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true) }
                .toggleStyle(.checkbox)
        }
    }

    @ViewBuilder
    private var qr: some View {
        if let image = model.qrImage {
            Image(nsImage: image).interpolation(.none).resizable()
                .frame(width: 140 * k, height: 140 * k)
                .padding(6)
                .background(Color.white)
                .accessibilityLabel(model.qrLabel)
        }
    }

    private var testStep: some View {
        step {
            heading(model.testHeading)
            WField(label: model.testLabel, text: testPassphrase, help: model.testHelp, isSecure: true, onSubmit: model.onTest)
            WButton(model.testButtonTitle, kind: .secondary, action: model.onTest).disabled(!model.canTest)
            if let passed = model.testPassedLine {
                WSuccessLine(text: passed)
            } else if let failed = model.testFailedLine {
                WErrorLine(text: failed)
            }
            Toggle(isOn: savedConfirmed) { Text(model.savedItLabel).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true) }
                .toggleStyle(.checkbox)
                .disabled(!model.canConfirmSaved)
            if !model.canConfirmSaved, let hint = model.savedItLockedHint {
                Text(hint).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

// MARK: Recovery

public enum WRecoveryMethod: Equatable, Sendable { case file, words }

/// The recovery screen: this Mac has no key for the memory database (or the key was rejected).
public struct WRecoveryModel {
    public var title = ""
    /// Why the screen is here: needs a key, or the key was rejected.
    public var reason = ""
    public var unchanged = ""
    public var methodLabel = ""
    public var fileMethodTitle = ""
    public var wordsMethodTitle = ""
    public var chooseFileTitle = ""
    public var chosenFileName: String?
    public var passphraseLabel = ""
    public var wordsLabel = ""
    public var wordsHelp = ""
    /// "Unlock with file", "Unlock with words", or the working words.
    public var unlockTitle = ""
    public var canUnlock = false
    public var working = false
    /// Directly under the form it belongs to.
    public var errorLine: String?
    public var restoredLine: String?
    public var finePrint: [String] = []
    public var quitTitle: String?
    public var quitEnabled = true
    public var onChooseFile: () -> Void = {}
    public var onUnlock: () -> Void = {}
    public var onQuit: () -> Void = {}

    public init() {}

    /// Quit, then the unlock button as the one primary.
    public var actions: [WScreenAction] {
        var list: [WScreenAction] = []
        if let quitTitle { list.append(WScreenAction(id: "quit", title: quitTitle, kind: .secondary, isEnabled: quitEnabled, run: onQuit)) }
        list.append(WScreenAction(id: "unlock", title: unlockTitle, kind: .primary, isEnabled: canUnlock, run: onUnlock))
        return list
    }
}

public struct WRecoveryView: View {
    @Environment(\.wTheme) private var theme
    let model: WRecoveryModel
    let method: Binding<WRecoveryMethod>
    let passphrase: Binding<String>
    let words: Binding<String>

    public init(model: WRecoveryModel, method: Binding<WRecoveryMethod>, passphrase: Binding<String>, words: Binding<String>) {
        self.model = model
        self.method = method
        self.passphrase = passphrase
        self.words = words
    }

    public var body: some View {
        WSetupPage(title: model.title, paragraph: model.reason, busy: model.working, actions: model.actions, finePrint: model.finePrint) {
            VStack(alignment: .leading, spacing: theme.space(16)) {
                Text(model.unchanged).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: theme.space(14)) {
                    Picker(model.methodLabel, selection: method) {
                        Text(model.fileMethodTitle).tag(WRecoveryMethod.file)
                        Text(model.wordsMethodTitle).tag(WRecoveryMethod.words)
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .accessibilityLabel(model.methodLabel)
                    .fixedSize()
                    switch method.wrappedValue {
                    case .file: fileForm
                    case .words: wordsForm
                    }
                    if let error = model.errorLine { WErrorLine(text: error) }
                    if let restored = model.restoredLine { WSuccessLine(text: restored) }
                }
                .padding(theme.space(18))
                .frame(maxWidth: .infinity, alignment: .leading)
                .wCard()
                .accessibilityElement(children: .contain)
            }
        }
    }

    private var fileForm: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            HStack(spacing: theme.space(10)) {
                WButton(model.chooseFileTitle, glyph: "doc", kind: .secondary, action: model.onChooseFile)
                if let name = model.chosenFileName {
                    Text(name).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1).truncationMode(.middle)
                }
            }
            WField(label: model.passphraseLabel, text: passphrase, isSecure: true, onSubmit: model.onUnlock)
        }
    }

    private var wordsForm: some View {
        WTextArea(label: model.wordsLabel, help: model.wordsHelp, text: words)
    }
}

/// A multi-line field for the 24 words: labeled, help before typing, monospaced, no spelling correction.
struct WTextArea: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @FocusState private var focused: Bool
    let label: String
    let help: String
    let text: Binding<String>

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(6)) {
            Text(label).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
            Text(help).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            TextEditor(text: text)
                .wFont(.body).monospaced()
                .foregroundStyle(theme.textPrimary)
                .scrollContentBackground(.hidden)
                .autocorrectionDisabled()
                .focused($focused)
                .padding(theme.space(6))
                .frame(minHeight: 96 * k, maxHeight: 140 * k)
                .background(shape.fill(theme.backgroundCard))
                .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
                .overlay { if focused { shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-1).allowsHitTesting(false) } }
                .frame(maxWidth: 420 * k)
                .accessibilityLabel(label)
                .accessibilityHint(help)
        }
    }
}

// MARK: Sync waiting

/// The waiting panel in the Devices pane: this device waits for another to approve it.
public struct WSyncWaitingModel {
    public var title = ""
    public var message = ""
    public var checkAgainTitle: String?
    /// "The other Mac isn't available?", shown while the fresh start warning is hidden.
    public var freshStartPrompt: String?
    public var freshStartWarning: String?
    public var startFreshTitle = ""
    public var cancelTitle = ""
    public var startingFresh = false
    public var actionError: String?
    public var onCheckAgain: () -> Void = {}
    public var onShowFreshStart: () -> Void = {}
    public var onStartFresh: () -> Void = {}
    public var onCancelFreshStart: () -> Void = {}

    public init() {}
}

public struct WSyncWaitingView: View {
    @Environment(\.wTheme) private var theme
    let model: WSyncWaitingModel

    public init(model: WSyncWaitingModel) { self.model = model }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            WProgressLine(label: model.title)
            Text(model.title).wFont(.subtitle).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true).accessibilityAddTraits(.isHeader)
            Text(model.message).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            if let title = model.checkAgainTitle {
                WButton(title, kind: .secondary, action: model.onCheckAgain)
            }
            if let warning = model.freshStartWarning {
                Text(warning).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                HStack(spacing: theme.space(10)) {
                    WButton(model.startFreshTitle, kind: .destructive, action: model.onStartFresh).disabled(model.startingFresh)
                    WButton(model.cancelTitle, kind: .secondary, action: model.onCancelFreshStart).disabled(model.startingFresh)
                }
            } else if let prompt = model.freshStartPrompt {
                WButton(prompt, kind: .quiet, action: model.onShowFreshStart)
            }
            if let error = model.actionError { WErrorLine(text: error) }
        }
        .padding(theme.space(16))
        .frame(maxWidth: 560, alignment: .leading)
        .accessibilityElement(children: .contain)
    }
}
