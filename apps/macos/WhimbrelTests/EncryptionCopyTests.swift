import XCTest
@testable import Whimbrel

/// Copy rules for the encryption screens (X4 Task 10, design section 7) and the Swift bridge
/// to core's recovery-key calls.
final class EncryptionCopyTests: XCTestCase {
    func testNoBannedPhrasesOrDashes() {
        XCTAssertGreaterThan(EncryptionCopy.all.count, 40)
        for s in EncryptionCopy.all {
            let lower = s.lowercased()
            for banned in ["nothing leaves", "never leaves", "we never receive", "encrypted at rest", "hmac", "sqlcipher",
                           "argon", "bip-39", "bip39", "aes"] {
                XCTAssertFalse(lower.contains(banned), "\(banned) in: \(s)")
            }
            XCTAssertFalse(s.contains("\u{2014}"), "em dash in: \(s)")
            XCTAssertFalse(s.contains("\u{2013}"), "en dash in: \(s)")
        }
    }

    func testTheWordsAreTheRawKeyNotice() {
        let notice = EncryptionCopy.wordsAreTheKey.lowercased()
        XCTAssertTrue(notice.contains("are the key itself"), notice)
        XCTAssertTrue(notice.contains("passphrase does not protect them"), notice)
        XCTAssertTrue(notice.contains("anyone"), notice)
        XCTAssertTrue(notice.contains("memory database"), notice)
    }

    func testEncryptionIsScopedToTheMemoryDatabase() {
        for s in [EncryptionCopy.settingsNotEncryptedBody, EncryptionCopy.settingsEncryptedBody, EncryptionCopy.setupIntro] {
            XCTAssertTrue(s.contains("memory database"), s)
        }
    }

    /// Task 13c (Mike's decisions 2026-11-16): the three sentences are exactly his words.
    func testTask13cSentencesAreVerbatim() {
        XCTAssertEqual(EncryptionCopy.deletedLibraryLine, "Your previous data was deleted. This is a new, empty library.")
        XCTAssertEqual(EncryptionCopy.libraryMissingNotice,
                       "Your library couldn't be found, so Whimbrel started a new, empty one. If you have a backup of your library, the help page \"Restoring a library backup\" explains how to put it back.")
        // The notice's link names the same page and goes to it on the website.
        XCTAssertEqual(EncryptionCopy.libraryBackupHelpLink, "Restoring a library backup")
        XCTAssertTrue(EncryptionCopy.libraryMissingNotice.contains("\"\(EncryptionCopy.libraryBackupHelpLink)\""))
        XCTAssertEqual(EncryptionCopy.libraryBackupHelpURL.absoluteString, "https://whimbrelapp.com/help/restore-library-backup/")
        let page = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("website/site/pages/help/restore-library-backup.html")
        XCTAssertTrue(FileManager.default.fileExists(atPath: page.path), "the help page exists: \(page.path)")
        let google = YourData.outbound(.init(hostedProviders: [], localOnly: false, syncOn: false, syncDevices: 0,
                                             googleAccounts: [], typesafeOn: false)).first { $0.channel == .google }
        XCTAssertEqual(google?.sees,
                       "Google sees the app reading your Gmail and Calendar. The data is stored in this Mac's database. It is only sent on to us if you turn on a feature listed on this page.")
        for s in [EncryptionCopy.deletedLibraryLine, EncryptionCopy.libraryMissingNotice, EncryptionCopy.libraryBackupHelpLink] {
            XCTAssertTrue(EncryptionCopy.all.contains(s), s)
        }
    }

    /// Task 15 (Mike 2026-11-17): Delete everything keeps connected Google accounts, and the
    /// sheet says so; the new lines follow the same rules (no forbidden claim, no dashes).
    func testDeleteEverythingSaysGoogleAccountsStay() {
        let body = YourData.deleteSheetBody(dataFolder: "12 MB", mediaFolder: "/Users/x/Pictures/Whimbrel", media: "3 MB")
        XCTAssertTrue(body.hasSuffix("Settings, keys, connected Google accounts and downloaded models stay."), body)
        XCTAssertTrue(body.contains("(12 MB)") && body.contains("/Users/x/Pictures/Whimbrel (3 MB)"), body)
        XCTAssertEqual(YourData.googleAccountsNotKeptFailure, "Could not keep your connected Google accounts. Nothing was deleted; try again.")
        for s in [body, YourData.googleAccountsNotKeptFailure] {
            let lower = s.lowercased()
            for banned in ["nothing leaves", "never leaves", "we never receive", "disconnect", "revoke"] {
                XCTAssertFalse(lower.contains(banned), "\(banned) in: \(s)")
            }
            XCTAssertFalse(s.contains("\u{2014}") || s.contains("\u{2013}"), "dash in: \(s)")
        }
    }

