import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

/// Renders the encryption screens to PNGs for review (X4 Task 10). Not a snapshot test: it
/// compares nothing and writes outside the repo. Skipped unless
/// `WHIMBREL_ENCRYPTION_SCREENSHOTS=<folder>` (pass `TEST_RUNNER_WHIMBREL_ENCRYPTION_SCREENSHOTS`
/// to xcodebuild).
@MainActor
final class EncryptionScreenshotTests: XCTestCase {
    /// Fixed sample words (not a real key: the checksum is not valid).
    static let words = ["abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract",
                        "absurd", "abuse", "access", "accident", "account", "accuse", "achieve", "acid",
                        "acoustic", "acquire", "across", "act", "action", "actor", "actress", "actual"]

    private func folder() throws -> URL {
        guard let path = ProcessInfo.processInfo.environment["WHIMBREL_ENCRYPTION_SCREENSHOTS"], !path.isEmpty else {
            throw XCTSkip("Set WHIMBREL_ENCRYPTION_SCREENSHOTS=<folder> to render the encryption screens")
        }
        let url = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func write(_ view: some View, _ name: String, size: CGSize, scheme: ColorScheme = .light, to dir: URL) throws {
        let pinned = view.redesignThemed().environment(\.locale, Locale(identifier: "en_US"))
        let rep = try XCTUnwrap(OffscreenRender.bitmap(of: pinned, size: size, colorScheme: scheme, scale: 2), name)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]), name)
        try png.write(to: dir.appendingPathComponent("\(name).png"))
    }

    private func setupActions() -> EncryptionSetupModel.Actions {
        .init(minimumLength: 12, passphraseLength: { $0.count },
              begin: { _ in .success(CoreSetupMaterial(key: Data(count: 32), recoveryFile: "FILE", words: Self.words)) },
              verify: { _, p in p == "correct horse battery" },
              saveFile: { _ in .saved(path: "/Volumes/USB/Whimbrel recovery key.txt", sameDiskAsDatabase: false) },
              printWords: { _ in .printed })
    }

    func testRenderEncryptionScreens() async throws {
        let dir = try folder()

        // Setup: passphrase typed, too short.
        let short = EncryptionSetupModel(actions: setupActions(), onContinue: { _ in })
        short.passphrase = "horse"
        try write(KeySetupRedesignView(model: short), "setup-1-passphrase", size: CGSize(width: 680, height: 560), to: dir)

        // Setup: key made, file saved, test passed, "I saved it" ticked.
        let done = EncryptionSetupModel(actions: setupActions(), continueTitle: EncryptionCopy.encryptContinue, onContinue: { _ in })
        done.passphrase = "correct horse battery"
        done.confirmation = "correct horse battery"
        await done.createRecoveryKey()
        await done.saveFile()
        done.testPassphrase = "correct horse battery"
        await done.runTest()
        done.savedConfirmed = true
        try write(KeySetupRedesignView(model: done), "setup-2-ready", size: CGSize(width: 680, height: 1320), to: dir)
        try write(KeySetupRedesignView(model: done), "setup-2-ready-dark", size: CGSize(width: 680, height: 1320), scheme: .dark, to: dir)

        // Setup: the test failed, so "I saved it" stays locked.
        let failed = EncryptionSetupModel(actions: setupActions(), continueTitle: EncryptionCopy.encryptContinue, onContinue: { _ in })
        failed.passphrase = "correct horse battery"
        failed.confirmation = "correct horse battery"
        await failed.createRecoveryKey()
        failed.testPassphrase = "wrong one"
        await failed.runTest()
        try write(KeySetupRedesignView(model: failed), "setup-3-test-failed", size: CGSize(width: 680, height: 1320), to: dir)

        try write(EncryptionPrintPage(words: Self.words, qr: EncryptionQR.image(for: Self.words.joined(separator: " "))),
                  "setup-4-print-page", size: CGSize(width: 540, height: 720), to: dir)

        // Recovery: key missing, file method with a wrong passphrase.
        let recovery = EncryptionRecoveryModel(reason: .keyMissing, actions: .init(
            chooseFile: { .chosen(name: "Whimbrel recovery key.txt", contents: "FILE") },
            restoreFromFile: { _, _ in .failure(.wrongPassphraseOrDamaged) },
            restoreFromWords: { _ in .failure(.badWords) },
            useKey: { _ in .opened }
        ))
        await recovery.perform(.chooseFile)
        recovery.passphrase = "not it"
        await recovery.restore()
        try write(RecoveryRedesignView(model: recovery), "recovery-1-file", size: CGSize(width: 680, height: 640), to: dir)

        // Recovery: key rejected, words method.
        let words = EncryptionRecoveryModel(reason: .keyRejected, actions: .init(
            chooseFile: { .cancelled },
            restoreFromFile: { _, _ in .failure(.malformed) },
            restoreFromWords: { _ in .failure(.badWords) },
            useKey: { _ in .opened }
        ))
        words.method = .words
        words.words = Self.words.prefix(20).map { $0.capitalized }.joined(separator: "  ")
        try write(RecoveryRedesignView(model: words), "recovery-2-words", size: CGSize(width: 680, height: 640), to: dir)

        // Key unavailable, after one failed retry.
        let unavailable = EncryptionKeyUnavailableModel(detail: "Keychain read failed (status -25308)") { false }
        await unavailable.retry()
        try write(KeyUnavailableRedesignView(model: unavailable), "key-unavailable", size: CGSize(width: 680, height: 440), to: dir)

        // Settings pane, each state.
        func actions(precheck: String? = nil, backup: Bool = false) -> EncryptionSettingsModel.Actions {
            .init(exportEverything: {}, precheck: { precheck }, plainBackupExists: { backup }, pendingKey: { nil },
                  setup: { _ in self.setupActions() }, restart: {}, encrypt: { _, _ in .encrypted })
        }
        func pane(_ m: EncryptionSettingsModel) -> some View {
            EncryptionSettingsPane(model: m).padding(Spacing.lg)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(Color.whimbrelBackgroundCard)
        }
        try write(pane(EncryptionSettingsModel(outcome: .needsMigration, actions: actions())), "settings-1-not-encrypted",
                  size: CGSize(width: 640, height: 320), to: dir)
        let offering = EncryptionSettingsModel(outcome: .needsMigration, actions: actions())
        offering.startEncrypt()
        try write(pane(offering), "settings-2-export-first", size: CGSize(width: 640, height: 420), to: dir)
        try write(pane(EncryptionSettingsModel(outcome: .opened, actions: actions(backup: true))), "settings-3-encrypted",
                  size: CGSize(width: 640, height: 340), to: dir)
        try write(pane(EncryptionSettingsModel(outcome: .keyRejected("x"), actions: actions())), "settings-4-needs-attention",
                  size: CGSize(width: 640, height: 300), to: dir)
        let skipped = EncryptionSettingsModel(outcome: .needsMigration, actions: actions(precheck: EncryptionCopy.skipRecording))
        skipped.startEncrypt()
        await skipped.continueToSetup()
        try write(pane(skipped), "settings-5-precheck-skipped", size: CGSize(width: 640, height: 340), to: dir)

        // Encryption handed to the next launch: Restart is the only action.
        let pending = EncryptionSettingsModel(outcome: .needsMigration, actions: .init(
            exportEverything: {}, precheck: { nil }, plainBackupExists: { false }, pendingKey: { nil },
            setup: { _ in self.setupActions() }, restart: {}, encrypt: { _, _ in .finishesAtNextLaunch }))
        pending.startEncrypt()
        await pending.continueToSetup()
        await pending.encrypt(with: Data(count: 32))
        try write(pane(pending), "settings-6-next-launch", size: CGSize(width: 640, height: 340), to: dir)

        // Setup: the print went to a PDF, which does not count.
        let pdf = EncryptionSetupModel(actions: .init(
            minimumLength: 12, passphraseLength: { $0.count },
            begin: { _ in .success(CoreSetupMaterial(key: Data(count: 32), recoveryFile: "FILE", words: Self.words)) },
            verify: { _, _ in true }, saveFile: { _ in .cancelled }, printWords: { _ in .savedAsFile }),
            continueTitle: EncryptionCopy.encryptContinue, onContinue: { _ in })
        pdf.passphrase = "correct horse battery"
        pdf.confirmation = "correct horse battery"
        await pdf.createRecoveryKey()
        pdf.printWords()
        try write(KeySetupRedesignView(model: pdf), "setup-5-print-to-pdf-refused", size: CGSize(width: 680, height: 1400), to: dir)
    }

    /// The launch window's screens, at the main window's smallest size.
    func testRenderLaunchScreens() async throws {
        let dir = try folder()
        let window = CGSize(width: 900, height: 600)
        func screen(_ s: LaunchScreen, _ name: String, busy: Bool = false) throws {
            try write(LaunchBlockingRedesignView(screen: s, busy: busy) { _ in }, name, size: window, to: dir)
        }

        let onboarding = EncryptionSetupModel(actions: setupActions(), onContinue: { _ in })
        try write(KeySetupRedesignView(model: onboarding, leave: .init(title: EncryptionCopy.quitAction, isEscape: false, isEnabled: true, run: {})), "launch-01-onboarding",
                  size: CGSize(width: 900, height: 760), to: dir)
        try write(LaunchMigrationRedesignView(fraction: 0.42), "launch-02-migration-progress", size: window, to: dir)
        let recovery = EncryptionRecoveryModel(reason: .keyRejected, actions: .init(
            chooseFile: { .chosen(name: "Whimbrel recovery key.txt", contents: "FILE") },
            restoreFromFile: { _, _ in .success(Data(count: 32)) }, restoreFromWords: { _ in .failure(.badWords) },
            useKey: { _ in .keyRejected }))
        await recovery.perform(.chooseFile)
        recovery.passphrase = "correct horse battery"
        await recovery.restore()
        try write(RecoveryRedesignView(model: recovery, quit: {}), "launch-03-recovery-wrong-key",
                  size: CGSize(width: 900, height: 700), to: dir)
        let unavailable = EncryptionKeyUnavailableModel(detail: "") { false }
        await unavailable.retry()
        try write(KeyUnavailableRedesignView(model: unavailable, quit: {}), "launch-04-key-unavailable",
                  size: window, to: dir)
        try screen(.keyStorageUnavailable, "launch-05-hard-stop-key-storage")
        try screen(.migrationInterrupted, "launch-06-interrupted")
        try screen(.damaged(.damaged), "launch-07-damaged")
        try screen(.damaged(.noAccess), "launch-08-no-access")
        try screen(.encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: nil), "launch-09-missing-backup-present")
        try screen(.encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false,
                                         lastAction: .restore(CoreRestoreResult(action: .partlyDone, movedAside: 1, warning: .none))),
                   "launch-10-missing-backup-partly-done")
        try screen(.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: true, lastAction: nil), "launch-11-missing-encrypted-copy-only")
        try screen(.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false, lastAction: nil), "launch-12-missing-no-copy")
        try screen(.error(.databaseMissingForRecoveredKey), "launch-13-error-file-gone-for-recovery-key")
        try screen(.error(.decisionUnavailable), "launch-14-error-generic", busy: true)
        try screen(.error(.plainOpenFailed), "launch-15-plain-open-failed")
        // Running without the memory database: the notice at the top of the main window / menu bar.
        try write(VStack(spacing: 0) { NoStoreNotice {}; Spacer() }.background(Color.whimbrelBackgroundPage),
                  "launch-17-no-store-notice", size: CGSize(width: 900, height: 160), to: dir)
        // The menu bar window is 320 points wide.
        try write(VStack(alignment: .leading, spacing: 0) {
            NoStoreNotice {}
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Button("Start Meeting") {}.buttonStyle(.whimbrelSecondary).recordingControl()
                RecordingOffCaption()
            }
            .padding(Spacing.md)
            Spacer()
        }
        .environment(\.recordingControls, RecordingControls(storeOpen: false))
        .background(Color.whimbrelBackgroundPage), "launch-18-no-store-menubar", size: CGSize(width: 320, height: 260), to: dir)
        try screen(.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false, lastAction: .reset(.failed)),
                   "launch-16-missing-no-copy-reset-failed")
    }
}
