import AppKit
import XCTest
@testable import Whimbrel

final class MeetingAskFormattingTests: XCTestCase {
    func testPresetMatchesItsNameLooselyButNotLongerQuestions() {
        XCTAssertEqual(AskPreset.matching("Make me sound smart"), .makeMeSoundSmart)
        XCTAssertEqual(AskPreset.matching("  make me sound SMART!  "), .makeMeSoundSmart)
        XCTAssertNil(AskPreset.matching("make me sound smart about pricing"))
        XCTAssertNil(AskPreset.matching("what did we decide?"))
    }

    func testPresetPromptAsksForProfileAndNextContributions() {
        let prompt = AskPreset.makeMeSoundSmart.prompt
        XCTAssertTrue(prompt.contains("profile"))
        XCTAssertTrue(prompt.contains("questions or contributions"))
    }

    func testMarkdownRendersHeadingsListsAndBold() {
        let md = "## Key points\n- **Budget** is approved\n- Ship *Friday*\n1. First\n\nPlain `code` line"
        let text = AskMarkdown.attributed(md)
        XCTAssertEqual(text.string, "Key points\n• Budget is approved\n• Ship Friday\n1. First\n\nPlain code line")
        let boldRange = (text.string as NSString).range(of: "Budget")
        let font = text.attribute(.font, at: boldRange.location, effectiveRange: nil) as? NSFont
        XCTAssertTrue(font?.fontDescriptor.symbolicTraits.contains(.bold) ?? false)
        let heading = text.attribute(.font, at: 0, effectiveRange: nil) as? NSFont
        let body = text.attribute(.font, at: (text.string as NSString).range(of: "Plain").location, effectiveRange: nil) as? NSFont
        XCTAssertGreaterThan(heading?.pointSize ?? 0, body?.pointSize ?? 0)
    }

    func testPlainTextCopyDropsMarkdownSyntax() {
        XCTAssertEqual(AskMarkdown.plainText("**Hi** there\n- a\n# T"), "Hi there\n• a\nT")
    }

    /// Answers can carry text from the meeting, so only web links stay clickable.
    func testOnlyWebLinksStayClickable() {
        let text = AskMarkdown.attributed("[site](https://example.com) [file](file:///etc/passwd) [app](x-whimbrel://do)")
        let ns = text.string as NSString
        XCTAssertNotNil(text.attribute(.link, at: ns.range(of: "site").location, effectiveRange: nil))
        XCTAssertNil(text.attribute(.link, at: ns.range(of: "file").location, effectiveRange: nil))
        XCTAssertNil(text.attribute(.link, at: ns.range(of: "app").location, effectiveRange: nil))
    }
}
