import AppKit
import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI
import UniformTypeIdentifiers

/// The recovery-key step that comes before the memory database is encrypted (X4 design 3.4):
/// choose a passphrase, save the sealed file, see the 24 words and their QR, pass the
/// type-it-back test, keep the key somewhere (file saved, words printed, or words written
/// down), confirm, then continue. There is no skip. The model holds the flow; every outside
/// effect (core, panels, printing) is an injected closure.
///
/// Once the key exists the passphrase is locked: changing it would make a new file and words
/// and silently void what the user already kept, so that takes an explicit Start over.
///
/// The words and QR are the raw key: nothing here writes them (or the file text) to the
/// pasteboard, logs them, or makes them selectable. They are shown, printed or written down,
/// and `clearSecrets()` drops them when the screen goes away or after Continue.
@MainActor
final class EncryptionSetupModel: ObservableObject {
    enum SaveResult: Equatable {
        /// `sameDiskAsDatabase`: the file sits on the volume that holds the database, so it
        /// is lost with this Mac (a warning, not a refusal).
        case saved(path: String, sameDiskAsDatabase: Bool)
        case cancelled
        case failed
    }

    /// What the print panel did. Only a real print job counts as keeping the key: a PDF or a
    /// preview would leave the raw key as an ordinary file on this Mac.
    enum PrintResult: Equatable {
        case printed
        case savedAsFile
        case cancelled
    }

    /// Every button on the screen goes through `perform(_:)`. No skip, no "later", no copy.
    enum Action: CaseIterable {
        case createRecoveryKey, saveFile, printWords, test, startOver, confirmStartOver, continueToEncrypt
    }

    struct Actions {
        var minimumLength: Int
        /// The count the minimum applies to (core: characters after NFC).
        var passphraseLength: (String) -> Int
        /// Core's `begin_setup`; slow (Argon2id), so the live one runs off the main thread.
        var begin: (String) async -> Result<CoreSetupMaterial, CoreRecoveryError>
        /// The type-it-back test against the in-memory file text: (file, passphrase).
        var verify: (String, String) async -> Bool
        /// Saves the sealed file text (never the words).
        var saveFile: @MainActor (String) async -> SaveResult
        /// Prints the words and QR (`EncryptionPrint.printWords`).
        var printWords: @MainActor ([String]) -> PrintResult
        /// Setup wraps a key that already exists (the pending key): Start over then keeps the
        /// same words and only makes a new file under a new passphrase.
        var reusesKey: Bool = false
    }

    @Published var passphrase = "" {
        didSet {
            guard passphrase != oldValue, !reverting else { return }
            if material != nil { revert { passphrase = oldValue } } else { generation += 1 }
        }
    }
    @Published var confirmation = "" {
        didSet {
            guard confirmation != oldValue, !reverting else { return }
            if material != nil { revert { confirmation = oldValue } } else { generation += 1 }
        }
    }
    /// Set while a refused edit is put back (a `@Published` didSet fires again on assignment).
    private var reverting = false
    private func revert(_ body: () -> Void) {
        reverting = true
        body()
        reverting = false
    }
    @Published private(set) var material: CoreSetupMaterial?
    @Published private(set) var creating = false
    @Published private(set) var createError: String?
    @Published private(set) var savedPath: String?
    @Published private(set) var savedOnSameDisk = false
    @Published private(set) var saveFailed = false
    /// A real print job ran for these words.
    @Published private(set) var printed = false
    /// The last print went to a file (PDF) or preview, which does not count and is warned about.
    @Published private(set) var printWentToFile = false
    /// "I wrote down all 24 words, in order": the third way to keep the key.
    @Published var wroteDownTicked = false { didSet { enforceSavedConfirmed() } }
    @Published var testPassphrase = "" {
        didSet {
            guard testPassphrase != oldValue else { return }
            testPassed = false
            testFailed = false
            enforceSavedConfirmed()
        }
    }
    @Published private(set) var testing = false
    @Published private(set) var testPassed = false
    @Published private(set) var testFailed = false
    /// "I have kept my recovery file or my 24 words ...". Refuses to turn on until
    /// `canConfirmSaved`.
    @Published var savedConfirmed = false { didSet { if savedConfirmed && !canConfirmSaved { savedConfirmed = false } } }
    /// The Start over confirmation is showing.
    @Published var startOverPending = false