    /// The Your data rows follow the same privacy rule: no "we never receive" and the like
    /// outside local-only mode, in every input state.
    func testWhatLeavesRowsMakeNoForbiddenClaims() {
        for localOnly in [false, true] {
            let rows = YourData.outbound(.init(hostedProviders: ["Anthropic"], localOnly: localOnly, syncOn: true, syncDevices: 2,
                                               googleAccounts: ["a@example.com"], typesafeOn: true))
            for row in rows {
                for s in [row.title, row.when, row.sees, row.state] {
                    let lower = s.lowercased()
                    for banned in ["nothing leaves", "never leaves", "we never receive"] {
                        XCTAssertFalse(lower.contains(banned), "\(banned) in: \(s)")
                    }
                    XCTAssertFalse(s.contains("\u{2014}") || s.contains("\u{2013}"), "dash in: \(s)")
                }
            }
        }
    }

    /// The set-aside files row (Task 13c item 3): count and size in the row and in the
    /// confirmation, which says the files cannot be recovered.
    func testForeignFilesCopy() {
        XCTAssertEqual(EncryptionCopy.foreignFilesSummary(count: 3, size: "1.2 GB"), "3 files kept aside, 1.2 GB")
        XCTAssertEqual(EncryptionCopy.foreignFilesSummary(count: 1, size: "4 KB"), "1 file kept aside, 4 KB")
        let body = EncryptionCopy.foreignFilesConfirmBody(count: 3, size: "1.2 GB")
        XCTAssertTrue(body.contains("3 files") && body.contains("1.2 GB") && body.contains("cannot be recovered"), body)
        let one = EncryptionCopy.foreignFilesConfirmBody(count: 1, size: "4 KB")
        XCTAssertTrue(one.contains("1 file ") && one.contains("4 KB") && one.contains("cannot be recovered"), one)
        XCTAssertEqual(EncryptionCopy.foreignFilesDeleteAction, "Delete these files")
        for s in [EncryptionCopy.foreignFilesSummary(count: 2, size: "1 MB"), EncryptionCopy.foreignFilesHelp,
                  EncryptionCopy.foreignFilesDeleteAction, EncryptionCopy.foreignFilesConfirmTitle, body,
                  EncryptionCopy.foreignFilesConfirm, EncryptionCopy.foreignFilesNotDeleted, EncryptionCopy.foreignFilesUnreadable] {
            XCTAssertTrue(EncryptionCopy.all.contains(s), s)
        }
    }

    func testRecoveryFileName() {
        XCTAssertEqual(EncryptionCopy.recoveryFileName, "Whimbrel recovery key.txt")
    }
}

/// The words, QR and recovery file are the key (or sealed key): no encryption source writes
/// them to the pasteboard or logs them, and no pasteboard-writing file in the app touches the
/// encryption types. Scans the app source found from this file's compile-time path.
final class EncryptionSecretHygieneTests: XCTestCase {
    private let appRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Whimbrel")

    private func swiftFiles() throws -> [URL] {
        let e = try XCTUnwrap(FileManager.default.enumerator(at: appRoot, includingPropertiesForKeys: nil))
        return e.compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" }
    }

    private func isEncryptionFile(_ url: URL) -> Bool {
        url.lastPathComponent.hasPrefix("Encryption") || url.lastPathComponent == "CoreEncryptionSetup.swift"
    }

