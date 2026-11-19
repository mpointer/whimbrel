import XCTest
@testable import Whimbrel

/// The launch window's presenter (X4 Task 11b-2): the coordinator's `present` and
/// `runOnboarding` calls become one observable stage, the user's buttons become answers, a
/// restored key learns whether it opened the database from what the coordinator does next,
/// and no screen shows diagnostic text or offers to start fresh.
@MainActor
final class LaunchPresenterTests: XCTestCase {
    static let words = (1...24).map { "w\($0)" }

    private func setupActions(_ seen: @escaping (Data?) -> Void = { _ in }) -> (Data?) -> EncryptionSetupModel.Actions {
        { existing in
            seen(existing)
            return .init(minimumLength: 12, passphraseLength: { $0.count },
                         begin: { _ in .success(CoreSetupMaterial(key: existing ?? Data(repeating: 9, count: 32),
                                                                  recoveryFile: "FILE", words: Self.words)) },
                         verify: { _, p in p == "correct horse battery" },
                         saveFile: { _ in .saved(path: "/Volumes/USB/x", sameDiskAsDatabase: false) },
                         printWords: { _ in .printed }, reusesKey: existing != nil)
        }
    }

    private func presenter(restoreKey: Data = Data(repeating: 1, count: 32),
                           seenSetup: @escaping (Data?) -> Void = { _ in }) -> LaunchUIPresenter {
        LaunchUIPresenter(setupActions: setupActions(seenSetup)) { useKey in
            .init(chooseFile: { .cancelled },
                  restoreFromFile: { _, _ in .failure(.malformed) },
                  restoreFromWords: { _ in .success(restoreKey) },
                  useKey: useKey)
        }
    }

    /// Waits (bounded) for the main actor's other tasks to reach `condition`.
    private func eventually(_ what: String, _ condition: () -> Bool) async {
        for _ in 0..<2000 {
            if condition() { return }
            try? await Task.sleep(nanoseconds: 1_000_000)
        }
        XCTFail("timed out waiting for \(what)")
    }

    func testAScreenWaitsForTheUsersAnswer() async {
        let p = presenter()
        XCTAssertFalse(p.needsWindow)
        let answer = Task { await p.present(.migrationInterrupted) }
        await eventually("the screen") { p.waitingForUser }
        XCTAssertTrue(p.needsWindow)
        guard case .screen(.migrationInterrupted) = p.stage else { return XCTFail("\(p.stage)") }
        p.respond(.retry)
        let r = await answer.value
        XCTAssertEqual(r, .retry)
        XCTAssertTrue(p.busy, "buttons stay disabled until the coordinator moves on")
        XCTAssertFalse(p.waitingForUser)
        p.respond(.quit) // nothing waits: ignored
    }

    func testAWrongRestoredKeyIsReportedAndTheRightOneOpens() async throws {
        let p = presenter()
        let first = Task { await p.present(.recovery(.needsKey)) }
        await eventually("the recovery screen") { p.waitingForUser }
        guard case let .recovery(model) = p.stage else { return XCTFail("\(p.stage)") }
        XCTAssertEqual(model.reason, .keyMissing)
        model.method = .words
        model.words = "some words"
        let attempt = Task { await model.restore() }
        let r1 = await first.value
        XCTAssertEqual(r1, .restoredKey(Data(repeating: 1, count: 32)))
        // The coordinator tried it and the database refused it: the same screen again.
        let second = Task { await p.present(.recovery(.keyRejected)) }
        await attempt.value
        XCTAssertEqual(model.error, EncryptionCopy.keyForAnotherDatabase)
        guard case let .recovery(same) = p.stage else { return XCTFail("\(p.stage)") }
        XCTAssertTrue(same === model, "the screen the user typed into stays")
        // Second try: the coordinator opens the database.
        model.words = "the right words"
        let again = Task { await model.restore() }
        let r2 = await second.value
        XCTAssertEqual(r2, .restoredKey(Data(repeating: 1, count: 32)))
        p.launchFinished(opened: true)
        await again.value
        XCTAssertTrue(model.restored)
        XCTAssertNil(model.error)
    }

