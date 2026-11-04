import AppKit
import SwiftUI

/// Ask answers arrive as Markdown. `AskMarkdown` renders the subset models use (headings,
/// bullet and numbered lists, bold, italic, code, links) for display and for "Copy as plain
/// text"; "Copy" keeps the Markdown itself.
enum AskMarkdown {
    static func attributed(_ markdown: String, color: NSColor = .labelColor, size: CGFloat = NSFont.systemFontSize) -> NSAttributedString {
        let body = NSFont.systemFont(ofSize: size)
        let out = NSMutableAttributedString()
        let lines = markdown.components(separatedBy: "\n")
        for (n, raw) in lines.enumerated() {
            var line = Substring(raw)
            var base = body
            var prefix = ""
            let indent = line.prefix { $0 == " " }.count
            let trimmed = line.drop { $0 == " " }
            if let hashes = heading(trimmed) {
                line = trimmed.dropFirst(hashes + 1)
                base = NSFont.boldSystemFont(ofSize: size + CGFloat(max(1, 5 - hashes)))
            } else if trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") || trimmed.hasPrefix("+ ") {
                line = trimmed.dropFirst(2)
                prefix = String(repeating: "  ", count: indent / 2) + "• "
            }
            out.append(NSAttributedString(string: prefix, attributes: [.font: base, .foregroundColor: color]))
            out.append(inline(String(line), base: base, color: color))
            if n < lines.count - 1 { out.append(NSAttributedString(string: "\n", attributes: [.font: body])) }
        }
        return out
    }

    static func plainText(_ markdown: String) -> String { attributed(markdown).string }

    /// The level of a `#`…`######` heading line, or nil.
    private static func heading(_ line: Substring) -> Int? {
        let hashes = line.prefix { $0 == "#" }.count
        return (1...6).contains(hashes) && line.dropFirst(hashes).first == " " ? hashes : nil
    }

    private static func inline(_ text: String, base: NSFont, color: NSColor) -> NSAttributedString {
        var options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        options.failurePolicy = .returnPartiallyParsedIfPossible
        let parsed = (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
        let out = NSMutableAttributedString()
        for run in parsed.runs {
            var font = base
            let intent = run.inlinePresentationIntent ?? []
            if intent.contains(.stronglyEmphasized) { font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) }
            if intent.contains(.emphasized) { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
            if intent.contains(.code) { font = NSFont.monospacedSystemFont(ofSize: base.pointSize - 1, weight: .regular) }
            var attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
            // Answers can quote the meeting, so only web links are clickable (no file: or app schemes).
            if let link = run.link, ["http", "https"].contains(link.scheme?.lowercased() ?? "") { attributes[.link] = link }
            out.append(NSAttributedString(string: String(parsed[run.range].characters), attributes: attributes))
        }
        return out
    }
}

/// A formatted, selectable answer. A text view rather than `Text`: the live transcript redraws
/// the Ask panel every few seconds, and the view only replaces its text when the answer itself
/// changes, so a selection survives those redraws. ⌘C on a selection copies rich text.
struct AskAnswerText: NSViewRepresentable {
    let markdown: String

    final class Coordinator { var shown: String? }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSTextView {
        let view = NSTextView(usingTextLayoutManager: false)
        view.isEditable = false
        view.isSelectable = true
        view.drawsBackground = false
        view.textContainerInset = .zero
        view.textContainer?.lineFragmentPadding = 0
        view.textContainer?.widthTracksTextView = true
        return view
    }

    func updateNSView(_ view: NSTextView, context: Context) {
        guard context.coordinator.shown != markdown else { return }
        context.coordinator.shown = markdown
        view.textStorage?.setAttributedString(AskMarkdown.attributed(markdown, color: NSColor(Color.whimbrelTextPrimary)))
    }

    /// Measured on a private copy of `markdown`, never on the view: SwiftUI can ask for a size
    /// before `updateNSView` has put new text in, and resizing the view's own container to a
    /// probe width (sometimes near zero inside a ScrollView) left a finished answer laid out one
    /// word per line under its Copy buttons, with nothing to scroll.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView view: NSTextView, context: Context) -> CGSize? {
        fittingSize(width: proposal.width)
    }

    func fittingSize(width: CGFloat?) -> CGSize? {
        guard let width, width > 1 else { return nil }
        return CGSize(width: width, height: Self.height(of: markdown, width: width))
    }

    /// Height of `markdown` laid out at `width`, as the view draws it.
    static func height(of markdown: String, width: CGFloat) -> CGFloat {
        let storage = NSTextStorage(attributedString: AskMarkdown.attributed(markdown)) // colour never changes layout
        let layout = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: width, height: .greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        layout.addTextContainer(container)
        storage.addLayoutManager(layout)
        layout.ensureLayout(for: container)
        return ceil(layout.usedRect(for: container).height)
    }
}