    func testEncryptionSourcesNeverTouchThePasteboardOrLogs() throws {
        let files = try swiftFiles().filter(isEncryptionFile)
        XCTAssertGreaterThanOrEqual(files.count, 5, "found \(files.map(\.lastPathComponent)) under \(appRoot.path)")
        let banned = try NSRegularExpression(pattern:
            #"NSPasteboard|UIPasteboard|\bprint\(|debugPrint\(|NSLog\(|os_log\(|Logger\(|\bdump\(|\.log\.|logger\.|\.(info|debug|notice|error|fault)\(|copyable|draggable|ShareLink|NSSharingService|onCopyCommand"#)
        for url in files {
            let text = try String(contentsOf: url, encoding: .utf8)
            let hits = banned.matches(in: text, range: NSRange(text.startIndex..., in: text))
                .map { (text as NSString).substring(with: $0.range) }
            XCTAssertEqual(hits, [], url.lastPathComponent)
            if text.contains("struct EncryptionWordGrid") {
                XCTAssertFalse(text.contains(".textSelection(.enabled)"), "the words must not be selectable for copying")
            }
        }
    }

    func testPasteboardWritersDoNotHandleRecoveryMaterial() throws {
        let writes = try NSRegularExpression(pattern: #"\.(setString|setData|writeObjects|declareTypes)\("#)
        let secrets = ["CoreSetupMaterial", "EncryptionSetupModel", "CoreEncryptionSetup", "EncryptionRecoveryModel", "recoveryFile", "EncryptionQR"]
        for url in try swiftFiles() {
            let text = try String(contentsOf: url, encoding: .utf8)
            guard writes.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil else { continue }
            XCTAssertFalse(isEncryptionFile(url), url.lastPathComponent)
            for s in secrets {
                XCTAssertFalse(text.contains(s), "\(url.lastPathComponent) writes the pasteboard and mentions \(s)")
            }
        }
    }
}

/// `CoreEncryptionSetup` over the real C ABI (fast in a release core build).
final class EncryptionSetupFFITests: XCTestCase {
    private let pass = "correct horse battery staple"

    func testBeginVerifyAndRestoreAgree() throws {
        let m = try CoreEncryptionSetup.begin(passphrase: pass).get()
        XCTAssertEqual(m.key.count, 32)
        XCTAssertEqual(m.words.count, 24)
        XCTAssertTrue(m.recoveryFile.hasPrefix("whimbrel-recovery 1\n"))
        XCTAssertTrue(CoreEncryptionSetup.verify(recoveryFile: m.recoveryFile, passphrase: pass))
        XCTAssertFalse(CoreEncryptionSetup.verify(recoveryFile: m.recoveryFile, passphrase: pass + "!"))
        XCTAssertEqual(try CoreEncryptionSetup.restore(recoveryFile: m.recoveryFile, passphrase: pass).get(), m.key)
        let messy = "  " + m.words.map { $0.uppercased() }.joined(separator: "  \n") + "\n"
        XCTAssertEqual(try CoreEncryptionSetup.restore(words: messy).get(), m.key)
    }

    func testExistingKeyIsKept() throws {
        let key = Data(repeating: 9, count: 32)
        let m = try CoreEncryptionSetup.begin(passphrase: pass, existingKey: key).get()
        XCTAssertEqual(m.key, key)
    }

    func testErrors() {
        XCTAssertEqual(CoreEncryptionSetup.begin(passphrase: "short").failure, .passphraseTooShort)
        XCTAssertEqual(CoreEncryptionSetup.restore(recoveryFile: "nope", passphrase: pass).failure, .malformed)
        XCTAssertEqual(CoreEncryptionSetup.restore(words: "abandon abandon").failure, .badWords)
        XCTAssertEqual(CoreEncryptionSetup.begin(passphrase: pass, existingKey: Data(count: 31)).failure, .invalidArgument)
    }

    func testLengthIsCountedAfterNFC() {
        XCTAssertEqual(CoreEncryptionSetup.minimumPassphraseLength, 12)
        XCTAssertEqual(CoreEncryptionSetup.passphraseLength("cafe\u{301}"), 4)
        XCTAssertEqual(CoreEncryptionSetup.passphraseLength("caf\u{e9}"), 4)
    }
}

private extension Result {
    var failure: Failure? {
        if case let .failure(e) = self { return e }
        return nil
    }
}
