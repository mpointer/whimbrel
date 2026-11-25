import AppKit
import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the redrawn Sync & Devices, Connectors and Automations sections, the last slice: the anchors, the
/// device rows, the shown-once values, the scope and client rows, the routine rules and the words. Nothing here opens a
/// window, reads the app's defaults or depends on the machine's language, time zone or date.
final class SettingsRedesignSyncConnectorsTests: XCTestCase {
    // MARK: Anchors and sections

    func testEachSectionDeclaresExactlyItsRegistryIds() {
        for (section, declared) in [(SettingsSection.devices, SettingsAnchorID.sync.all), (.connectors, SettingsAnchorID.connectors.all)] {
            let registry = SettingsSearch.entries.filter { $0.section == section }.map(\.id)
            XCTAssertEqual(Set(declared), Set(registry), "\(section)")
            XCTAssertEqual(declared.count, Set(declared).count, "\(section)")
            XCTAssertEqual(SettingsRedesign.declaredAnchorIDs(for: section), declared)
        }
        XCTAssertEqual(SettingsAnchorID.sync.all.count, 5)
        XCTAssertEqual(SettingsAnchorID.connectors.all.count, 5)
        // Routines have no search entries.
        XCTAssertTrue(SettingsSearch.entries.filter { $0.section == .automations }.isEmpty)
        XCTAssertEqual(SettingsRedesign.declaredAnchorIDs(for: .automations), [])
    }

    func testNoSectionStillHostsAClassicPaneExceptDiagnostics() {
        for section in SettingsSection.allCases {
            XCTAssertEqual(SettingsRedesign.hostsClassicPanes(section), section == .diagnostics, "\(section)")
        }
    }

    func testEverySearchResultOfTheThreeSectionsOpensItsSection() {
        for entry in SettingsSearch.entries where [.devices, .connectors, .automations].contains(entry.section) {
            XCTAssertEqual(SettingsRedesign.section(forResult: entry.id), entry.section, entry.id)
        }
    }

    // MARK: Sync state and devices

    func testEverySyncStatusMapsToAPhase() {
        XCTAssertEqual(SyncDevicesAdapter.phase(.notSignedIn), .signedOut)
        XCTAssertEqual(SyncDevicesAdapter.phase(.signingIn), .signingIn)
        XCTAssertEqual(SyncDevicesAdapter.phase(.waitingForApproval), .waiting)
        XCTAssertEqual(SyncDevicesAdapter.phase(.waitingForGroupKey), .waiting)
        XCTAssertEqual(SyncDevicesAdapter.phase(.deviceLimit(kind: "computer", holders: [])), .deviceLimit)
        XCTAssertEqual(SyncDevicesAdapter.phase(.ready(tursoToken: "x", tursoDatabaseUrl: "y")), .ready)
        XCTAssertEqual(SyncDevicesAdapter.phase(.error("no")), .failed("no"))
        XCTAssertEqual(SyncDevicesAdapter.phase(.rateLimited), .failed(SyncDevicesAdapter.rateLimitedMessage))
    }

    private func record(_ id: String, key: String, status: String, platform: String? = "macos", name: String? = nil) -> SyncDeviceRecord {
        SyncDeviceRecord(id: id, accountId: "a", publicKey: key, name: name, platform: platform, status: status, enrolledAtUnixMs: 0, revokedAtUnixMs: nil)
    }

    func testDeviceRowsFlagTwoPendingDevicesThatShowTheSameCode() {
        let devices = [
            record("1", key: "k1", status: "pending", name: "Mac mini"), record("2", key: "k2", status: "pending", name: "Other"),
            record("3", key: "k3", status: "pending", name: "Third"), record("4", key: "k4", status: "approved", name: "Home"),
        ]
        let rows = SyncDevicesAdapter.rows(
            devices: devices, thisDeviceKey: "k4", runnerLastRanMs: nil, code: { ["k1": "111", "k2": "111", "k3": "222", "k4": "111"][$0] }, lastRan: { _ in "" })
        XCTAssertEqual(rows.map(\.collides), [true, true, false, false])
        XCTAssertEqual(rows[0].code, .value("111"))
        XCTAssertFalse(rows[0].approveIsEnabled)
        XCTAssertTrue(rows[2].approveIsEnabled)
        XCTAssertEqual(rows[2].approveKind, .secondary)
        XCTAssertFalse(rows[3].canApprove)
        XCTAssertFalse(rows[3].canResend, "this device's own key matches")
    }

    func testAPendingDeviceWithNoCodeCannotBeVerifiedAndTheRunnerCanOnlyBeRemoved() {
        let devices = [record("1", key: "k1", status: "pending", name: "Mac"), record("2", key: "k2", status: "approved", platform: "cloud")]
        let rows = SyncDevicesAdapter.rows(devices: devices, thisDeviceKey: "k0", runnerLastRanMs: 1_000, code: { _ in nil }, lastRan: { "ran \($0)" })
        XCTAssertEqual(rows[0].code, .unverifiable)
        XCTAssertEqual(rows[1].kind, .runner)
        XCTAssertEqual(rows[1].name, CloudRoutineCopy.runnerName)
        XCTAssertTrue(rows[1].canRemoveRunner)
        XCTAssertFalse(rows[1].canRevoke)
        XCTAssertEqual(rows[1].lastRan, "ran 1000")
        XCTAssertNil(rows[0].lastRan)
    }

