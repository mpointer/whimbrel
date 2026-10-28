import XCTest

/// Hard-coded colors do not follow a theme. This fails if a source file uses one.
final class ColorTokenUsageTests: XCTestCase {
    private let forbidden = [
        "Color.white", ".foregroundStyle(.white)", ".foregroundColor(.white)", "NSColor.white",
        "whimbrelBorder.opacity(", "whimbrelAccentHighlight.opacity(",
    ]

    func testNoSourceFileUsesARawWhiteOrAnOpacityOfAThemedFillThatNowHasAToken() throws {
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Whimbrel")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil))
        var offenders: [String] = []
        for case let url as URL in files where url.pathExtension == "swift" {
            let text = try String(contentsOf: url, encoding: .utf8)
            for pattern in forbidden where text.contains(pattern) {
                offenders.append("\(url.lastPathComponent): \(pattern)")
            }
            // `Color(hex:` belongs only to the design system's own helper.
            if url.lastPathComponent != "DesignSystem.swift", text.contains("Color(hex:") {
                offenders.append("\(url.lastPathComponent): Color(hex:")
            }
        }
        XCTAssertEqual(offenders, [], "use a theme token instead: \(offenders)")
    }
}