    func testARestoredKeyFollowedByAnotherScreenIsAFailureOnTheRecoveryScreen() async {
        let p = presenter()
        let first = Task { await p.present(.recovery(.keyRejected)) }
        await eventually("the recovery screen") { p.waitingForUser }
        guard case let .recovery(model) = p.stage else { return XCTFail("\(p.stage)") }
        model.method = .words
        model.words = "w"
        let attempt = Task { await model.restore() }
        _ = await first.value
        let next = Task { await p.present(.error(.databaseMissingForRecoveredKey)) }
        await attempt.value
        XCTAssertEqual(model.error, EncryptionCopy.restoreFailed)
        guard case .screen(.error(.databaseMissingForRecoveredKey)) = p.stage else { return XCTFail("\(p.stage)") }
        p.respond(.quit)
        _ = await next.value
    }

    func testKeyUnavailableNeverShowsTheDiagnosticAndRetryReportsTheResult() async {
        let p = presenter()
        let first = Task { await p.present(.keyUnavailable(LaunchDiagnostic(diagnosticOnly: "SECRET status -25308"))) }
        await eventually("the screen") { p.waitingForUser }
        guard case let .keyUnavailable(model) = p.stage else { return XCTFail("\(p.stage)") }
        XCTAssertEqual(model.detail, "", "diagnosticOnly is never rendered")
        let retry = Task { await model.retry() }
        let a1 = await first.value
        XCTAssertEqual(a1, .retry)
        // Still unreadable: the coordinator shows the same screen.
        let second = Task { await p.present(.keyUnavailable(LaunchDiagnostic(diagnosticOnly: "SECRET"))) }
        await retry.value
        XCTAssertTrue(model.stillUnavailable)
        let retry2 = Task { await model.retry() }
        let a2 = await second.value
        XCTAssertEqual(a2, .retry)
        p.launchFinished(opened: true)
        await retry2.value
        XCTAssertFalse(model.stillUnavailable)
    }

    func testOnboardingReturnsTheSetupKeyAndWrapsAnExistingKey() async throws {
        var seen: [Data?] = []
        let pending = Data(repeating: 4, count: 32)
        let p = presenter(seenSetup: { seen.append($0) })
        let result = Task { await p.runOnboarding(existingKey: pending) }
        await eventually("the setup") { p.waitingForUser }
        guard case let .onboarding(model) = p.stage else { return XCTFail("\(p.stage)") }
        XCTAssertEqual(seen, [pending], "the setup wraps the pending key: same words")
        model.passphrase = "correct horse battery"
        model.confirmation = "correct horse battery"
        await model.createRecoveryKey()
        await model.saveFile()
        model.testPassphrase = "correct horse battery"
        await model.runTest()
        model.savedConfirmed = true
        await model.perform(.continueToEncrypt)
        let key = await result.value
        XCTAssertEqual(key, pending)
        XCTAssertFalse(p.waitingForUser)
    }

    /// Task 13c item 1: after Delete everything (the one-shot marker is there) the first
    /// onboarding screen carries the line, and showing it consumes the marker. Without the
    /// marker (a normal new install) there is no line.
    func testTheFirstOnboardingScreenAfterDeleteEverythingSaysTheLibraryIsNew() async throws {
        let name = "launch-presenter-13c.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }

        let fresh = presenter()
        fresh.consumeFreshLibraryMarker = { FreshLibraryMarker.consume(defaults) }
        let first = Task { await fresh.runOnboarding(existingKey: nil) }
        await eventually("the setup") { fresh.waitingForUser }
        XCTAssertNil(fresh.onboardingNotice, "a normal new install shows no line")
        fresh.quit()
        _ = await first.value

        FreshLibraryMarker.mark(defaults)
        let p = presenter()
        p.consumeFreshLibraryMarker = { FreshLibraryMarker.consume(defaults) }
        let result = Task { await p.runOnboarding(existingKey: nil) }
        await eventually("the setup") { p.waitingForUser }
        XCTAssertEqual(p.onboardingNotice, EncryptionCopy.deletedLibraryLine)
        XCTAssertFalse(FreshLibraryMarker.consume(defaults), "consumed when the screen showed")
        p.quit()
        _ = await result.value
    }