    let continueTitle: String
    let actions: Actions
    private let onContinue: (Data) -> Void
    /// Bumped whenever the passphrase changes or the key is discarded, so a slow `begin` for
    /// an old passphrase is dropped.
    private var generation = 0

    init(actions: Actions, continueTitle: String = EncryptionCopy.continueDefault, onContinue: @escaping (Data) -> Void) {
        self.actions = actions
        self.continueTitle = continueTitle
        self.onContinue = onContinue
    }

    var minimumLength: Int { actions.minimumLength }
    var passphraseCount: Int { actions.passphraseLength(passphrase) }
    var meetsMinimum: Bool { passphraseCount >= minimumLength }
    var confirmationMatches: Bool { !confirmation.isEmpty && confirmation == passphrase }
    /// The passphrase fields are read-only once the key exists (Start over to change it).
    var passphraseLocked: Bool { material != nil }

    /// What is wrong with the passphrase pair, or nil (also nil while nothing is typed).
    var passphraseProblem: String? {
        if passphrase.isEmpty || passphraseLocked { return nil }
        if !meetsMinimum { return EncryptionCopy.passphraseTooShort(minimumLength) }
        if !confirmation.isEmpty && confirmation != passphrase { return EncryptionCopy.passphrasesDiffer }
        return nil
    }

    var canCreate: Bool { meetsMinimum && confirmationMatches && !creating && material == nil }
    var canTest: Bool { material != nil && !testPassphrase.isEmpty && !testing }
    /// The key has been kept somewhere: file saved, words printed, or written down.
    var keptSomewhere: Bool { savedPath != nil || printed || wroteDownTicked }
    var canConfirmSaved: Bool { testPassed && keptSomewhere }
    var canContinue: Bool { material != nil && canConfirmSaved && savedConfirmed }

    /// The QR of the words (the raw key), for the screen and the printout.
    var qrImage: NSImage? { material.flatMap { EncryptionQR.image(for: $0.words.joined(separator: " ")) } }

    func perform(_ action: Action) async {
        switch action {
        case .createRecoveryKey: await createRecoveryKey()
        case .saveFile: await saveFile()
        case .printWords: printWords()
        case .test: await runTest()
        case .startOver: requestStartOver()
        case .confirmStartOver: confirmStartOver()
        case .continueToEncrypt: continueToEncrypt()
        }
    }

    func createRecoveryKey() async {
        guard canCreate else { return }
        creating = true
        createError = nil
        let started = generation
        let result = await actions.begin(passphrase)
        creating = false
        guard started == generation else { return }
        switch result {
        case let .success(m): material = m
        case .failure(.passphraseTooShort): createError = EncryptionCopy.passphraseTooShort(minimumLength)
        case .failure: createError = EncryptionCopy.createFailed
        }
    }

    func saveFile() async {
        guard let material else { return }
        switch await actions.saveFile(material.recoveryFile) {
        case let .saved(path, sameDisk):
            savedPath = path
            savedOnSameDisk = sameDisk
            saveFailed = false
        case .cancelled:
            break
        case .failed:
            saveFailed = true
        }
    }

    func printWords() {
        guard let material else { return }
        switch actions.printWords(material.words) {
        case .printed:
            printed = true
            printWentToFile = false
        case .savedAsFile:
            printWentToFile = true
        case .cancelled:
            break
        }
    }

