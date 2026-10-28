import CoreGraphics
import CoreText
import XCTest
@testable import Whimbrel

final class TextRecognizerTests: XCTestCase {
    func testRecognizesRenderedText() throws {
        let width = 800, height = 200
        let ctx = try XCTUnwrap(CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        ctx.setFillColor(CGColor(gray: 1, alpha: 1))
        ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))

        let font = CTFontCreateWithName("Helvetica" as CFString, 48, nil)
        let attrs: [CFString: Any] = [kCTFontAttributeName: font, kCTForegroundColorFromContextAttributeName: true]
        ctx.setFillColor(CGColor(gray: 0, alpha: 1))
        let line = CTLineCreateWithAttributedString(CFAttributedStringCreate(nil, "Quarterly Roadmap" as CFString, attrs as CFDictionary))
        ctx.textPosition = CGPoint(x: 40, y: 80)
        CTLineDraw(line, ctx)

        let image = try XCTUnwrap(ctx.makeImage())
        XCTAssertTrue(TextRecognizer.recognize(image)?.contains("Quarterly Roadmap") == true)
    }
}
