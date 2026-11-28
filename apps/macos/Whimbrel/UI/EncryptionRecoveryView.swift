import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The key-gone screen (X4 design 3.4): the memory database is encrypted and this Mac has no
/// key for it, or its key does not open it. The only ways forward are the recovery file
/// with its passphrase, or the 24 words. Whimbrel never offers to start fresh, create a new
/// database or delete this one, and touches nothing until a recovery key opens it.
@MainActor
final class EncryptionRecoveryModel: ObservableObject {
    enum Reason: Equatable {
        /// `needsKey`: an encrypted file and no key.
        case keyMissing
        /// `keyRejected`: the stored key does not open the file.
        case keyRejected
    }

    enum Method: String, CaseIterable, Identifiable {
        case file, words
        var id: String { rawValue }
    }

    /// Every button on the screen goes through `perform(_:)`. Deliberately nothing that
    /// starts over, creates a database or deletes.
    enum Action: CaseIterable {
        case chooseFile, restoreFromFile, restoreFromWords
    }

    enum ChosenFile: Equatable {
        case chosen(name: String, contents: String)
        case cancelled
        case unreadable
    }

    /// What became of a restored key once the shell stored it and opened the database
    /// (the launch wiring, Task 11).
    enum UseKeyResult: Equatable {
        case opened
        /// The key is a valid recovery key but does not open this database.
        case keyRejected
        case failed
    }

    struct Actions {
        var chooseFile: @MainActor () -> ChosenFile
        /// (file text, passphrase) -> key. Slow (Argon2id).
        var restoreFromFile: (String, String) async -> Result<Data, CoreRecoveryError>
        /// The words exactly as typed or pasted; core cleans spacing and capitals.
        var restoreFromWords: (String) async -> Result<Data, CoreRecoveryError>
        /// Store the key and open the database with it.
        var useKey: (Data) async -> UseKeyResult
    }

    let reason: Reason
    private let actions: Actions

    @Published var method: Method = .file { didSet { error = nil } }
    @Published var passphrase = "" { didSet { if passphrase != oldValue { error = nil } } }
    @Published var words = "" { didSet { if words != oldValue { error = nil } } }
    @Published private(set) var chosenFileName: String?
    @Published private(set) var working = false
    @Published private(set) var error: String?
    @Published private(set) var restored = false
    private var chosenContents: String?

    init(reason: Reason, actions: Actions) {
        self.reason = reason
        self.actions = actions
    }

    var explanation: String {
        reason == .keyMissing ? EncryptionCopy.recoveryKeyMissing : EncryptionCopy.recoveryKeyRejected
    }

    var canRestore: Bool {
        guard !working, !restored else { return false }
        switch method {
        case .file: return chosenContents != nil && !passphrase.isEmpty
        case .words: return !words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    func perform(_ action: Action) async {
        switch action {
        case .chooseFile: chooseFile()
        case .restoreFromFile:
            method = .file
            await restore()
        case .restoreFromWords:
            method = .words
            await restore()
        }
    }

    func chooseFile() {
        switch actions.chooseFile() {
        case let .chosen(name, contents):
            chosenFileName = name
            chosenContents = contents
            error = nil
        case .cancelled:
            break
        case .unreadable:
            chosenFileName = nil
            chosenContents = nil
            error = EncryptionCopy.fileUnreadable
        }
    }

    func restore() async {
        guard canRestore else { return }
        working = true
        defer { working = false }
        let result: Result<Data, CoreRecoveryError>
        switch method {
        case .file: result = await actions.restoreFromFile(chosenContents ?? "", passphrase)
        case .words: result = await actions.restoreFromWords(words)
        }
        switch result {
        case let .success(key):
            switch await actions.useKey(key) {
            case .opened:
                restored = true
                clearSecrets()
                error = nil
            case .keyRejected:
                error = EncryptionCopy.keyForAnotherDatabase
            case .failed:
                error = EncryptionCopy.restoreFailed
            }
        case let .failure(e):
            error = Self.message(for: e)
        }
    }

    /// Drops the typed passphrase, the words and the file text. Called once the key opened the
    /// database and when the screen disappears. Nothing here logs or copies them.
    func clearSecrets() {
        passphrase = ""
        words = ""
        chosenContents = nil
        chosenFileName = nil
    }

    static func message(for error: CoreRecoveryError) -> String {
        switch error {
        case .wrongPassphraseOrDamaged: return EncryptionCopy.wrongPassphrase
        case .malformed: return EncryptionCopy.notARecoveryFile
        case .badWords: return EncryptionCopy.badWords
        case .passphraseTooShort, .invalidArgument, .unknown: return EncryptionCopy.restoreFailed
        }
    }
}

extension EncryptionRecoveryModel.Actions {
    /// Core restores and an NSOpenPanel; `useKey` is the launch flow's (Task 11): store the
    /// key in the Keychain and open the database with it.
    static func live(useKey: @escaping (Data) async -> EncryptionRecoveryModel.UseKeyResult) -> Self {
        Self(
            chooseFile: {
                let panel = NSOpenPanel()
                panel.canChooseFiles = true
                panel.canChooseDirectories = false
                panel.allowsMultipleSelection = false
                panel.allowedContentTypes = [.plainText]
                guard panel.runModal() == .OK, let url = panel.url else { return .cancelled }
                // A recovery file is a few hundred bytes; refuse anything large rather than read it.
                if let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size > 64 * 1024 { return .unreadable }
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { return .unreadable }
                return .chosen(name: url.lastPathComponent, contents: text)
            },
            restoreFromFile: { file, passphrase in
                await Task.detached(priority: .userInitiated) {
                    CoreEncryptionSetup.restore(recoveryFile: file, passphrase: passphrase)
                }.value
            },
            restoreFromWords: { words in
                await Task.detached(priority: .userInitiated) { CoreEncryptionSetup.restore(words: words) }.value
            },
            useKey: useKey
        )
    }
}

/// The Keychain could not be read at launch (locked, denied, failing). Not a lost key: the
/// only action is Retry, and nothing is opened, changed or deleted.
@MainActor
final class EncryptionKeyUnavailableModel: ObservableObject {
    enum Action: CaseIterable {
        case retry
    }

    let detail: String
    /// Looks the key up again and opens if it can; true when that worked.
    private let retryAction: () async -> Bool
    @Published private(set) var retrying = false
    @Published private(set) var stillUnavailable = false

    init(detail: String, retry: @escaping () async -> Bool) {
        self.detail = detail
        self.retryAction = retry
    }

    func perform(_ action: Action) async {
        switch action {
        case .retry: await retry()
        }
    }

    func retry() async {
        guard !retrying else { return }
        retrying = true
        let ok = await retryAction()
        retrying = false
        stillUnavailable = !ok
    }
}
