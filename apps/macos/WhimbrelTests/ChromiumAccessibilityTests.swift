import XCTest
@testable import Whimbrel

@MainActor
final class ChromiumAccessibilityTests: XCTestCase {
    private func changes(_ enabled: Set<pid_t> = [], candidate: pid_t? = nil, tracked: pid_t? = nil, mic: Bool = false) -> (enable: Set<pid_t>, disable: Set<pid_t>) {
        ChromiumAccessibility.changes(enabled: enabled, candidate: candidate, tracked: tracked, micInUse: mic)
    }

    func testOrdinaryBrowsingIsNeverTouched() {
        let c = changes(candidate: 7, mic: false)
        XCTAssertEqual(c.enable, [])
        XCTAssertEqual(c.disable, [])
    }

    func testAFrontmostBrowserWithTheMicInUseTurnsOn() {
        XCTAssertEqual(changes(candidate: 7, mic: true).enable, [7])
    }

    func testItStaysOnWhileTheMicIsInUseAfterSwitchingApps() {
        let c = changes([7], candidate: nil, mic: true)
        XCTAssertEqual(c.enable, [])
        XCTAssertEqual(c.disable, [])
    }

    func testItTurnsOffWhenTheMicStops() {
        XCTAssertEqual(changes([7], candidate: 7, mic: false).disable, [7])
    }

    func testATrackedMeetingKeepsItOnWithoutTheMic() {
        XCTAssertEqual(changes([7], tracked: 7, mic: false).disable, [])
    }

    func testTwoBrowsersAreHandledSeparately() {
        let c = changes([7], candidate: 8, tracked: 7, mic: false)
        XCTAssertEqual(c.enable, [])
        XCTAssertEqual(c.disable, [])
        XCTAssertEqual(changes([7], candidate: 8, mic: true).enable, [8])
    }

    func testScreenMemoryLeavesPageContentAloneWhileTheTreeIsOn() {
        XCTAssertTrue(AXTextWalker.skips(role: "AXWebArea", subrole: nil, skipWebAreas: true))
        XCTAssertFalse(AXTextWalker.skips(role: "AXWebArea", subrole: nil, skipWebAreas: false))
        XCTAssertFalse(AXTextWalker.skips(role: "AXStaticText", subrole: nil, skipWebAreas: true))
    }

    func testPasswordFieldsAreAlwaysSkipped() {
        XCTAssertTrue(AXTextWalker.skips(role: "AXTextField", subrole: "AXSecureTextField", skipWebAreas: false))
    }

    func testEveryChromiumBrowserIsChecked() {
        XCTAssertTrue(ChromiumAccessibility.bundleIDs.isSubset(of: BrowserMeetingCheck.knownBrowserBundleIDs))
    }

    func testAddressBarTextBecomesAURLForTheExclusionCheck() {
        XCTAssertEqual(AXTextWalker.addressBarURL(description: "Address and search bar", value: "mail.google.com/mail/u/0"),
                       "https://mail.google.com/mail/u/0")
        XCTAssertEqual(CaptureExclusions.host(of: AXTextWalker.addressBarURL(description: "Address and search bar", value: "bank.example.com")!),
                       "bank.example.com")
        XCTAssertEqual(AXTextWalker.addressBarURL(description: "Address and search bar", value: "https://x.org/a"), "https://x.org/a")
    }

    func testOtherTextFieldsAndSearchTextAreIgnored() {
        XCTAssertNil(AXTextWalker.addressBarURL(description: "Search", value: "example.com"))
        XCTAssertNil(AXTextWalker.addressBarURL(description: "Address and search bar", value: "how to bake bread"))
        XCTAssertNil(AXTextWalker.addressBarURL(description: "Address and search bar", value: ""))
    }
}
