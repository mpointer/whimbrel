import Foundation
import SwiftUI

// An answer as text a person can read, select and copy (docs/ui-redesign/screens/ask.md, "The page" and "Keyboard and screen reader").
// The answer is cut into blocks (paragraphs, list items, headings, code, tables); each block is one attributed `Text`, with Markdown through
// the same parser options the old Ask page uses, and each source mark an inline link run. The marker grammar stays in `WAnswerMarks.parts`,
// the one tested parser, and which numbers become marks is the core's `cited` list; nothing here reads a `[n]` any other way.

/// One block of an answer.
public enum WAnswerBlock: Equatable, Sendable {
    /// A paragraph. Its lines are joined with a newline, which the Markdown parser reads as a soft break, as on the old page.
    case paragraph(String)
    case heading(String)
    /// A list item. `marker` is what stands before it ("\u{2022}" for a bullet, "2." for a numbered item); `level` is how deep it nests (0 is
    /// the outer list).
    case item(marker: String, text: String, level: Int)
    /// A fenced block: shown as written, never read for marks or Markdown.
    case code(String)
    /// A table, as aligned monospaced text (the pipes stay, the columns line up). Not read as Markdown, but marks still apply.
    case table(String)
}

public enum WAnswerBlocks {
    private static let bulletMarkers: [Character] = ["-", "*", "+"]
    private static let newline = String(Character(UnicodeScalar(10)))
    private static let tab = Character(UnicodeScalar(9))

    /// A fence in the core's reading (`llm::bracketed_numbers`): up to three spaces, then three or more backticks whose info string holds
    /// no backtick, or three or more tildes. It opens a block only when it is the whole line's business; a "```code```" on one line is
    /// inline code in a paragraph.
    struct Fence: Equatable {
        var char: Character
        var length: Int
    }

    static func fenceOpening(_ raw: String) -> Fence? {
        let indent = leadingSpaces(raw)
        guard indent <= 3 else { return nil }
        let rest = raw.dropFirst(indent)
        guard let first = rest.first, first == "`" || first == "~" else { return nil }
        let run = rest.prefix { $0 == first }
        guard run.count >= 3 else { return nil }
        if first == "`", rest.dropFirst(run.count).contains("`") { return nil }
        return Fence(char: first, length: run.count)
    }

    /// A line of the same character, at least as long, with nothing but whitespace after it, closes the fence.
    static func closes(_ raw: String, _ fence: Fence) -> Bool {
        let indent = leadingSpaces(raw)
        guard indent <= 3 else { return false }
        let rest = raw.dropFirst(indent)
        let run = rest.prefix { $0 == fence.char }
        guard run.count >= fence.length else { return false }
        return rest.dropFirst(run.count).allSatisfy { $0.isWhitespace }
    }

    private static func leadingSpaces(_ raw: String) -> Int {
        var n = 0
        for c in raw {
            if c == " " { n += 1 } else if c == tab { n += 4 } else { break }
        }
        return n
    }

