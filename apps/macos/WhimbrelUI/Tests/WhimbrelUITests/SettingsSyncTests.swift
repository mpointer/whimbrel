import Foundation
import Testing
@testable import WhimbrelUI

@Suite struct SettingsSyncTests {
    // MARK: Sync state

    @Test func everySyncPhaseHasOneWordAndATone() {
        let l = WSyncLabels()
        #expect(WSyncStatusRule.line(.signedOut, labels: l) == WSyncStatusLine(text: "Off", tone: .neutral))
        #expect(WSyncStatusRule.line(.ready, labels: l).tone == .good)
        #expect(WSyncStatusRule.line(.waiting, labels: l).tone == .warning)
        #expect(WSyncStatusRule.line(.deviceLimit, labels: l).tone == .warning)
        #expect(WSyncStatusRule.line(.failed("x"), labels: l).tone == .problem)
        #expect(WSyncStatusRule.announcement(.ready, labels: l) == "Sync: On")
    }

    // MARK: Devices

    private func row(
        status: String = "approved", runner: Bool = false, differs: Bool = false, code: String? = nil, collides: Bool = false, removable: Bool = false
    ) -> WDeviceRow {
        WDeviceRow.make(
            id: "d", name: "Mac", status: status, statusText: status, isRunner: runner, keyDiffersFromThisDevice: differs, code: code,
            collides: collides, canRemoveRunner: removable)
    }

    @Test func aPendingDeviceShowsItsCodeAndCanBeApprovedAndRevoked() {
        let d = row(status: "pending", code: "123 456")
        #expect(d.code == .value("123 456"))
        #expect(d.canApprove && d.approveIsEnabled && d.approveKind == .secondary)
        #expect(d.canRevoke && !d.canResend && !d.canRemoveRunner)
    }

    @Test func aPendingDeviceWithNoCodeSaysItCannotBeVerified() {
        #expect(row(status: "pending", code: nil).code == .unverifiable)
        #expect(row(status: "approved", code: "999").code == .none)
    }

    @Test func approveIsOffAndSaysWhyWhileACodeCollides() {
        let d = row(status: "pending", code: "123 456", collides: true)
        #expect(d.collides)
        #expect(d.canApprove)
        #expect(!d.approveIsEnabled)
        #expect(d.approveKind == .secondary)
        // Revoke stays available: it is what the person should do.
        #expect(d.canRevoke)
    }

    @Test func aCollisionOnADeviceThatIsNotPendingIsIgnored() {
        #expect(!row(status: "approved", collides: true).collides)
    }

    @Test func aDeviceWithAnotherKeyCanBeSentTheKeyAgain() {
        #expect(row(status: "approved", differs: true).canResend)
        #expect(!row(status: "approved", differs: false).canResend)
        #expect(!row(status: "pending", differs: true).canResend)
    }

    @Test func aRevokedDeviceCannotBeRevokedAgain() {
        #expect(!row(status: "revoked").canRevoke)
        #expect(row(status: "resetting").canRevoke)
    }

    @Test func theCloudRunnerCanOnlyBeRemoved() {
        let r = row(status: "approved", runner: true, differs: true, removable: true)
        #expect(r.kind == .runner)
        #expect(r.canRemoveRunner)
        #expect(!r.canRevoke && !r.canApprove && !r.canResend)
        #expect(!row(status: "revoked", runner: true, removable: false).canRemoveRunner)
    }

    // MARK: Shown once

    @Test func copiedShowsOnlyAfterTheWriteSucceeded() {
        var state = WShownOnceState(values: [WShownValue(id: "t", name: "", value: "tok_example")])
        state.copyFinished(id: "t", succeeded: false)
        #expect(state.values[0].copied == false)
        state.copyFinished(id: "t", succeeded: true)
        #expect(state.values[0].copied)
    }