    /// A launch that ends without onboarding (keys that could not be deleted opened a new
    /// store) still consumes the marker, so the line can never surface later.
    func testLaunchFinishedConsumesTheFreshLibraryMarker() throws {
        let name = "launch-presenter-13c.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        FreshLibraryMarker.mark(defaults)
        let p = presenter()
        p.consumeFreshLibraryMarker = { FreshLibraryMarker.consume(defaults) }
        p.launchFinished(opened: true)
        XCTAssertFalse(FreshLibraryMarker.consume(defaults))
        XCTAssertNil(p.onboardingNotice)
    }

    func testQuitDuringOnboardingReturnsNothing() async {
        let p = presenter()
        let result = Task { await p.runOnboarding(existingKey: nil) }
        await eventually("the setup") { p.waitingForUser }
        p.quit()
        let key = await result.value
        XCTAssertNil(key)
    }

    func testMigrationProgressFromAnyThreadNeverGoesBack() async {
        let p = presenter()
        let progress = { (overall: Double) in
            CoreEncryptionMigration.Progress(step: 1, stepFraction: 0, overall: overall, note: nil)
        }
        await Task.detached { p.migrationProgress(progress(0.4)) }.value
        await eventually("progress") { if case .migrating(0.4) = p.stage { return true } else { return false } }
        XCTAssertTrue(p.needsWindow)
        await Task.detached { p.migrationProgress(progress(0.2)) }.value
        await Task.detached { p.migrationProgress(progress(0.7)) }.value
        await eventually("progress") { if case .migrating(0.7) = p.stage { return true } else { return false } }
    }

    // MARK: Screen content

