import XCTest
import WhimbrelUI
@testable import Whimbrel

/// The pure parts of the onboarding adapters (`UI/Redesign`): what state a permission shows, which buttons a
/// blocking screen has, and how a skin becomes a `WTheme`. Nothing here touches the app's defaults or launches it.
final class RedesignOnboardingAdapterTests: XCTestCase {
    func testPermissionState() {
        XCTAssertEqual(PermissionRedesign.state(granted: true, systemDenied: true, askedHere: true), .allowed)
        XCTAssertEqual(PermissionRedesign.state(granted: false, systemDenied: false, askedHere: false), .notYet)
        XCTAssertEqual(PermissionRedesign.state(granted: false, systemDenied: true, askedHere: false), .denied)
        XCTAssertEqual(PermissionRedesign.state(granted: false, systemDenied: false, askedHere: true), .denied)
    }

    func testRequiredAndOptionalCoverEveryKindOnce() {
        XCTAssertEqual(Set(PermissionRedesign.required + PermissionRedesign.optional), Set(PermissionKind.allCases))
        XCTAssertEqual(PermissionRedesign.required, PermissionKind.firstRun)
    }

    func testRedesignReasonsHaveNoDashesOrDeveloperTerms() {
        for kind in PermissionKind.allCases {
            let text = PermissionRedesign.reason(kind)
            XCTAssertFalse(text.contains("\u{2014}") || text.contains("\u{2013}"), text)
            XCTAssertFalse(text.contains("FR-"), text)
        }
    }

    func testBlockingScreenButtonsQuitFirstPrimaryLast() {
        let missing = LaunchScreenContent.of(.encryptedFileMissing(backupPresent: true, encryptedCopyPresent: true, lastAction: nil))
        var m = LaunchBlockingRedesignView.model(missing, busy: false, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
        XCTAssertEqual(m.actions.map(\.id), ["quit", "restore"])
        XCTAssertEqual(m.actions.last?.kind, .primary)
        let failed = LaunchScreenContent.of(.error(.plainOpenFailed))
        m = LaunchBlockingRedesignView.model(failed, busy: false, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
        XCTAssertEqual(m.actions.map(\.id), ["quit", "continueWithoutStore", "retry"])
        m = LaunchBlockingRedesignView.model(failed, busy: true, quit: {}, restore: {}, reset: {}, continueWithout: {}, retry: {})
        XCTAssertTrue(m.actions.allSatisfy { !$0.isEnabled })
        XCTAssertEqual(m.actions.last?.title, EncryptionCopy.trying)
    }

    func testThreadSkinKeepsThreadShape() {
        XCTAssertEqual(RedesignBridge.shape(SkinDefaults.whimbrel).cardRadius, SkinDefaults.whimbrel.shape.radius)
        XCTAssertEqual(RedesignBridge.family(.newYork), .serif)
        XCTAssertEqual(RedesignBridge.family(.avenirNext), .named("Avenir Next"))
    }
}