    func testAnUnnamedAndroidDeviceKeepsItsClassicName() {
        let rows = SyncDevicesAdapter.rows(devices: [record("1", key: "k", status: "approved", platform: "android")], thisDeviceKey: nil, runnerLastRanMs: nil, code: { _ in nil }, lastRan: { _ in "" })
        XCTAssertEqual(rows[0].name, String(localized: "Android phone"))
    }

    func testOnlyTheProvisionBundleIsASecretValue() {
        let values = SyncDevicesAdapter.provisionValues(RunnerProvision(provisionBundle: "bundle_example", approverPublicKey: "pub_example"), workerURL: "https://example.com")
        XCTAssertEqual(values.filter(\.isSecret).map(\.name), ["RUNNER_PROVISION"])
        XCTAssertEqual(values.count, 3)
    }

    // MARK: Shown once: nothing survives the close

    func testAShownTokenIsGoneFromItsStateOnceTheSheetCloses() {
        let secret = "whm_mcp_example_not_a_real_token"
        var state = WShownOnceState(values: ConnectorsAdapter.shownToken(secret))
        XCTAssertTrue(state.needsAskBeforeClosing)
        state.closed()
        XCTAssertTrue(state.values.isEmpty)
        XCTAssertFalse(String(reflecting: state).contains(secret))
    }

    func testNoConnectorsModelHasAFieldForASecret() {
        // The own client is an id and a flag; the secret has nowhere to live.
        let own = WOwnClientModel(isSaved: true, clientId: "id")
        XCTAssertEqual(Set(Mirror(reflecting: own).children.compactMap(\.label)), ["isSaved", "clientId", "error"])
        let row = WMcpClientRow(id: "1", name: "n", detail: "d")
        XCTAssertEqual(Set(Mirror(reflecting: row).children.compactMap(\.label)), ["id", "name", "detail", "revokedText"])
    }

    // MARK: Connectors

    func testTheAddFormOffersTheClassicScopesAndNotWrite() {
        let rows = ConnectorsAdapter.scopeRows(selected: ["memory"])
        XCTAssertEqual(rows.map(\.id), McpClientScopes.all)
        XCTAssertFalse(rows.map(\.id).contains(McpClientScopes.write))
        XCTAssertEqual(rows.filter(\.isOn).map(\.id), ["memory"])
        XCTAssertNotNil(rows.first { $0.id == McpClientScopes.googleData }?.detail)
    }

    func testAClientHoldingWriteIsListedInWordsAndARevokedOneCarriesItsDate() {
        let row = ConnectorsAdapter.clientRow(id: 7, name: "Desk", scopes: ["memory", "write"], createdMs: 1, revokedMs: 2, date: { "d\($0)" })
        XCTAssertEqual(row.id, "7")
        XCTAssertTrue(row.detail.contains(McpClientScopes.label("write")))
        XCTAssertTrue(row.detail.contains(McpClientScopes.label("memory")))
        XCTAssertNotNil(row.revokedText)
        XCTAssertNil(ConnectorsAdapter.clientRow(id: 8, name: "x", scopes: ["memory"], createdMs: 1, revokedMs: nil, date: { "\($0)" }).revokedText)
    }

    func testRemovingAGoogleAccountAsksWithTheClassicWords() {
        let ask = ConnectorsAdapter.removeQuestion(service: .gmail, revokes: false)
        XCTAssertEqual(ask.title, GoogleDisconnect.confirmationTitle)
        XCTAssertEqual(ask.message, GoogleDisconnect.confirmationMessage(for: .gmail, revokes: false))
    }

