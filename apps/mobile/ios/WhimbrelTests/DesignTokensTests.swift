import XCTest
@testable import Whimbrel

final class DesignTokensTests: XCTestCase {
    func testFixedColorPicksTheVariantForEachAppearance() {
        let c = WFixedColor(light: 1, dark: 2, highContrastLight: 3, highContrastDark: 4)
        XCTAssertEqual(c.value(dark: false, highContrast: false), 1)
        XCTAssertEqual(c.value(dark: true, highContrast: false), 2)
        XCTAssertEqual(c.value(dark: false, highContrast: true), 3)
        XCTAssertEqual(c.value(dark: true, highContrast: true), 4)
    }

    func testRecordingAndAiInkAreGenerated() {
        XCTAssertNotNil(WFixed.all["recording"])
        XCTAssertNotNil(WFixed.all["onRecording"])
        XCTAssertNotNil(WFixed.all["aiInk"])
        XCTAssertNotNil(WFixed.all["focus"])
    }

    func testTapTargetMeetsTheMinimum() {
        XCTAssertGreaterThanOrEqual(WTokens.Size.target, 44)
    }

    func testNoTypeRoleIsUnderTheMinimumReadableSize() {
        let s = WTokens.TypeScale.self
        let roles = [s.caption, s.secondary, s.body, s.bodyStrong, s.time, s.subtitle, s.title, s.pageTitle, s.reading, s.readingTitle]
        for role in roles {
            XCTAssertGreaterThanOrEqual(role.size, WTokens.TypeScale.minimumReadable)
            XCTAssertGreaterThanOrEqual(role.lineHeight, role.size)
        }
    }
}

@MainActor
final class DesignComponentsTests: XCTestCase {
    func testElapsedClock() {
        XCTAssertEqual(WElapsed.clock(0), "0:00")
        XCTAssertEqual(WElapsed.clock(767), "12:47")
        XCTAssertEqual(WElapsed.clock(3767), "1:02:47")
        XCTAssertEqual(WElapsed.clock(-5), "0:00")
    }

    /// The spoken name carries the state in words, never color alone.
    func testRecordingSpokenLabelNamesTheState() {
        let s = WStrings.english
        XCTAssertTrue(WRecordingIndicator.spokenLabel(state: .recording, elapsedSeconds: 767, strings: s).hasPrefix("Recording, "))
        XCTAssertTrue(WRecordingIndicator.spokenLabel(state: .paused, elapsedSeconds: 767, strings: s).hasPrefix("Paused, "))
        XCTAssertEqual(WRecordingIndicator.spokenLabel(state: .off, elapsedSeconds: 0, strings: s), "Not recording")
        XCTAssertEqual(WRecordingIndicator.spokenLabel(state: .problem("Microphone not found"), elapsedSeconds: 0, strings: s), "Problem, Microphone not found")
    }

    func testEveryRecordingStateHasItsOwnGlyphAndWord() {
        let states: [WRecordingState] = [.recording, .paused, .processing, .off, .problem("x")]
        XCTAssertEqual(Set(states.map(\.glyph)).count, states.count)
        XCTAssertEqual(Set(states.map { $0.word(.english) }).count, states.count)
    }

    func testSpeakerInitials() {
        XCTAssertEqual(WSpeakerChip.initials("Sam Okafor"), "SO")
        XCTAssertEqual(WSpeakerChip.initials("Jo, Northwind"), "JN")
        XCTAssertEqual(WSpeakerChip.initials("You"), "Y")
    }

    /// The consent wording matches desktop and never claims anyone agreed.
    func testConsentWording() {
        XCTAssertEqual(WStrings.english.consentMessage, "You are recording. Tell the others before you go on.")
    }

    #if DEBUG
    func testGalleryThreadThemesHaveEveryTokenAndPassTheContrastGuard() {
        for theme in WGallery.threadThemes {
            XCTAssertEqual(theme.light.count, ThemeToken.allCases.count)
            XCTAssertEqual(theme.dark.count, ThemeToken.allCases.count)
            XCTAssertEqual(ThemeContrast.failures(theme.light), [], theme.id)
            XCTAssertEqual(ThemeContrast.failures(theme.dark), [], theme.id)
        }
    }
    #endif
}