    /// The Start over confirmation, true to what it does: a fresh key makes new words that the
    /// old file and printout do not match; a reused key keeps the words and only re-seals the file.
    var startOverTitle: String { actions.reusesKey ? EncryptionCopy.startOverReuseTitle : EncryptionCopy.startOverTitle }
    var startOverBody: String { actions.reusesKey ? EncryptionCopy.startOverReuseBody : EncryptionCopy.startOverBody }

    func runTest() async {
        guard canTest, let material else { return }
        testing = true
        let typed = testPassphrase
        let ok = await actions.verify(material.recoveryFile, typed)
        testing = false
        guard typed == testPassphrase, self.material == material else { return }
        testPassed = ok
        testFailed = !ok
    }

    /// Asks before throwing the key away (the view shows the confirmation).
    func requestStartOver() {
        guard material != nil else { return }
        startOverPending = true
    }

    /// Discards the key, words, file and checks; the passphrase is typed again.
    func confirmStartOver() {
        startOverPending = false
        clearSecrets()
    }

    func continueToEncrypt() {
        guard canContinue, let material else { return }
        let key = material.key
        clearSecrets()
        onContinue(key)
    }

    /// Drops the passphrases, the words, the QR and the file text from the model. Called after
    /// Continue, on Start over and when the screen disappears (a Swift copy may linger until
    /// reused; it is never written anywhere).
    func clearSecrets() {
        discardMaterial()
        passphrase = ""
        confirmation = ""
    }

    private func enforceSavedConfirmed() {
        if savedConfirmed && !canConfirmSaved { savedConfirmed = false }
    }

    private func discardMaterial() {
        generation += 1
        material = nil
        createError = nil
        savedPath = nil
        savedOnSameDisk = false
        saveFailed = false
        printed = false
        printWentToFile = false
        wroteDownTicked = false
        testPassphrase = ""
        testPassed = false
        testFailed = false
        savedConfirmed = false
    }
}

extension EncryptionSetupModel.Actions {
    /// The real collaborators: core for the key, an NSSavePanel for the file and the print
    /// panel. No pasteboard: the words are the raw key.
    ///
    /// `existingKey` is the launch flow's pending key (Task 11): store the setup's key as
    /// `db-key-pending` when the user continues, and pass it here on any retry, so a retry
    /// never produces new words. `databaseFolder` is where the database lives, for the
    /// same-disk warning.
    static func live(existingKey: Data? = nil, databaseFolder: URL? = SpikePaths.root) -> Self {
        Self(
            minimumLength: CoreEncryptionSetup.minimumPassphraseLength,
            passphraseLength: CoreEncryptionSetup.passphraseLength,
            begin: { passphrase in
                await Task.detached(priority: .userInitiated) {
                    CoreEncryptionSetup.begin(passphrase: passphrase, existingKey: existingKey)
                }.value
            },
            verify: { file, passphrase in
                await Task.detached(priority: .userInitiated) {
                    CoreEncryptionSetup.verify(recoveryFile: file, passphrase: passphrase)
                }.value
            },
            saveFile: { contents in EncryptionRecoveryFilePanel.save(contents, databaseFolder: databaseFolder) },
            printWords: { words in EncryptionPrint.printWords(words) },
            reusesKey: existingKey != nil
        )
    }
}

/// Saving `Whimbrel recovery key.txt` through an NSSavePanel.
enum EncryptionRecoveryFilePanel {
    @MainActor
    static func save(_ contents: String, databaseFolder: URL?) -> EncryptionSetupModel.SaveResult {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = EncryptionCopy.recoveryFileName
        panel.allowedContentTypes = [.plainText]
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return .cancelled }
        do {
            try Data(contents.utf8).write(to: url, options: .atomic)
            return .saved(path: url.path, sameDiskAsDatabase: sameVolume(url, databaseFolder))
        } catch {
            return .failed
        }
    }

    /// Whether `file` is on the volume that holds `folder` (false when either is unknown).
    static func sameVolume(_ file: URL, _ folder: URL?) -> Bool {
        guard let folder else { return false }
        func volume(_ url: URL) -> NSObject? {
            (try? url.resourceValues(forKeys: [.volumeIdentifierKey]))?.volumeIdentifier as? NSObject
        }
        guard let a = volume(file.deletingLastPathComponent()), let b = volume(folder) else { return false }
        return a.isEqual(b)
    }
}