    /// Cuts an answer into blocks. A blank line separates blocks; consecutive plain lines are one paragraph (a single newline is a soft
    /// break, as on the old page); a list item starts a block of its own and a line indented under it continues it; an item indented
    /// under another nests one level deeper. Lines between a pair of fences are one code block, and a table is one block.
    public static func parse(_ text: String) -> [WAnswerBlock] {
        var blocks: [WAnswerBlock] = []
        var fence: Fence?
        var code: [String] = []
        var paragraph: [String] = []
        var indents: [Int] = []
        let lines = text.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).map(String.init)
        func flushParagraph() {
            guard !paragraph.isEmpty else { return }
            blocks.append(.paragraph(paragraph.joined(separator: newline).trimmingCharacters(in: .whitespaces)))
            paragraph = []
        }
        var index = 0
        while index < lines.count {
            let raw = lines[index]
            index += 1
            if let open = fence {
                if closes(raw, open) { blocks.append(.code(code.joined(separator: newline))); fence = nil; code = [] } else { code.append(raw) }
                continue
            }
            if let open = fenceOpening(raw) {
                flushParagraph()
                fence = open
                continue
            }
            let trimmed = raw.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty { flushParagraph(); continue }
            if isTableRow(trimmed), index < lines.count, isTableSeparator(lines[index]) {
                flushParagraph()
                var rows = [trimmed]
                rows.append(lines[index].trimmingCharacters(in: .whitespaces))
                index += 1
                while index < lines.count {
                    let next = lines[index].trimmingCharacters(in: .whitespaces)
                    guard !next.isEmpty, isTableRow(next) else { break }
                    rows.append(next)
                    index += 1
                }
                blocks.append(.table(alignedTable(rows)))
                continue
            }
            let indent = leadingSpaces(raw)
            if let (marker, rest) = itemParts(trimmed) {
                flushParagraph()
                blocks.append(.item(marker: marker, text: rest, level: level(forIndent: indent, in: &indents)))
            } else if paragraph.isEmpty, indent > 0, case .item(let marker, let previous, let level)? = blocks.last {
                blocks[blocks.count - 1] = .item(marker: marker, text: previous + newline + trimmed, level: level)
            } else if let heading = headingText(trimmed) {
                flushParagraph()
                indents = []
                blocks.append(.heading(heading))
            } else {
                if paragraph.isEmpty { indents = [] }
                paragraph.append(String(raw.drop { $0 == " " || $0 == tab }))
            }
        }
        flushParagraph()
        if fence != nil { blocks.append(.code(code.joined(separator: newline))) }
        return blocks
    }

    /// How deep an item at `indent` nests: an indent two or more spaces past the open level's opens a deeper one; one two or more spaces
    /// short of it closes levels; within a space of an open level's indent it belongs to that level.
    private static func level(forIndent indent: Int, in indents: inout [Int]) -> Int {
        while let last = indents.last, last - indent >= 2 { indents.removeLast() }
        if let last = indents.last, abs(last - indent) < 2 {
            // The same level.
        } else {
            indents.append(indent)
        }
        return indents.count - 1
    }

    private static func headingText(_ line: String) -> String? {
        let hashes = line.prefix { $0 == "#" }
        guard (1...6).contains(hashes.count) else { return nil }
        let rest = line.dropFirst(hashes.count)
        guard rest.first == " " else { return nil }
        let text = rest.trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? nil : text
    }

    private static func itemParts(_ line: String) -> (String, String)? {
        guard let first = line.first else { return nil }
        let rest = line.dropFirst()
        if bulletMarkers.contains(first), rest.first == " " {
            let text = rest.trimmingCharacters(in: .whitespaces)
            return text.isEmpty ? nil : ("\u{2022}", text)
        }
        let digits = line.prefix { $0.isASCII && $0.isNumber }
        guard !digits.isEmpty, digits.count <= 3 else { return nil }
        let after = line.dropFirst(digits.count)
        guard let punct = after.first, punct == "." || punct == ")", after.dropFirst().first == " " else { return nil }
        let text = after.dropFirst().trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? nil : (String(digits) + ".", text)
    }

    // MARK: Tables

    private static func isTableRow(_ line: String) -> Bool { line.contains("|") }

    /// A delimiter row: cells of dashes with optional colons, between pipes, and at least one pipe ("---" alone is a rule, not a table).
    static func isTableSeparator(_ raw: String) -> Bool {
        let line = raw.trimmingCharacters(in: .whitespaces)
        guard line.contains("|"), line.contains("-") else { return false }
        let cells = cells(of: line)
        guard !cells.isEmpty else { return false }
        return cells.allSatisfy { cell in
            let c = cell.trimmingCharacters(in: .whitespaces)
            return !c.isEmpty && c.contains("-") && c.allSatisfy { $0 == "-" || $0 == ":" }
        }
    }

    /// The cells of a row: split at pipes that are not escaped, the outer pipes dropped, each cell trimmed.
    static func cells(of line: String) -> [String] {
        var out: [String] = []
        var current = ""
        var escaped = false
        for c in line {
            if escaped { current.append(c == "|" ? "|" : c); escaped = false; continue }
            if c == "\\" { escaped = true; continue }
            if c == "|" { out.append(current); current = "" } else { current.append(c) }
        }
        out.append(current)
        var trimmed = out.map { $0.trimmingCharacters(in: .whitespaces) }
        if line.hasPrefix("|"), !trimmed.isEmpty { trimmed.removeFirst() }
        if line.hasSuffix("|"), !line.hasSuffix("\\|"), !trimmed.isEmpty { trimmed.removeLast() }
        return trimmed
    }

    /// The rows as monospaced text with the columns lined up: cells padded to the widest in their column, the delimiter row redrawn.
    static func alignedTable(_ rows: [String]) -> String {
        let parsed = rows.enumerated().map { $0.offset == 1 ? [String]() : cells(of: $0.element) }
        let columns = parsed.map(\.count).max() ?? 0
        var widths = [Int](repeating: 1, count: columns)
        for row in parsed { for (i, cell) in row.enumerated() { widths[i] = max(widths[i], cell.count) } }
        func pad(_ s: String, _ w: Int) -> String { s + String(repeating: " ", count: max(0, w - s.count)) }
        var lines: [String] = []
        for (r, row) in parsed.enumerated() {
            if r == 1 {
                lines.append(widths.map { String(repeating: "-", count: $0) }.joined(separator: " | "))
                continue
            }
            let cells = (0..<columns).map { pad($0 < row.count ? row[$0] : "", widths[$0]) }
            lines.append(cells.joined(separator: " | ").replacingOccurrences(of: " +$", with: "", options: .regularExpression))
        }
        return lines.joined(separator: newline)
    }
}

