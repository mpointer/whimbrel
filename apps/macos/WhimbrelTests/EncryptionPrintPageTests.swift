import AppKit
import SwiftUI
import XCTest
@testable import Whimbrel

/// The printed recovery page (X4 Task 11b-2): in every language the app ships, the title, the
/// raw-key notice, the 24 words, the QR and the footer fit on the 540 by 720 page with its
/// margins, so no safety text is clipped. Also renders the page per language for review when
/// `WHIMBREL_ENCRYPTION_SCREENSHOTS` is set.
@MainActor
final class EncryptionPrintPageTests: XCTestCase {
    /// The longest words the 24-word list has (8 letters), so the grid is at its widest.
    static let words = ["abstract", "accident", "acoustic", "activity", "actually", "addition", "adjust", "advance",
                        "aircraft", "alphabet", "although", "aluminum", "american", "ancestor", "anything", "approval",
                        "argument", "artefact", "assault", "athletes", "attitude", "auction", "audience", "bachelor"]
    static let languages = ["en", "fr", "de", "es", "pt"]
    /// The content's room: the page less its margins.
    static let inner = CGSize(width: EncryptionPrintPage.size.width - 2 * EncryptionPrintPage.margin,
                              height: EncryptionPrintPage.size.height - 2 * EncryptionPrintPage.margin)

    private func copy(_ language: String) throws -> EncryptionPrintCopy {
        if language == "en" { return EncryptionPrintCopy() }
        let c = try XCTUnwrap(EncryptionPrintCopy.localized(language), "no \(language) table in the app bundle")
        let english = EncryptionPrintCopy()
        // Really translated, or this test would measure English five times.
        XCTAssertNotEqual(c.notice, english.notice, language)
        XCTAssertNotEqual(c.footer, english.footer, language)
        XCTAssertNotEqual(c.title, english.title, language)
        return c
    }

    private func page(_ language: String, _ c: EncryptionPrintCopy) -> some View {
        EncryptionPrintPage(words: Self.words, qr: EncryptionQR.image(for: Self.words.joined(separator: " ")), copy: c)
            .environment(\.locale, Locale(identifier: language))
    }

    /// The content's natural height at the page's inner width (not the fixed page frame,
    /// which is always 720).
    static func contentHeight(_ c: EncryptionPrintCopy, language: String) -> CGFloat {
        let content = EncryptionPrintPageContent(words: words, qr: EncryptionQR.image(for: words.joined(separator: " ")), copy: c)
            .environment(\.locale, Locale(identifier: language))
            .frame(width: inner.width)
            .environment(\.colorScheme, .light)
        let host = NSHostingView(rootView: content)
        host.frame = CGRect(origin: .zero, size: CGSize(width: inner.width, height: 2000))
        host.layoutSubtreeIfNeeded()
        return host.fittingSize.height
    }

    func testTheWholePageFitsInEveryLanguage() throws {
        var heights: [String: CGFloat] = [:]
        for language in Self.languages {
            let h = Self.contentHeight(try copy(language), language: language)
            heights[language] = h
            XCTAssertGreaterThan(h, 400, "\(language): measured something real")
            XCTAssertLessThanOrEqual(h, Self.inner.height, "\(language): the page clips its safety text (\(h) > \(Self.inner.height))")
        }
        print("print page content heights (limit \(Self.inner.height)): \(heights.sorted { $0.key < $1.key })")
        // fr-CA ships its own table (same text as fr today).
        XCTAssertLessThanOrEqual(Self.contentHeight(try XCTUnwrap(EncryptionPrintCopy.localized("fr-CA")), language: "fr-CA"),
                                 Self.inner.height)
    }

    /// The measurement wraps text at the page width: a longer notice measures taller, and one
    /// that cannot fit is caught.
    func testTheMeasurementSeesLongerText() {
        var long = EncryptionPrintCopy()
        let base = Self.contentHeight(long, language: "en")
        long.notice = String(repeating: long.notice + " ", count: 2)
        let longer = Self.contentHeight(long, language: "en")
        XCTAssertGreaterThan(longer, base + 40, "base \(base), longer \(longer)")
        long.footer = String(repeating: long.footer + " ", count: 6)
        XCTAssertGreaterThan(Self.contentHeight(long, language: "en"), Self.inner.height, "a page that cannot fit is caught")
    }

    func testRenderThePrintedPagePerLanguage() throws {
        guard let path = ProcessInfo.processInfo.environment["WHIMBREL_ENCRYPTION_SCREENSHOTS"], !path.isEmpty else {
            throw XCTSkip("Set WHIMBREL_ENCRYPTION_SCREENSHOTS=<folder> to render the printed page")
        }
        let dir = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        for language in Self.languages {
            let rep = try XCTUnwrap(OffscreenRender.bitmap(of: page(language, try copy(language)), size: EncryptionPrintPage.size,
                                                           colorScheme: .light, scale: 2))
            let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
            try png.write(to: dir.appendingPathComponent("print-\(language).png"))
        }
    }
}