/// A QR code of the words, drawn with hard pixel edges so it prints and scans cleanly.
enum EncryptionQR {
    static func image(for text: String, scale: CGFloat = 8) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: scale, y: scale)) else { return nil }
        let rep = NSCIImageRep(ciImage: output)
        let image = NSImage(size: rep.size)
        image.addRepresentation(rep)
        return image
    }
}

/// The printed page: the words, the QR and the warning.
enum EncryptionPrint {
    /// `.printed` only for a real print job (`jobDisposition == .spool` and `run()` true);
    /// "Save as PDF" and preview are `.savedAsFile`; a cancel is `.cancelled`.
    @MainActor
    static func printWords(_ words: [String]) -> EncryptionSetupModel.PrintResult {
        let page = EncryptionPrintPage(words: words, qr: EncryptionQR.image(for: words.joined(separator: " ")), copy: EncryptionPrintCopy.forSkin())
        let host = NSHostingView(rootView: page.environment(\.colorScheme, .light))
        host.frame = CGRect(x: 0, y: 0, width: 540, height: 720)
        let info = NSPrintInfo.shared.copy() as? NSPrintInfo ?? NSPrintInfo()
        info.horizontalPagination = .fit
        info.verticalPagination = .fit
        let operation = NSPrintOperation(view: host, printInfo: info)
        let ran = operation.run()
        return result(ran: ran, disposition: operation.printInfo.jobDisposition)
    }

    static func result(ran: Bool, disposition: NSPrintInfo.JobDisposition) -> EncryptionSetupModel.PrintResult {
        guard ran else { return .cancelled }
        return disposition == .spool ? .printed : .savedAsFile
    }
}

/// The printed page's words, in the app's language by default. A test renders it with each
/// language's translations (`localized(_:)`), which `EncryptionCopy`'s strings cannot do.
struct EncryptionPrintCopy: Equatable {
    var title = EncryptionCopy.printTitle
    var notice = EncryptionCopy.wordsAreTheKey
    var footer = EncryptionCopy.printFooter
    var qrLabel = EncryptionCopy.qrLabel

    /// The page with the redesigned wording when the switch is on ("your library"), this page unchanged when it is off.
    static func forSkin(redesign: Bool = RedesignFlag.isOn()) -> EncryptionPrintCopy {
        guard redesign else { return EncryptionPrintCopy() }
        return EncryptionPrintCopy(notice: LibraryCopyRedesign.map(EncryptionCopy.wordsAreTheKey), footer: LibraryCopyRedesign.map(EncryptionCopy.printFooter))
    }

    /// The same strings from one language's table in the app bundle (nil without that table).
    static func localized(_ language: String, bundle: Bundle = .main) -> EncryptionPrintCopy? {
        guard let path = bundle.path(forResource: language, ofType: "lproj"), let table = Bundle(path: path) else { return nil }
        let english = EncryptionPrintCopy()
        func t(_ key: String) -> String { table.localizedString(forKey: key, value: key, table: nil) }
        return .init(title: t(english.title), notice: t(english.notice), footer: t(english.footer), qrLabel: t(english.qrLabel))
    }
}

/// The printed page, 540 by 720 points. Every line of the safety text must fit: the words
/// and the QR are the raw key, so a clipped notice or footer would hide the warning. Top to
/// bottom: title, notice, the 24 words, the QR, then the footer under it.
/// `EncryptionPrintPageTests` measures the content in every shipped language.
struct EncryptionPrintPage: View {
    static let size = CGSize(width: 540, height: 720)
    static let margin: CGFloat = 36
    /// Paper white, deliberately not a theme token: the page is printed, and a QR code needs a
    /// white quiet zone to scan, whatever the app's theme.
    static let paper = Color(.sRGB, red: 1, green: 1, blue: 1, opacity: 1)