/// The inline text of one block as an `AttributedString`, with each source mark a link run.
public enum WAnswerAttributed {
    /// The link a mark carries. Plain letters only: a scheme of its own, the number as the path.
    public static func url(forSource n: Int) -> URL? {
        var parts = URLComponents()
        parts.scheme = scheme
        parts.path = String(n)
        return parts.url
    }

    static let scheme = "whimbrelsource"

    /// The source a mark's link names, or nil for any other link.
    public static func source(in url: URL) -> Int? {
        guard url.scheme == scheme else { return nil }
        return Int(url.path.isEmpty ? (url.host ?? "") : url.path)
    }

    /// The same parse the old Ask page uses (`TimelineView.markdownAttributedString`): Markdown as far as it reads, partly parsed when it is
    /// malformed, plain when it fails. Kept here because the package cannot see the app; the app's tests hold the two equal.
    public static func markdown(_ text: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.failurePolicy = .returnPartiallyParsedIfPossible
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }

    /// One block's words. Markdown first (unless `markdown` is false: a table's text is shown as it is); then, in every run that is not
    /// code and not already a link, each `[n]` the core cited (`cited`) becomes the run "[n]" carrying a link to that source. A marker
    /// inside a code span stays literal. A number the core did not cite, and bracketed text that is not a marker, stay as written.
    /// `markColor` colours the marks; the selected one also gets a background.
    public static func build(
        _ text: String, cited: Set<Int>, selected: Int? = nil, markColor: Color? = nil, selectedBackground: Color? = nil, markdown parses: Bool = true
    ) -> AttributedString {
        var result = parses ? markdown(text) : AttributedString(text)
        var edits: [(Range<AttributedString.Index>, AttributedString)] = []
        for run in result.runs {
            if run.link != nil { continue }
            if let intent = run.inlinePresentationIntent, intent.contains(.code) { continue }
            let words = String(result[run.range].characters)
            let parts = WAnswerMarks.parts(words, cited: cited)
            guard parts.contains(where: { if case .mark = $0 { return true } else { return false } }) else { continue }
            var container = AttributeContainer()
            container.inlinePresentationIntent = run.inlinePresentationIntent
            var replacement = AttributedString()
            for part in parts {
                switch part {
                case .text(let plain):
                    replacement.append(AttributedString(plain, attributes: container))
                case .mark(let n):
                    var mark = AttributedString("[\(n)]", attributes: container)
                    mark.link = url(forSource: n)
                    mark.inlinePresentationIntent = [.stronglyEmphasized]
                    if let markColor { mark.foregroundColor = markColor }
                    if n == selected, let selectedBackground { mark.backgroundColor = selectedBackground }
                    replacement.append(mark)
                }
            }
            edits.append((run.range, replacement))
        }
        for (range, replacement) in edits.reversed() { result.replaceSubrange(range, with: replacement) }
        return result
    }

    /// The numbers of the marks in a built string, in order (for the tests and for the screen reader's list of sources).
    public static func markNumbers(in text: AttributedString) -> [Int] {
        text.runs.compactMap { $0.link.flatMap(source(in:)) }
    }
}

/// What the page says aloud when an ask finishes.
public enum WFindAnnouncement {
    public static func spoken(for exchange: WFindExchange, strings: WFindStrings) -> String {
        switch exchange.body {
        case .answered(let text), .legacy(let text, _): return strings.answerIn(WAnswerMarks.plain(text, cited: exchange.citedNumbers))
        case .failed(let message, _): return message
        }
    }
}
