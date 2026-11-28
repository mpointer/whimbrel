import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the redesigned Memory & Privacy section (`UI/Redesign/Settings/MemoryPrivacySettingsRedesign.swift`):
/// the Memory row's state, the retention decisions, the anchors, the words and the search mapping. Nothing here opens a
/// window, reads the app's defaults or depends on the machine's language, time zone or date.
final class SettingsRedesignMemoryTests: XCTestCase {
    private let at = Date(timeIntervalSince1970: 1_700_000_000)

    // MARK: Memory row

    func testTheMemoryRowUsesTheMenusStateRule() {
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at, capturing: true), .on)
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at, capturing: false), .onNotCapturing)
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: false, resumeAt: nil, now: at, capturing: false), .off)
        // The hold: a state that has not lasted a few seconds is not yet a problem.
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at.addingTimeInterval(2), capturing: false, since: at), .on)
        XCTAssertEqual(HomeRedesignAdapter.memoryState(enabled: true, resumeAt: nil, now: at.addingTimeInterval(30), capturing: false, since: at), .onNotCapturing)
        if case .pausedUntil = HomeRedesignAdapter.memoryState(enabled: false, resumeAt: at.addingTimeInterval(900), now: at) {} else { XCTFail("expected a pause") }
    }

    func testTheReasonIsTheMenusReasonAndOnlyWhenNotCapturing() {
        for state in [WMemoryState.on, .off, .pausedUntil("3:10 PM")] {
            XCTAssertNil(MemoryPrivacyAdapter.memoryReason(memory: state, accessibilityGranted: false, noStoreShown: false))
        }
        let noPermission = MemoryPrivacyAdapter.memoryReason(memory: .onNotCapturing, accessibilityGranted: false, noStoreShown: false)
        XCTAssertEqual(noPermission?.hasPrefix(String(localized: "Whimbrel can't read text on your screen.", comment: "Line in the menu bar menu under Needs attention: the Accessibility permission is off, so screen text is not saved to memory and Ask about this window cannot read a window.")), true)
        let other = MemoryPrivacyAdapter.memoryReason(memory: .onNotCapturing, accessibilityGranted: true, noStoreShown: false)
        XCTAssertEqual(other?.hasPrefix(String(localized: "Whimbrel isn\u{2019}t saving text from your screen.", comment: "Line in the menu bar menu under Needs attention: memory is on but nothing is being saved from the screen, and no permission explains it.")), true)
        XCTAssertNil(MemoryPrivacyAdapter.memoryReason(memory: .onNotCapturing, accessibilityGranted: true, noStoreShown: true))
    }

    func testTheControlsFollowTheState() {
        XCTAssertEqual(WMemoryControls.controls(for: .on, storeOpen: true).toggle, .turnOff)
        XCTAssertEqual(WMemoryControls.controls(for: .off, storeOpen: true).toggle, .turnOn)
        XCTAssertFalse(WMemoryControls.controls(for: .off, storeOpen: true).showsPause)
        XCTAssertNil(WMemoryControls.controls(for: .pausedUntil("3:10 PM"), storeOpen: true).toggle)
        XCTAssertFalse(WMemoryControls.controls(for: .on, storeOpen: false).isEnabled)
    }

    // MARK: Retention

    func testRetentionDecisionsMatchTheSpec() {
        let limited = WRetentionStored(days: 90, keepForever: false)
        XCTAssertEqual(WRetentionDecision.commit(stored: limited, proposed: 120), .save(days: 120))
        XCTAssertEqual(WRetentionDecision.commit(stored: limited, proposed: 90), .nothingToSave)
        XCTAssertEqual(WRetentionDecision.commit(stored: limited, proposed: 30), .askFirst(days: 30, replacing: .days(90)))
        XCTAssertEqual(WRetentionDecision.commit(stored: WRetentionStored(days: 30, keepForever: true), proposed: 30), .askFirst(days: 30, replacing: .forever))
        XCTAssertEqual(WRetentionDecision.commit(stored: limited, proposed: nil), .refuse(.empty))
        XCTAssertEqual(WRetentionDecision.commit(stored: limited, proposed: 0), .refuse(.belowMinimum(1)))
        XCTAssertEqual(WRetentionDecision.commit(stored: limited, proposed: 4000), .refuse(.aboveMaximum(3650)))
        XCTAssertEqual(WRetentionDecision.numberToShow(stored: WRetentionStored(days: 45, keepForever: true)), 45)
    }

    func testTheRowsUseTheNamingTableAndKeepForever() {
        let rows = MemoryPrivacyAdapter.retentionModels(captureDays: 30, captureForever: true, meetingDays: 90, meetingForever: false, conversationDays: 30, conversationForever: true, firstAnchor: "a")
        XCTAssertEqual(rows.map(\.label), [
            String(localized: "Screen text", comment: "Name of a retention row in Settings: how long Memory keeps the text it saved from your screen."),
            String(localized: "Meetings and recordings", comment: "Name of a retention row in Settings: how long meetings and their recordings are kept."),
            String(localized: "Conversations"),
        ])
        XCTAssertTrue(rows.allSatisfy { $0.foreverTitle == String(localized: "Keep forever") && $0.asksBeforeShortening })
        XCTAssertEqual(rows.map(\.keepForever), [true, false, true])
        let media = MemoryPrivacyAdapter.mediaModel(folder: "/m", canReset: false, refusal: nil, compressDays: 30, compressForever: true, deleteDays: 365, deleteForever: true, shrinkDays: 90, shrinkForever: true, deleteNote: nil, usage: nil, preview: nil, progress: nil, fraction: nil)
        // "Keep forever" means the audio is never deleted on Delete audio after, but on Compress and Shrink it means "never
        // compress" and "never shrink", so those two say "Keep full quality".
        XCTAssertEqual(media.deleteAudio.foreverTitle, String(localized: "Keep forever"))
        XCTAssertEqual([media.compress, media.shrink].map(\.foreverTitle), Array(repeating: String(localized: "Keep full quality", comment: "Checkbox on the Compress audio after and Shrink images after rows in Settings: when it is on, the audio or the images are never made smaller."), count: 2))
        XCTAssertNotEqual(media.compress.foreverTitle, media.deleteAudio.foreverTitle)
        // Every row that destroys or replaces data asks before a shorter period, compress and shrink included.
        XCTAssertTrue([media.compress, media.deleteAudio, media.shrink].allSatisfy(\.asksBeforeShortening))
        XCTAssertEqual([media.compress.effect, media.deleteAudio.effect, media.shrink.effect], [.recompressesAudio, .deletes, .shrinksImages])
        XCTAssertEqual(rows.map(\.effect), [.deletes, .deletes, .deletes])
    }

    /// The question never uses the row's label as a noun ("Delete Delete audio after older than 7 days?"): each row has
    /// its own sentence, built with the real adapter labels.
    func testEveryRetentionQuestionHasItsOwnSentenceNotTheRowLabel() {
        let l = MemoryPrivacyAdapter.labels()
        let media = MemoryPrivacyAdapter.mediaModel(folder: "/m", canReset: false, refusal: nil, compressDays: 30, compressForever: false, deleteDays: 30, deleteForever: false, shrinkDays: 30, shrinkForever: false, deleteNote: nil, usage: nil, preview: nil, progress: nil, fraction: nil)
        let deleteAudio = WRetentionAsk(effect: .deletes, kind: .deleteAudio, name: media.deleteAudio.label, days: 7, replacing: .days(30), count: nil)
        XCTAssertEqual(l.askTitle(deleteAudio), String(localized: "Delete audio from recordings older than \(7) days?"))
        XCTAssertTrue(l.askMessage(deleteAudio).hasPrefix(String(localized: "Audio from recordings is kept for \(30) days today.")), l.askMessage(deleteAudio))
        let screen = WRetentionAsk(effect: .deletes, kind: .screenText, name: "", days: 7, replacing: .forever, count: nil)
        XCTAssertEqual(l.askTitle(screen), String(localized: "Delete screen text older than \(7) days?"))
        XCTAssertTrue(l.askMessage(screen).hasPrefix(String(localized: "Screen text is kept forever today.")))
        for ask in MemoryPrivacyAdapter.retentionAsks {
            for label in [media.deleteAudio.label, media.compress.label, media.shrink.label] {
                XCTAssertFalse(l.askTitle(ask).contains(label), "\(l.askTitle(ask)) / \(label)")
                XCTAssertFalse(l.askMessage(ask).contains(label), "\(l.askMessage(ask)) / \(label)")
            }
        }
    }

    /// Each companion checkbox writes the stored flag its row is named for, on a throwaway defaults suite (not the app's).
    @MainActor
    func testEachMediaCheckboxWritesItsOwnFlag() {
        let suite = "whimbrel.test.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let media = MediaSettings(defaults: defaults)
        media.keepFullAudioForever = false; media.neverDeleteAudio = false; media.keepFullImagesForever = false
        MemoryPrivacyAdapter.applyMedia("compress", days: nil, forever: true, to: media)
        XCTAssertEqual([media.keepFullAudioForever, media.neverDeleteAudio, media.keepFullImagesForever], [true, false, false])
        MemoryPrivacyAdapter.applyMedia("deleteAudio", days: nil, forever: true, to: media)
        XCTAssertEqual([media.keepFullAudioForever, media.neverDeleteAudio, media.keepFullImagesForever], [true, true, false])
        MemoryPrivacyAdapter.applyMedia("shrink", days: 7, forever: false, to: media)
        XCTAssertEqual([media.keepFullAudioForever, media.neverDeleteAudio, media.keepFullImagesForever], [true, true, false])
        XCTAssertEqual(media.shrinkImagesAfterDays, 7)
        MemoryPrivacyAdapter.applyMedia("compress", days: 14, forever: false, to: media)
        XCTAssertEqual(media.compressAfterDays, 14)
        XCTAssertFalse(media.keepFullAudioForever)
        // A flag set on the model's row means the same flag the classic pane writes: the policy drops a step when its flag is on.
        XCTAssertNil(media.policy.deleteAudioAfterDays)
        XCTAssertEqual(media.policy.shrinkImagesAfterDays, 7)
    }

    /// The encryption card appears when the launch wiring arrives after the section opened: a box made without it stays empty,
    /// and asking again with nothing available changes nothing.
    @MainActor
    func testTheEncryptionBoxStaysEmptyWithoutTheLaunchWiring() {
        let box = LibraryEncryptionBox(live: nil)
        XCTAssertNil(box.live)
        XCTAssertNil(box.model)
        XCTAssertNil(box.card())
        box.attachIfNeeded(nil)
        XCTAssertNil(box.live)
        XCTAssertFalse(box.showsSetup)
    }

    func testApplyRetentionNowHasNoConfirmWhenEveryRowKeepsForever() {
        XCTAssertFalse(WRetentionApplyPlan.hasLimit([WRetentionStored(days: 30, keepForever: true)]))
        XCTAssertTrue(WRetentionApplyPlan.hasLimit([WRetentionStored(days: 30, keepForever: false)]))
    }

    func testApplyingRetentionNamesOnlyTheLimitedClasses() {
        let limits = WRetentionApplyPlan.limits([(.screenText, WRetentionStored(days: 30, keepForever: true)), (.meetings, WRetentionStored(days: 90, keepForever: false))])
        XCTAssertEqual(limits, [WRetentionApplyPlan.Limit(kind: .meetings, days: 90)])
    }

    /// Apply Retention Now: a full sentence per kind with a period (the real adapter's strings), none for a kind kept forever,
    /// and the day count agrees in one and in thirty.
    func testApplyRetentionNowConfirmIsOneSentencePerKindWithAPeriod() {
        let l = MemoryPrivacyAdapter.labels()
        let text = l.applyConfirmMessage([.init(kind: .meetings, days: 90), .init(kind: .screenText, days: 1)])
        XCTAssertEqual(text, [String(localized: "Meetings and recordings older than \(90) days are deleted."), String(localized: "Screen text older than \(1) days is deleted."),
                              String(localized: "This can't be undone.")].joined(separator: "\n"))
        XCTAssertFalse(text.contains(": older than"))
        XCTAssertEqual(l.applyConfirmMessage([]), String(localized: "Every period is set to Keep forever, so nothing will be deleted."))
        XCTAssertTrue(l.applyConfirmMessage([.init(kind: .conversations, days: 30)]).hasPrefix(String(localized: "Conversations older than \(30) days are deleted, except starred ones.")))
    }

    /// Closing the window clears only the secret drafts (a token the secret rows and the Connectors sheet listen for); the window's
    /// content is not rebuilt, so a running export, compact or delete is not shown as idle on reopen.
    func testClosingTheWindowDoesNotRebuildTheSections() throws {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/Redesign/Settings/SettingsRedesignView.swift")
        let text = try String(contentsOf: url, encoding: .utf8)
        XCTAssertFalse(text.contains(".id(session)"))
        XCTAssertTrue(text.contains("sections.environment(\\.wSecretClearToken, secretSession)"))
    }

    func testTheCountSentenceNamesTheThingAndFormatsTheNumberForTheLanguage() {
        let l = MemoryPrivacyAdapter.labels()
        let ask = WRetentionAsk(effect: .deletes, kind: .screenText, name: "", days: 30, replacing: .forever, count: 3904)
        let n = Int64(3904).formatted(), d = SettingsCopy.retentionDays(30)
        XCTAssertTrue(l.askMessage(ask).hasPrefix(String(localized: "\(n) screen captures are older than \(d).")), l.askMessage(ask))
        XCTAssertFalse(l.askMessage(ask).contains(" 3904 "))
        let one = WRetentionAsk(effect: .deletes, kind: .screenText, name: "", days: 1, replacing: .forever, count: 1)
        XCTAssertTrue(l.askMessage(one).hasPrefix(String(localized: "\(Int64(1).formatted()) screen capture is older than \(SettingsCopy.retentionDays(1)).")), l.askMessage(one))
    }

    /// The stepper's minimum is 1, so "1 days" must not appear in any sentence that carries a day count, in English.
    func testADayCountAgreesInOneAndInThirty() throws {
        let l = MemoryPrivacyAdapter.labels()
        XCTAssertNotEqual(l.days(1), l.days(30))
        XCTAssertEqual(l.savePeriod(1), String(localized: "Keep for \(1) days"))
        guard Locale.preferredLanguages.first?.hasPrefix("en") == true else { return }
        XCTAssertEqual(l.days(1), "1 day")
        XCTAssertEqual(l.days(30), "30 days")
        XCTAssertEqual(l.savePeriod(1), "Keep for 1 day")
        XCTAssertEqual(l.savePeriod(30), "Keep for 30 days")
        XCTAssertEqual(l.periodProblem(.belowMinimum(1)), "Use 1 day or more. Nothing was changed.")
        let ask = WRetentionAsk(effect: .deletes, kind: .screenText, name: "", days: 1, replacing: .days(1), count: nil)
        XCTAssertEqual(l.askTitle(ask), "Delete screen text older than 1 day?")
        XCTAssertTrue(l.askMessage(ask).hasPrefix("Screen text is kept for 1 day today."), l.askMessage(ask))
        XCTAssertEqual(l.askConfirm(ask), "Keep 1 day and delete older")
        for text in MemoryPrivacyAdapter.ownStrings + [l.askTitle(ask), l.askMessage(ask)] { XCTAssertFalse(text.contains("1 days"), text) }
    }

    // MARK: Anchors

    func testTheSectionDeclaresExactlyTheRegistryIds() {
        let registry = SettingsSearch.entries.filter { $0.section == .capture }.map(\.id)
        XCTAssertEqual(Set(SettingsAnchorID.memoryPrivacy.all), Set(registry))
        XCTAssertEqual(SettingsAnchorID.memoryPrivacy.all.count, 12)
        XCTAssertEqual(SettingsRedesign.declaredAnchorIDs(for: .capture), SettingsAnchorID.memoryPrivacy.all)
        XCTAssertFalse(SettingsRedesign.hostsClassicPanes(.capture))
    }

    // MARK: Words

    func testNoRedesignedSentenceSaysMemoryDatabase() {
        let all = MemoryPrivacyAdapter.ownStrings + LibraryCopyRedesign.pairs.map(\.redesigned)
        for text in all {
            XCTAssertFalse(text.lowercased().contains("memory database"), text)
        }
        // Every classic sentence the card can show that says it is swapped for one that does not.
        for pair in LibraryCopyRedesign.pairs {
            XCTAssertNotEqual(pair.classic, pair.redesigned)
            XCTAssertEqual(LibraryCopyRedesign.map(pair.classic), pair.redesigned)
        }
        XCTAssertEqual(LibraryCopyRedesign.map("something else"), "something else")
    }

    /// Decision 3 reaches every redesigned surface: each classic sentence that says "memory database" is either swapped for a
    /// "your library" one or listed as drawn only by shared classic code. The sentences are found in the source file by their
    /// English text, and looked up the way the app looks them up, so the check reads the same in every language.
    func testEveryClassicMemoryDatabaseSentenceIsSwappedOrListed() throws {
        let file = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel/UI/EncryptionCopy.swift")
        let source = try String(contentsOf: file, encoding: .utf8)
        let literals = source.components(separatedBy: "String(localized: \"").dropFirst().compactMap { chunk -> String? in
            guard let end = chunk.range(of: "\")") else { return nil }
            let text = String(chunk[..<end.lowerBound])
            return text.lowercased().contains("memory database") && !text.contains("\\(") ? text : nil
        }
        XCTAssertGreaterThan(literals.count, 30)
        let covered = Set(LibraryCopyRedesign.pairs.map(\.classic) + LibraryCopyRedesign.classicOnly)
        for literal in literals {
            let classic = String(localized: String.LocalizationValue(literal))
            XCTAssertTrue(covered.contains(classic), "No redesigned sentence for: \(literal)")
        }
        XCTAssertEqual(Set(LibraryCopyRedesign.pairs.map(\.classic)).count, LibraryCopyRedesign.pairs.count)
    }

    /// What the redesigned launch screens, the recovery screen and the key-unavailable screen put on screen says "your
    /// library", for every blocking screen the launch can stop on.
    @MainActor
    func testTheRedesignedLaunchScreensNeverSayMemoryDatabase() {
        let screens: [LaunchScreen] = [
            .keyStorageUnavailable, .recovery(.needsKey), .recovery(.keyRejected), .migrationInterrupted,
            .damaged(.damaged), .damaged(.noAccess), .damaged(.openElsewhere),
            .encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: nil),
            .encryptedFileMissing(backupPresent: false, encryptedCopyPresent: true, lastAction: nil),
            .encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false, lastAction: nil),
            .error(.filesUnreadable), .error(.plainOpenFailed), .error(.databaseMissingForRecoveredKey), .error(.keyMissing(.dbKey)),
            .error(.decisionUnavailable), .error(.keyedOpenFailed(.damaged(""))), .error(.keyedOpenFailed(.startupFailed(""))),
        ]
        var shown: [String] = []
        for screen in screens {
            let content = LaunchScreenContent.of(screen)
            let model = LaunchBlockingRedesignView.model(content, busy: false, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
            shown += [model.title] + model.paragraphs + model.hints + [model.errorLine].compactMap { $0 } + model.actions.map(\.title)
            shown.append(LibraryCopyRedesign.map(content.resetConfirmBody ?? EncryptionCopy.resetConfirmBody))
        }
        shown += [LaunchMigrationRedesignView.model(fraction: 0.5)].flatMap { [$0.title] + $0.paragraphs + $0.hints }
        let unavailable = KeyUnavailableRedesignView.content(EncryptionKeyUnavailableModel(detail: "") { true }, quit: nil, retry: {})
        shown += [unavailable.title] + unavailable.paragraphs + unavailable.hints
        for reason in [EncryptionRecoveryModel.Reason.keyMissing, .keyRejected] {
            let model = EncryptionRecoveryModel(reason: reason, actions: .init(
                chooseFile: { .cancelled }, restoreFromFile: { _, _ in .failure(.malformed) }, restoreFromWords: { _ in .failure(.badWords) }, useKey: { _ in .opened }))
            let m = RecoveryRedesignView.content(model, quit: nil, unlock: {}, chooseFile: {})
            shown += [m.title, m.reason] + m.finePrint
        }
        XCTAssertGreaterThan(shown.count, 40)
        for text in shown { XCTAssertFalse(text.lowercased().contains("memory database"), text) }
        for text in shown { XCTAssertFalse(text.lowercased().contains("the database"), text) }
        // In any language: nothing on these screens is still a classic sentence that has a redesigned one.
        let classic = Set(LibraryCopyRedesign.pairs.map(\.classic))
        for text in shown { XCTAssertFalse(classic.contains(text), text) }
    }

    /// The notice, the two recording-refused messages and the printed page say "your library".
    func testTheSharedLibrarySentencesSayYourLibrary() {
        let sentences = [(EncryptionCopy.noStoreNotice, LibraryCopyRedesign.noStoreNotice), (EncryptionCopy.recordingBlockedNoStore, LibraryCopyRedesign.recordingBlockedNoStore),
                         (EncryptionCopy.recordingNotSaved, LibraryCopyRedesign.recordingNotSaved), (EncryptionCopy.printFooter, LibraryCopyRedesign.printFooter),
                         (EncryptionCopy.wordsAreTheKey, LibraryCopyRedesign.wordsAreTheKey)]
        for (classic, expected) in sentences {
            let redesigned = LibraryCopyRedesign.map(classic)
            XCTAssertNotEqual(redesigned, classic)
            // The redesigned sentence itself, from the same localized lookup the code uses, not a word of English.
            XCTAssertEqual(redesigned, expected)
            XCTAssertFalse(redesigned.lowercased().contains("memory database"), redesigned)
        }
        XCTAssertEqual(RecordingStartInsert.outcome(insertedMeetingId: nil), .refuse(message: LibraryCopyRedesign.recordingNotSaved))
        XCTAssertNotEqual(EncryptionPrintCopy.forSkin(), EncryptionPrintCopy())
        XCTAssertFalse(EncryptionPrintCopy.forSkin().footer.lowercased().contains("memory database"))
        XCTAssertFalse(EncryptionPrintCopy.forSkin().notice.lowercased().contains("memory database"))
        XCTAssertEqual(MenuBarAdapter.noStoreNeed(openSettings: {}).title, LibraryCopyRedesign.noStoreNotice)
    }

    func testTheClassicLibrarySentencesTheCardShowsAreAllCovered() {
        let shown = [
            EncryptionCopy.settingsNotEncryptedBody, EncryptionCopy.settingsEncryptedBody, EncryptionCopy.encryptDone, EncryptionCopy.encryptAtNextLaunch,
            EncryptionCopy.skipLowDisk, EncryptionCopy.skipOpenElsewhere, EncryptionCopy.attentionNeedsKey, EncryptionCopy.attentionKeyRejected,
            EncryptionCopy.attentionMigrationInterrupted, EncryptionCopy.attentionDamaged, EncryptionCopy.attentionStartupFailed,
            EncryptionCopy.noteKeyStoreUnavailable, EncryptionCopy.noteSkippedOpenElsewhere, EncryptionCopy.notePlainBackupKept,
            EncryptionCopy.noteRestoredCopy, EncryptionCopy.noteRecoveredKeyNotStored, EncryptionCopy.noteRecoveredKeyNoSlot,
            EncryptionCopy.noteRecoveredKeyUnverified, EncryptionCopy.notePendingKeyNotPromoted, EncryptionCopy.notePendingKeyKept,
            EncryptionCopy.noteStoreUnavailableHint, EncryptionCopy.fastSearchHelp, EncryptionCopy.speedEncrypted, EncryptionCopy.speedNotEncrypted,
        ]
        for text in shown { XCTAssertFalse(LibraryCopyRedesign.map(text).lowercased().contains("memory database"), text) }
    }

    func testNewTextHasNoDashesOrHypeWords() {
        for text in MemoryPrivacyAdapter.ownStrings {
            XCTAssertFalse(text.contains("\u{2014}") || text.contains("\u{2013}"), text)
        }
    }

    // MARK: Search

    func testSearchingMemoryFindsTheSectionWithoutChangingTheClassicResults() {
        let classic = SettingsSearch.results(for: "memory").map(\.id)
        let redesigned = SettingsRedesign.results(for: "memory").map(\.id)
        XCTAssertEqual(Array(redesigned.prefix(classic.count)), classic)
        for entry in SettingsSearch.entries where entry.section == .capture {
            XCTAssertTrue(redesigned.contains(entry.id), entry.id)
        }
        // The classic search is the classic function, untouched by the extra entries.
        XCTAssertFalse(classic.contains(SettingsAnchorID.captureScreenCaptures.rawValue))
    }
}