    let words: [String]
    let qr: NSImage?
    var copy = EncryptionPrintCopy()

    var body: some View {
        EncryptionPrintPageContent(words: words, qr: qr, copy: copy)
            .padding(Self.margin)
            .frame(width: Self.size.width, height: Self.size.height, alignment: .topLeading)
            .background(Self.paper)
    }
}

/// The page without its margins, so its natural height can be measured.
struct EncryptionPrintPageContent: View {
    let words: [String]
    let qr: NSImage?
    var copy = EncryptionPrintCopy()

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(copy.title).font(.title2.bold()).fixedSize(horizontal: false, vertical: true)
            Text(copy.notice).font(.body).fixedSize(horizontal: false, vertical: true)
            EncryptionWordGrid(words: words)
            if let qr {
                Image(nsImage: qr).interpolation(.none).resizable().frame(width: 200, height: 200)
                    .accessibilityLabel(copy.qrLabel)
            }
            Text(copy.footer).font(.footnote).fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(Color.black)
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}

/// The 24 words, numbered, in three columns of eight.
struct EncryptionWordGrid: View {
    let words: [String]

    var body: some View {
        let columns = Array(repeating: GridItem(.flexible(), alignment: .leading), count: 3)
        LazyVGrid(columns: columns, alignment: .leading, spacing: 6) {
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                HStack(spacing: 6) {
                    Text("\(index + 1).").monospacedDigit().foregroundStyle(.secondary).frame(width: 26, alignment: .trailing)
                    Text(word).font(.system(.body, design: .monospaced))
                }
            }
        }
    }
}