    private static let allScreens: [LaunchScreen] = [
        .keyStorageUnavailable,
        .keyUnavailable(LaunchDiagnostic(diagnosticOnly: "SECRET-DIAG")),
        .recovery(.needsKey), .recovery(.keyRejected),
        .migrationInterrupted,
        .damaged(.damaged), .damaged(.noAccess),
        .encryptedFileMissing(backupPresent: true, encryptedCopyPresent: false, lastAction: nil),
        .encryptedFileMissing(backupPresent: true, encryptedCopyPresent: true,
                              lastAction: .restore(CoreRestoreResult(action: .partlyDone, movedAside: 2, warning: .none))),
        .encryptedFileMissing(backupPresent: false, encryptedCopyPresent: true, lastAction: nil),
        .encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false, lastAction: nil),
        .error(.filesUnreadable), .error(.decisionUnavailable), .error(.tooManyDecisions), .error(.unknownStep(code: 99)),
        .error(.stepOutOfOrder(code: 3)), .error(.plainOpenFailed), .error(.keyedOpenFailed(.damaged("SECRET-CORE"))),
        .error(.keyedOpenFailed(.startupFailed("SECRET-CORE"))), .error(.keyedOpenFailed(.unsupported("SECRET-CORE"))),
        .error(.migrationCallFailed), .error(.onboardingKeyMismatch), .error(.freshKeyRefused), .error(.keyMissing(.dbKey)),
        .error(.databaseMissingForRecoveredKey),
    ]

    func testEveryScreensButtonsAreTheOnesItOffers() {
        for screen in Self.allScreens {
            let c = LaunchScreenContent.of(screen)
            XCTAssertTrue(screen.offers(.quit), "\(screen)")
            if case .recovery = screen { continue } // its own view: restore only
            if case .keyUnavailable = screen { continue }
            XCTAssertEqual(c.retry, screen.offers(.retry), "\(screen)")
            XCTAssertEqual(c.restoreCopy, screen.offers(.restoreRollbackCopy), "\(screen)")
            XCTAssertEqual(c.reset, screen.offers(.resetEncryptionStatus), "\(screen)")
            XCTAssertEqual(c.continueWithoutStore, screen.offers(.continueWithoutStore), "\(screen)")
        }
    }

    /// Only a failed PLAIN open may continue without the memory database; every keyed,
    /// key or damage problem stays blocking.
    func testOnlyAFailedPlainOpenOffersToContinueWithoutTheDatabase() {
        for screen in Self.allScreens {
            XCTAssertEqual(screen.offers(.continueWithoutStore), screen == .error(.plainOpenFailed), "\(screen)")
        }
        XCTAssertTrue(LaunchScreenContent.of(.error(.plainOpenFailed)).continueWithoutStore)
    }

    func testNoScreenShowsRawTextOrOffersToStartFresh() {
        for screen in Self.allScreens {
            let c = LaunchScreenContent.of(screen)
            let text = ([c.title] + c.paragraphs + c.hints + [c.lastAction].compactMap { $0 }).joined(separator: " ")
            XCTAssertFalse(text.contains("SECRET"), "\(screen): \(text)")
            let lower = text.lowercased()
            for banned in ["start fresh", "start over", "new database", "create a new", "delete the database"] {
                XCTAssertFalse(lower.contains(banned), "\(banned) in \(screen)")
            }
            // Starting with an empty memory database is said only where the user can choose
            // it, behind a confirmation that says it again.
            if lower.contains("empty") {
                XCTAssertTrue(c.reset, "\(screen): 'empty' without the reset it describes")
                XCTAssertTrue(c.resetConfirmBody?.lowercased().contains("empty memory database") == true, "\(screen)")
            }
            XCTAssertFalse(c.title.isEmpty)
            XCTAssertFalse(c.paragraphs.isEmpty, "\(screen)")
        }
    }

    func testThePartlyDoneRestoreIsShownOnTheMissingFileScreen() {
        let c = LaunchScreenContent.of(.encryptedFileMissing(
            backupPresent: true, encryptedCopyPresent: false,
            lastAction: .restore(CoreRestoreResult(action: .partlyDone, movedAside: 1, warning: .none))))
        XCTAssertEqual(c.lastAction, EncryptionCopy.restorePartlyDone)
        XCTAssertTrue(c.restoreCopy)
        // Neither copy: the support hint, and Reset behind a confirmation that says it starts empty.
        let none = LaunchScreenContent.of(.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false, lastAction: nil))
        XCTAssertEqual(none.paragraphs, [EncryptionCopy.missingNoCopyResetBody])
        // Its own hint: not "leave the folder as it is" beside a button that changes it.
        XCTAssertEqual(none.hints, [EncryptionCopy.missingNoCopyHint])
        XCTAssertFalse(EncryptionCopy.missingNoCopyHint.contains("as it is"))
        XCTAssertTrue(EncryptionCopy.missingNoCopyHint.contains("quit Whimbrel, put the backup back, and open Whimbrel again"))
        // "Starts with an empty memory database" is said once, in the confirmation.
        XCTAssertFalse(EncryptionCopy.missingNoCopyHint.contains("empty"))
        XCTAssertFalse(EncryptionCopy.missingNoCopyResetBody.contains("empty"))
        // The confirmation's way back matches the screen's buttons (Quit and Reset only).
        XCTAssertTrue(EncryptionCopy.resetConfirmBody.contains("quit Whimbrel, put it back, and open Whimbrel again"))
        XCTAssertFalse(EncryptionCopy.resetConfirmBody.contains("try again"))
        XCTAssertTrue(none.reset)
        XCTAssertFalse(none.retry || none.restoreCopy)
        XCTAssertEqual(none.resetConfirmBody, EncryptionCopy.resetConfirmBody)
        XCTAssertTrue(EncryptionCopy.resetConfirmBody.contains("empty memory database"))
        XCTAssertTrue(EncryptionCopy.missingNoCopyResetBody.contains("nothing is deleted"))
        let failed = LaunchScreenContent.of(.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: false,
                                                                  lastAction: .reset(.failed)))
        XCTAssertEqual(failed.lastAction, EncryptionCopy.resetStatusFailed)
        // Only the unfinished encrypted copy: support only, no button changes a file.
        let copyOnly = LaunchScreenContent.of(.encryptedFileMissing(backupPresent: false, encryptedCopyPresent: true, lastAction: nil))
        XCTAssertEqual(copyOnly.hints, [EncryptionCopy.supportNeeded])
        XCTAssertFalse(copyOnly.retry || copyOnly.restoreCopy || copyOnly.reset)
    }

    /// Every support address in the launch and Settings copy is the one Help > Send
    /// Feedback writes to.
    func testTheSupportAddressIsTheAppsOwn() throws {
        let address = try XCTUnwrap(WhimbrelApp.feedbackMailURL().absoluteString.split(separator: "?").first)
            .replacingOccurrences(of: "mailto:", with: "")
        let mentions = EncryptionCopy.all.filter { $0.contains("@") }
        XCTAssertFalse(mentions.isEmpty)
        for s in mentions { XCTAssertTrue(s.contains(address), s) }
    }

    // MARK: Needs attention and quitting the setup

    /// Cmd-Q and the menu bar's Quit go through `AppTerminationGuard`: with recovery
    /// material made for a fresh key, the app does not quit; the setup's confirmation shows.
    func testQuittingTheAppDuringTheSetupAsksFirst() async {
        let p = presenter()
        XCTAssertEqual(AppTerminationGuard.reply(p), .terminateNow, "nothing on screen: quit")
        let result = Task { await p.runOnboarding(existingKey: nil) }
        await eventually("the setup") { p.waitingForUser }
        guard case let .onboarding(m) = p.stage else { return XCTFail("\(p.stage)") }
        XCTAssertEqual(AppTerminationGuard.reply(p), .terminateNow, "no material yet")
        m.passphrase = "correct horse battery"
        m.confirmation = "correct horse battery"
        await m.createRecoveryKey()
        XCTAssertEqual(AppTerminationGuard.reply(p), .terminateCancel)
        XCTAssertTrue(p.confirmingQuit, "the confirmation shows instead")
        XCTAssertEqual(AppTerminationGuard.reply(p), .terminateCancel, "still asking")
        p.quit()
        _ = await result.value
        XCTAssertEqual(AppTerminationGuard.reply(p), .terminateNow, "confirmed: the launch quits")
    }

    func testWaitingForTheUserIsPublishedForTheMenuBarAndWindow() async {
        let p = presenter()
        var seen: [Bool] = []
        let watch = p.$waitingForUser.sink { seen.append($0) }
        let answer = Task { await p.present(.migrationInterrupted) }
        await eventually("the screen") { p.waitingForUser }
        p.respond(.quit)
        _ = await answer.value
        watch.cancel()
        XCTAssertEqual(seen, [false, true, false])
    }

    /// Quitting after the recovery file or words were made asks first: the next launch would
    /// make new words and the saved ones would stop working.
    func testQuittingTheSetupAfterMaterialWasMadeAsksFirst() async {
        let p = presenter()
        let result = Task { await p.runOnboarding(existingKey: nil) }
        await eventually("the setup") { p.waitingForUser }
        guard case let .onboarding(model) = p.stage else { return XCTFail("\(p.stage)") }
        p.requestQuit()
        XCTAssertFalse(p.confirmingQuit, "nothing made yet: quits at once")
        let key = await result.value
        XCTAssertNil(key)

        let p2 = presenter()
        let result2 = Task { await p2.runOnboarding(existingKey: nil) }
        await eventually("the setup") { p2.waitingForUser }
        guard case let .onboarding(m2) = p2.stage else { return XCTFail("\(p2.stage)") }
        m2.passphrase = "correct horse battery"
        m2.confirmation = "correct horse battery"
        await m2.createRecoveryKey()
        await m2.saveFile()
        p2.requestQuit()
        XCTAssertTrue(p2.confirmingQuit)
        XCTAssertTrue(p2.waitingForUser, "still in the setup")
        p2.cancelQuit()
        XCTAssertFalse(p2.confirmingQuit)
        XCTAssertTrue(p2.waitingForUser)
        p2.requestQuit()
        p2.quit()
        let key2 = await result2.value
        XCTAssertNil(key2)
        withExtendedLifetime(model) {}
    }
}
