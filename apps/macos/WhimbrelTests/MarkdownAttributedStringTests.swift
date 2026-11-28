import WhimbrelUI
import XCTest
@testable import Whimbrel

final class MarkdownAttributedStringTests: XCTestCase {
    func testBoldTextRendersWithoutLiteralAsterisks() {
        let result = WAnswerAttributed.markdown("This is **bold** text")
        XCTAssertFalse(String(result.characters).contains("**"), "literal ** must not survive Markdown parsing")
        XCTAssertTrue(String(result.characters).contains("bold"))
    }

    func testBulletListRendersAsBulletsNotLiteralAsterisks() {
        let result = WAnswerAttributed.markdown("- item one\n- item two")
        let plain = String(result.characters)
        XCTAssertFalse(plain.hasPrefix("- item"), "a real list item must not stay as literal '- item' text")
    }

    func testPlainTextWithNoMarkdownPassesThroughUnchanged() {
        let result = WAnswerAttributed.markdown("just plain text, no formatting")
        XCTAssertEqual(String(result.characters), "just plain text, no formatting")
    }

    func testMalformedMarkdownStillReturnsReadableText() {
        // Unbalanced emphasis markers -- must not crash or drop the text entirely.
        let result = WAnswerAttributed.markdown("this has an *unclosed emphasis marker")
        XCTAssertTrue(String(result.characters).contains("unclosed emphasis marker"))
    }
}