    func testTheClientIdAndSecretNeedBothAndSavingWhatIsSavedDoesNothing() {
        let saved = GoogleOAuthClient(clientId: "id", clientSecret: "s", isCustom: true)
        XCTAssertFalse(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "", saved: nil))
        XCTAssertTrue(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "s", saved: nil))
        XCTAssertFalse(ConnectorsSettingsPane.canSaveCustomClient(id: "id", secret: "s", saved: saved))
    }

    // MARK: Shown-once secrets reach the clipboard concealed

    /// Written to a private named pasteboard, never the general one, so the test cannot touch the owner's clipboard.
    func testACopiedSecretCarriesTheConcealedMarkAndTheText() {
        XCTAssertEqual(SettingsPasteboard.concealedType.rawValue, "org.nspasteboard.ConcealedType")
        let board = NSPasteboard(name: NSPasteboard.Name("whimbrel.test.\(UUID().uuidString)"))
        defer { board.releaseGlobally() }
        XCTAssertTrue(SettingsPasteboard.copy("whm_mcp_example_not_a_real_token", to: board))
        XCTAssertEqual(board.string(forType: .string), "whm_mcp_example_not_a_real_token")
        XCTAssertNotNil(board.data(forType: SettingsPasteboard.concealedType))
        XCTAssertTrue(board.types?.contains(SettingsPasteboard.concealedType) ?? false)
    }

    // MARK: Automations

    func testOnlyTheWriteCapableRoutinesOfferAutoApprove() {
        XCTAssertTrue(AutomationsAdapter.performsWriteActions(template: "eod_recap"))
        XCTAssertTrue(AutomationsAdapter.performsWriteActions(template: "focus_time"))
        XCTAssertTrue(AutomationsAdapter.performsWriteActions(template: "weekly_digest"))
        XCTAssertFalse(AutomationsAdapter.performsWriteActions(template: "daily_brief"))
        XCTAssertFalse(AutomationsAdapter.performsWriteActions(template: "unknown"))
    }

    func testTheReadyByTimeRoundTripsThroughADate() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let day = Date(timeIntervalSince1970: 1_800_000_000)
        let date = AutomationsAdapter.readyByDate("08:05", day: day, calendar: calendar)
        XCTAssertEqual(AutomationsAdapter.hhmm(from: date, calendar: calendar), "08:05")
    }

    func testClassicArrowsAndDashesReadAsPlainSentences() {
        XCTAssertEqual(AutomationsAdapter.plain("Add it in Settings \u{2192} Devices."), "Add it in Settings, Devices.")
        XCTAssertEqual(AutomationsAdapter.plain("one \u{2014} two"), "one, two")
    }

    func testTheCloudNoteIsNotSaidTwiceWhenTheSwitchIsOffAndBlocked() {
        let reason = AutomationsAdapter.blockedReason(syncReady: false, routineEnabled: true, localOnly: false, hasSyncedKey: true)
        XCTAssertNotNil(reason)
        XCTAssertNil(AutomationsAdapter.statusNote(on: false, blockedReason: reason, runnerMissing: true, template: "daily_brief"))
        XCTAssertEqual(AutomationsAdapter.statusNote(on: true, blockedReason: reason, runnerMissing: true, template: "daily_brief"), reason)
    }

    func testTheRunsOnChoicesAreAnyDeviceThenApprovedNonRunnerDevices() {
        let devices = [record("1", key: "k1", status: "approved"), record("2", key: "k2", status: "pending"), record("3", key: "k3", status: "approved", platform: "cloud")]
        let options = AutomationsAdapter.runsOnOptions(devices: devices, label: { "device \($0)" })
        XCTAssertEqual(options.map(\.id), ["", "k1"])
    }

    // MARK: Words

    func testNoRedesignedSettingsStringSaysMemoryDatabaseOrUsesADash() {
        let setup = KeySetupStrings.library
        let all = SyncDevicesAdapter.ownStrings + ConnectorsAdapter.ownStrings() + AutomationsAdapter.ownStrings + MemoryPrivacyAdapter.ownStrings
            + LibraryCopyRedesign.all + [setup.intro, setup.wordsAreTheKey, setup.lostBoth, setup.sameDiskWarning, setup.encryptContinue]
        for text in all {
            XCTAssertFalse(text.lowercased().contains("memory database"), text)
            XCTAssertFalse(text.contains("\u{2014}") || text.contains("\u{2013}"), text)
        }
    }

    func testTheKeySetupSheetSaysYourLibraryAndTheClassicWordsAreKept() {
        XCTAssertNotEqual(KeySetupStrings.library.intro, KeySetupStrings.classic.intro)
        XCTAssertEqual(KeySetupStrings.classic.intro, EncryptionCopy.setupIntro)
        XCTAssertEqual(KeySetupStrings.library.continueTitle(EncryptionCopy.encryptContinue), KeySetupStrings.library.encryptContinue)
        XCTAssertEqual(KeySetupStrings.library.continueTitle(EncryptionCopy.continueDefault), EncryptionCopy.continueDefault)
    }

    func testEveryAttentionSentenceReadsAsYourLibrary() {
        let attention = [
            EncryptionCopy.attentionNeedsKey, EncryptionCopy.attentionKeyRejected, EncryptionCopy.attentionKeyUnavailable,
            EncryptionCopy.attentionMigrationInterrupted, EncryptionCopy.attentionDamaged, EncryptionCopy.attentionStartupFailed,
            EncryptionCopy.attentionUnexpected(42), EncryptionCopy.attentionUnexpected(-7),
        ]
        for text in attention { XCTAssertFalse(LibraryCopyRedesign.map(text).lowercased().contains("memory database"), text) }
        XCTAssertEqual(LibraryCopyRedesign.unexpectedCode(in: EncryptionCopy.attentionUnexpected(42)), 42)
        XCTAssertEqual(LibraryCopyRedesign.unexpectedCode(in: EncryptionCopy.attentionUnexpected(-7)), -7)
        XCTAssertNil(LibraryCopyRedesign.unexpectedCode(in: "something else"))
        XCTAssertEqual(LibraryCopyRedesign.map(EncryptionCopy.attentionKeyUnavailable), EncryptionCopy.attentionKeyUnavailable)
        XCTAssertEqual(LibraryCopyRedesign.map(EncryptionCopy.attentionUnexpected(42)), LibraryCopyRedesign.unexpected(42))
    }
}
