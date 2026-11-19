import Foundation
import SwiftUI
import Testing
@testable import WhimbrelUI

@Suite struct SetupScreensTests {
    private func row(_ state: WPermissionState) -> WPermissionRowModel {
        WPermissionRowModel(id: "x", glyph: "mic.fill", name: "Microphone", reason: "Why", state: state, onAllow: {}, onOpenSettings: {})
    }

    @Test func permissionRowShowsOneButtonChosenByState() {
        let s = WOnboardingStrings.english
        #expect(row(.notYet).buttonTitle(using: s) == "Allow")
        #expect(row(.denied).buttonTitle(using: s) == "Open System Settings")
        #expect(row(.allowed).buttonTitle(using: s) == nil)
        #expect(WPermissionState.notYet.button == .allow)
        #expect(WPermissionState.denied.button == .openSystemSettings)
        #expect(WPermissionState.allowed.button == nil)
        #expect(WPermissionState.allowed.word(using: s) == "Allowed")
        #expect(WPermissionState.notYet.word(using: s) == "Not yet")
        #expect(WPermissionState.denied.word(using: s) == "Denied")
    }

    @Test func spokenAndStepText() {
        let s = WOnboardingStrings.english
        #expect(s.stepOf(2, 3) == "Step 2 of 3")
        #expect(s.wordItem(3, 24, "apple") == "Word 3 of 24, apple")
        #expect(s.allowNamed("Microphone") == "Allow Microphone")
        #expect(s.openSettingsNamed("Microphone") == "Open System Settings for Microphone")
    }

    @Test func wordListColumnsShrinkWithTextSize() {
        #expect(WWordList.columnCount(textScale: 1) == 3)
        #expect(WWordList.columnCount(textScale: 1.5) == 2)
        #expect(WWordList.columnCount(textScale: 2) == 1)
    }

    private func actions(_ n: Int) -> [WScreenAction] {
        (0..<n).map { WScreenAction(id: "a\($0)", title: "t\($0)", kind: $0 == n - 1 ? .primary : .secondary, run: {}) }
    }

    @Test func moreThanThreeActionsGoInAMenuAndThePrimaryStaysLast() {
        for n in 1...3 {
            let split = WActionLayout.split(actions(n))
            #expect(split.menu.isEmpty)
            #expect(split.row.count == n)
        }
        let five = WActionLayout.split(actions(5))
        #expect(five.row.map(\.id) == ["a0", "a3", "a4"])
        #expect(five.menu.map(\.id) == ["a1", "a2"])
        #expect(five.row.last?.kind == .primary)
    }

    @Test func keySetupPrimaryIsCreateThenContinue() {
        var m = WKeySetupModel()
        m.createTitle = "Create"
        m.continueTitle = "Continue"
        m.leaveTitle = "Quit"
        #expect(m.stepIndex == 1)
        #expect(m.actions.map(\.id) == ["leave", "create"])
        #expect(m.actions.last?.isEnabled == false)
        m.canCreate = true
        #expect(m.actions.last?.isEnabled == true)
        m.hasKey = true
        m.canContinue = false
        #expect(m.stepIndex == 2)
        #expect(m.actions.map(\.id) == ["leave", "startOver", "continue"])
        #expect(m.actions.last?.isEnabled == false)
        #expect(m.actions.filter { $0.kind == .primary }.count == 1)
        m.leaveTitle = nil
        #expect(m.actions.map(\.id) == ["startOver", "continue"])
    }

    @Test func keySetupErrorsSitUnderTheirStep() {
        var m = WKeySetupModel()
        m.passphraseProblem = "Too short"
        m.saveFailedLine = "Not saved"
        m.testFailedLine = "Wrong"
        // Before the key exists only the passphrase error can show.
        #expect(m.errorLines.map(\.place) == [.passphrase])
        m.hasKey = true
        #expect(m.errorLines.map(\.place) == [.passphrase, .file, .test])
        #expect(m.errorLines.map(\.text) == ["Too short", "Not saved", "Wrong"])
    }

    @Test func recoveryHasQuitThenOnePrimaryUnlock() {
        var m = WRecoveryModel()
        m.unlockTitle = "Unlock"
        #expect(m.actions.map(\.id) == ["unlock"])
        m.quitTitle = "Quit"
        m.canUnlock = true
        #expect(m.actions.map(\.id) == ["quit", "unlock"])
        #expect(m.actions.last?.kind == .primary)
        #expect(m.actions.last?.isEnabled == true)
    }

    @Test func onboardingEnglishHasNoEmptyStringAndNoDash() {
        let s = WOnboardingStrings.english
        let plain = [s.allowed, s.notYet, s.denied, s.allow, s.openSystemSettings, s.recordingCaption, s.moreOptions,
                     s.allowNamed("A"), s.openSettingsNamed("A"), s.stepOf(1, 2), s.wordItem(1, 24, "w")]
        for text in plain {
            #expect(!text.trimmingCharacters(in: .whitespaces).isEmpty)
            #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}"), "\(text)")
        }
    }

    @Test func sampleScreensHaveNoDashes() {
        // The gallery's sample words are the text the app shows; none may hold an em or en dash.
        let all: [String] = [
            WSampleCopy.firstRunIntro, WSampleCopy.changeLater, WSampleCopy.checklistIntro, WSampleCopy.screenReason,
            WSampleCopy.accessibilityReason, WSampleCopy.calendarReason, WSampleCopy.modelFailedHelp, WSampleCopy.syncMessage,
        ]
        for text in all { #expect(!text.contains("\u{2014}") && !text.contains("\u{2013}"), "\(text)") }
        #expect(WOnboardingGallery.screens().count >= 12)
    }
}