    @Test func closingBeforeCopyingAsksFirst() {
        var state = WShownOnceState(values: [WShownValue(id: "t", name: "", value: "tok_example")])
        #expect(state.needsAskBeforeClosing)
        let first = state.requestClose()
        #expect(first == false)
        #expect(state.asking)
        state.keepOpen()
        #expect(!state.asking)
        state.copyFinished(id: "t", succeeded: true)
        let second = state.requestClose()
        #expect(second)
    }

    @Test func onlyASecretValueMakesClosingAsk() {
        var state = WShownOnceState(values: [WShownValue(id: "u", name: "URL", value: "https://example.com", isSecret: false)])
        #expect(!state.needsAskBeforeClosing)
        let may = state.requestClose()
        #expect(may)
        state = WShownOnceState(values: [
            WShownValue(id: "u", name: "URL", value: "https://example.com", isSecret: false),
            WShownValue(id: "p", name: "Bundle", value: "bundle_example", isSecret: true),
        ])
        state.copyFinished(id: "u", succeeded: true)
        #expect(state.needsAskBeforeClosing)
    }

    @Test func closingDropsEveryValue() {
        let secret = "tok_example_not_real_0123"
        var state = WShownOnceState(values: [WShownValue(id: "t", name: "Token", value: secret)])
        _ = state.requestClose()
        state.closed()
        #expect(state.values.isEmpty)
        #expect(!state.asking)
        #expect(!String(describing: state).contains(secret))
        #expect(!String(reflecting: state).contains(secret))
    }

    // MARK: Time of day

    @Test func aTimeOfDayIsTwoDigitsColonTwoDigitsInRange() {
        #expect(WTimeRule.parse("07:30") == "07:30")
        #expect(WTimeRule.parse(" 23:59 ") == "23:59")
        #expect(WTimeRule.parse("00:00") == "00:00")
        #expect(WTimeRule.parse("7:30") == nil)
        #expect(WTimeRule.parse("24:00") == nil)
        #expect(WTimeRule.parse("12:60") == nil)
        #expect(WTimeRule.parse("12:5") == nil)
        #expect(WTimeRule.parse("") == nil)
        #expect(WTimeRule.parse("ab:cd") == nil)
    }

    // MARK: Anchors

    @Test func theAnchorListsAreTheirSections() {
        let sync = WSyncAnchors(overview: "a", syncNow: "b", name: "c", phone: "d", deleteAccount: "e", tellAFriend: "f")
        #expect(sync.all == ["a", "b", "c", "d", "e", "f"])
        let c = WConnectorsAnchors(calendar: "a", gmail: "b", outlook: "c", ownClient: "d", mcp: "e")
        #expect(c.all == ["a", "b", "c", "d", "e"])
    }

    // MARK: Words

    @Test func theSyncAndConnectorWordsHaveNoEmDashesOrEnDashes() {
        let sync = WSyncLabels()
        let conn = WConnectorsLabels()
        let texts = [
            sync.intro, sync.phoneBlurb, sync.collisionTitle, sync.collisionText, sync.revokeMessage, sync.runnerNotShownAgain, sync.runnerCloseQuestion,
            conn.intro, conn.ownClientNote, conn.mcpIntro, conn.revokeMessage, conn.tokenMessage, conn.tokenCloseQuestion,
        ]
        for t in texts {
            #expect(!t.contains("\u{2014}") && !t.contains("\u{2013}"), Comment(rawValue: t))
        }
    }

    // MARK: Meetings: the consent sentence

    @MainActor @Test func theConsentSentenceIsItsOwnFullRowWhetherTheReminderIsOnOrOff() {
        for on in [true, false] {
            let model = WSettingsWindowGallery.meetings(noticeOn: on)
            let text = model.labels.consentNote
            #expect(!text.isEmpty)
            // Its own row, first, ahead of the switch: not a card description.
            #expect(model.noticeRows.first == .consent(text), Comment(rawValue: "reminder on: \(on)"))
            #expect(model.noticeRows.contains(.toggle))
            #expect(model.noticeRows.contains(.customText) == on)
            #expect(model.noticeRows.filter { if case .consent = $0 { return true } else { return false } }.count == 1)
        }
    }
}