struct EncryptionSetupView: View {
    @ObservedObject var model: EncryptionSetupModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.lg) {
                Text(EncryptionCopy.setupTitle).font(.whimbrelDisplay()).foregroundStyle(Color.whimbrelTextPrimary)
                Text(EncryptionCopy.setupIntro).font(.whimbrelBody()).foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                passphraseSection
                if model.material != nil {
                    Divider().overlay(Color.whimbrelBorder)
                    fileSection
                    Divider().overlay(Color.whimbrelBorder)
                    wordsSection
                    Divider().overlay(Color.whimbrelBorder)
                    testSection
                }
                Text(EncryptionCopy.lostBoth).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Spacer()
                    Button(model.continueTitle) { Task { await model.perform(.continueToEncrypt) } }
                        .buttonStyle(.whimbrelPrimary)
                        .disabled(!model.canContinue)
                }
            }
            .padding(Spacing.xl)
        }
        .background(Color.whimbrelBackgroundPage)
        .onDisappear { model.clearSecrets() }
        .alert(model.startOverTitle, isPresented: $model.startOverPending) {
            Button(EncryptionCopy.startOverConfirm, role: .destructive) { Task { await model.perform(.confirmStartOver) } }
            Button(EncryptionCopy.keepThisKey, role: .cancel) {}
        } message: {
            Text(model.startOverBody)
        }
    }

    private func header(_ text: String) -> some View {
        Text(text).font(.whimbrelBody(.headline)).foregroundStyle(Color.whimbrelTextPrimary)
    }

    private var passphraseSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            header(EncryptionCopy.stepPassphrase)
            Text(EncryptionCopy.passphraseHelp).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            SecureField(EncryptionCopy.passphraseField, text: $model.passphrase).textFieldStyle(.roundedBorder)
                .disabled(model.passphraseLocked)
            SecureField(EncryptionCopy.confirmField, text: $model.confirmation).textFieldStyle(.roundedBorder)
                .disabled(model.passphraseLocked)
            HStack {
                Text(EncryptionCopy.passphraseCount(model.passphraseCount, model.minimumLength))
                    .font(.whimbrelBody(.caption)).monospacedDigit()
                    .foregroundStyle(model.meetsMinimum ? Color.whimbrelSuccess : Color.whimbrelTextSecondary)
                Spacer()
                if model.material == nil {
                    Button(model.creating ? EncryptionCopy.creatingKey : EncryptionCopy.createKey) {
                        Task { await model.perform(.createRecoveryKey) }
                    }
                    .buttonStyle(.whimbrelPrimary)
                    .disabled(!model.canCreate)
                } else {
                    Button(EncryptionCopy.startOver) { Task { await model.perform(.startOver) } }
                        .buttonStyle(.whimbrelSecondary)
                }
            }
            if let problem = model.passphraseProblem ?? model.createError {
                Text(problem).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
            if model.passphraseLocked {
                Text(EncryptionCopy.passphraseLockedHint).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    private var fileSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            header(EncryptionCopy.stepFile)
            Text(EncryptionCopy.fileHelp).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Button(EncryptionCopy.saveFile) { Task { await model.perform(.saveFile) } }
                .buttonStyle(.whimbrelPrimary)
                .controlSize(.large)
            if let path = model.savedPath {
                Text(EncryptionCopy.savedTo(path)).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelSuccess)
                    .lineLimit(2).truncationMode(.middle)
                if model.savedOnSameDisk {
                    Text(EncryptionCopy.sameDiskWarning).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else if model.saveFailed {
                Text(EncryptionCopy.saveFailed).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
        }
    }

    private var wordsSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            header(EncryptionCopy.stepWords)
            HStack(alignment: .top, spacing: Spacing.sm) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color.whimbrelWarning)
                Text(EncryptionCopy.wordsAreTheKey).font(.whimbrelBody(.body).weight(.semibold))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(Spacing.md)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color.whimbrelWarning.opacity(0.12)))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.whimbrelWarning, lineWidth: 1))
            HStack(alignment: .top, spacing: Spacing.lg) {
                EncryptionWordGrid(words: model.material?.words ?? [])
                    .font(.whimbrelBody())
                    .foregroundStyle(Color.whimbrelTextPrimary)
                if let qr = model.qrImage {
                    Image(nsImage: qr).interpolation(.none).resizable().frame(width: 150, height: 150)
                        .padding(6).background(EncryptionPrintPage.paper)
                        .accessibilityLabel(EncryptionCopy.qrLabel)
                }
            }
            HStack {
                Button(EncryptionCopy.printWords) { Task { await model.perform(.printWords) } }.buttonStyle(.whimbrelSecondary)
                if model.printed {
                    Label(EncryptionCopy.printedNote, systemImage: "checkmark.circle.fill")
                        .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelSuccess)
                }
            }
            if model.printWentToFile {
                Text(EncryptionCopy.printAsFileRefused).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Toggle(EncryptionCopy.wroteDown, isOn: $model.wroteDownTicked).toggleStyle(.checkbox)
        }
    }

    private var testSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            header(EncryptionCopy.stepTest)
            Text(EncryptionCopy.testHelp).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                SecureField(EncryptionCopy.passphraseField, text: $model.testPassphrase).textFieldStyle(.roundedBorder)
                    .onSubmit { Task { await model.perform(.test) } }
                Button(model.testing ? EncryptionCopy.testing : EncryptionCopy.testButton) { Task { await model.perform(.test) } }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(!model.canTest)
            }
            if model.testPassed {
                Label(EncryptionCopy.testPassed, systemImage: "checkmark.circle.fill")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelSuccess)
            } else if model.testFailed {
                Text(EncryptionCopy.testFailed).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelError)
            }
            Toggle(EncryptionCopy.iSavedIt, isOn: $model.savedConfirmed)
                .toggleStyle(.checkbox)
                .disabled(!model.canConfirmSaved)
            if !model.canConfirmSaved {
                Text(EncryptionCopy.iSavedItLocked).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }
}
