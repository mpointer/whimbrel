import XCTest
@testable import Whimbrel

final class MarkdownAttributedStringTests: XCTestCase {
    func testBoldTextRendersWithoutLiteralAsterisks() {
        let result = TimelineView.markdownAttributedString("This is **bold** text")
        XCTAssertFalse(String(result.characters).contains("**"), "literal ** must not survive Markdown parsing")
        XCTAssertTrue(String(result.characters).contains("bold"))
    }

    func testBulletListRendersAsBulletsNotLiteralAsterisks() {
        let result = TimelineView.markdownAttributedString("- item one\n- item two")
        let plain = String(result.characters)
        XCTAssertFalse(plain.hasPrefix("- item"), "a real list item must not stay as literal '- item' text")
    }

    func testPlainTextWithNoMarkdownPassesThroughUnchanged() {
        let result = TimelineView.markdownAttributedString("just plain text, no formatting")
        XCTAssertEqual(String(result.characters), "just plain text, no formatting")
    }

    func testMalformedMarkdownStillReturnsReadableText() {
        // Unbalanced emphasis markers -- must not crash or drop the text entirely.
        let result = TimelineView.markdownAttributedString("this has an *unclosed emphasis marker")
        XCTAssertTrue(String(result.characters).contains("unclosed emphasis marker"))
    }
}
