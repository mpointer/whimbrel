import Foundation
import SwiftUI

// The finished note page and its transcript view as plain values (docs/ui-redesign/screens/note.md).
// A model says what to show; text the app owns arrives in it, already in the person's language.
// `WNoteActions` says what each control does. The views are in NoteViews.swift and Lanes.swift.

/// Which of the page's two views is showing.
public enum WNoteMode: String, Equatable, Sendable { case note, transcript }

/// Where the quote of a source line came from: a line of the transcript, with its own stored words.
public struct WNoteSource: Equatable, Sendable, Identifiable {
    /// The number drawn in the mark and on the margin card, counted down the page. It renumbers when a line is
    /// edited or the summary is made again, so nothing that must outlive one of those keeps it: use `id`.
    public var number: Int
    public var segmentId: String
    public var speaker: String
    /// "18:25".
    public var time: String
    public var quote: String
    /// The words of the summary line this source belongs to.
    public var lineText: String

    public init(number: Int, segmentId: String, speaker: String, time: String, quote: String, lineText: String = "") {
        self.number = number
        self.segmentId = segmentId
        self.speaker = speaker
        self.time = time
        self.quote = quote
        self.lineText = lineText
    }

    /// Stays the same when the numbers shift: the transcript line and the summary line it supports.
    public var id: String { "\(segmentId)\u{1F}\(lineText)" }
}

/// One line of the summary, a decision or an open question. `isAI` is false for a line the person edited:
/// it is theirs, so it has no AI ink and no source.
public struct WNoteLine: Identifiable, Equatable, Sendable {
    public let id: String
    public var text: String
    public var isAI: Bool
    public var source: WNoteSource?

    public init(id: String, text: String, isAI: Bool = true, source: WNoteSource? = nil) {
        self.id = id
        self.text = text
        self.isAI = isAI
        self.source = isAI ? source : nil
    }
}

/// An action item: a line with a checkbox, and an owner and due date only when the model gave them.
public struct WNoteActionItem: Identifiable, Equatable, Sendable {
    public let id: String
    public var line: WNoteLine
    public var isDone: Bool
    public var owner: String?
    public var due: String?

    public init(id: String, line: WNoteLine, isDone: Bool = false, owner: String? = nil, due: String? = nil) {
        self.id = id
        self.line = line
        self.isDone = isDone
        self.owner = owner
        self.due = due
    }
}

/// A moment the person marked. `clock` is the offset into the meeting ("18:25"); `atCs` is the stored time
/// on the clock transcript lines use.
public struct WNoteMark: Identifiable, Equatable, Sendable {
    public let id: String
    public var atCs: Int64
    public var clock: String
    public var label: String

    public init(id: String, atCs: Int64, clock: String, label: String) {
        self.id = id
        self.atCs = atCs
        self.clock = clock
        self.label = label
    }
}

/// One transcript line. `startCs` and `endCs` are centiseconds on one clock; a continued meeting's later parts
/// keep the meeting's clock, and `partIndex` says which part the line is in.
public struct WNoteSegment: Identifiable, Equatable, Sendable {
    public let id: String
    public var startCs: Int64
    public var endCs: Int64?
    public var time: String
    /// The stored label ("Speaker 1"), which tells speakers apart even when two share a shown name.
    public var speakerKey: String
    /// The name as shown, a guess included ("Sam?").
    public var speaker: String
    public var text: String
    public var partIndex: Int
    /// The "rejoined" divider drawn above this line, when it is the first of a later part.
    public var dividerBefore: String?

    public init(
        id: String, startCs: Int64, endCs: Int64? = nil, time: String, speakerKey: String, speaker: String, text: String,
        partIndex: Int = 0, dividerBefore: String? = nil
    ) {
        self.id = id
        self.startCs = startCs
        self.endCs = endCs
        self.time = time
        self.speakerKey = speakerKey
        self.speaker = speaker
        self.text = text
        self.partIndex = partIndex
        self.dividerBefore = dividerBefore
    }
}

/// A transcript line with no stored time: an older meeting whose lines come only from its transcript file.
public struct WNoteTurn: Identifiable, Equatable, Sendable {
    public let id: Int
    public var speaker: String
    public var time: String
    public var text: String
    public var dividerBefore: String?

    public init(id: Int, speaker: String, time: String, text: String, dividerBefore: String? = nil) {
        self.id = id
        self.speaker = speaker
        self.time = time
        self.text = text
        self.dividerBefore = dividerBefore
    }
}

/// What the note area shows when there is no finished summary to draw, or a line about the summary.
public enum WNoteState: Equatable, Sendable {
    /// A summary exists (or the person's own notes do): draw the sections.
    case ready
    /// Transcribing or summarizing: the stage and no summary yet.
    case processing(stage: String)
    /// Transcript only: no summary yet, with the reason in words.
    case noSummary(String)
    /// The summary failed: the failure line. Regenerate is offered.
    case failed(String)
}

public struct WNoteModel: Sendable {
    public var mode: WNoteMode
    /// A label for the back control ("Meetings"); nil when the page is a window with no back.
    public var backTitle: String?
    public var title: String
    /// "Mon, Nov 16, 10:00 AM · 32 min".
    public var eyebrow: String
    /// "Sam Okafor, Dana Whitfield, Jo (Northwind), you". Empty draws nothing.
    public var attendees: String
    /// The person's own notes, as stored. Empty draws nothing.
    public var yourNotes: String
    public var state: WNoteState
    /// Edit is offered whenever the meeting is finished, as it is on the page today. Another device's meeting is not
    /// treated differently.
    public var canEdit: Bool
    /// Whether the failed state offers Regenerate.
    public var canRegenerate: Bool
    public var isEditing: Bool
    public var canSendRecap: Bool
    public var canCopy: Bool
    /// "Sent the recap to 3 people".
    public var sentLine: String?
    public var summary: WNoteLine?
    public var decisions: [WNoteLine]
    public var openQuestions: [WNoteLine]
    public var actionItems: [WNoteActionItem]
    public var marks: [WNoteMark]
    /// "Generated by gemma (ollama)", "Edited by you".
    public var provenance: [String]
    /// The `id` of the source selected on the page.
    public var selectedSource: String?
    /// How much the person's reading size setting scales the note and the transcript (1 is the standard size). The
    /// header and its controls keep their size.
    public var readingScale: CGFloat

    // Transcript
    public var segments: [WNoteSegment]
    /// Lines without times, when there are no stored segments.
    public var turns: [WNoteTurn]
    /// Said when there are no lines: why, in words ("No transcript for this meeting.").
    public var transcriptMessage: String?
    public var highlightedSegmentId: String?
    public var canCopyTranscript: Bool
    /// The lanes, when the app worked them out once for these segments. Nil: the view works them out.
    public var laneGroups: [WLaneGroup]?

    public init(
        mode: WNoteMode = .note, backTitle: String? = nil, title: String, eyebrow: String = "", attendees: String = "", yourNotes: String = "",
        state: WNoteState = .ready, canEdit: Bool = true, canRegenerate: Bool = true, isEditing: Bool = false, canSendRecap: Bool = true,
        canCopy: Bool = true, sentLine: String? = nil, summary: WNoteLine? = nil, decisions: [WNoteLine] = [], openQuestions: [WNoteLine] = [],
        actionItems: [WNoteActionItem] = [], marks: [WNoteMark] = [], provenance: [String] = [], selectedSource: String? = nil,
        readingScale: CGFloat = 1, segments: [WNoteSegment] = [], turns: [WNoteTurn] = [], transcriptMessage: String? = nil,
        highlightedSegmentId: String? = nil, canCopyTranscript: Bool = true, laneGroups: [WLaneGroup]? = nil
    ) {
        self.mode = mode
        self.backTitle = backTitle
        self.title = title
        self.eyebrow = eyebrow
        self.attendees = attendees
        self.yourNotes = yourNotes
        self.state = state
        self.canEdit = canEdit
        self.canRegenerate = canRegenerate
        self.isEditing = isEditing
        self.canSendRecap = canSendRecap
        self.canCopy = canCopy
        self.sentLine = sentLine
        self.summary = summary
        self.decisions = decisions
        self.openQuestions = openQuestions
        self.actionItems = actionItems
        self.marks = marks
        self.provenance = provenance
        self.selectedSource = selectedSource
        self.readingScale = readingScale
        self.segments = segments
        self.turns = turns
        self.transcriptMessage = transcriptMessage
        self.highlightedSegmentId = highlightedSegmentId
        self.canCopyTranscript = canCopyTranscript
        self.laneGroups = laneGroups
    }

    /// Every sourced line in the order it sits on the page: decisions, open questions, then action items.
    public var sources: [WNoteSource] {
        (decisions.compactMap(\.source)) + openQuestions.compactMap(\.source) + actionItems.compactMap(\.line.source)
    }

    /// Whether any line on the page is Whimbrel's.
    public var hasAILines: Bool {
        (summary?.isAI ?? false) || decisions.contains(where: \.isAI) || openQuestions.contains(where: \.isAI) || actionItems.contains(where: \.line.isAI)
    }

    /// The margin is drawn when there is a Whimbrel line to explain.
    public var showsMargin: Bool { hasAILines && !isEditing }

    /// The margin's first line says what the numbers mean, so it is drawn only when there is a number to explain.
    public var showsMarginIntro: Bool { !sources.isEmpty }

    /// Old meetings: Whimbrel lines but none with a source, so the margin says how to get them.
    public var showsSourcesHint: Bool { hasAILines && sources.isEmpty }

    /// Edit is offered whenever the meeting is finished: with a summary, with none yet, while one is being made, and
    /// after a failure. It is what the page offers today.
    public var offersEdit: Bool { canEdit }

    public func source(id: String) -> WNoteSource? { sources.first { $0.id == id } }

    /// The position of the selected source in page order, when it is still on the page.
    public var selectedSourceIndex: Int? { selectedSource.flatMap { id in sources.firstIndex { $0.id == id } } }

    /// Whether there is anything to draw in the note view's sections.
    public var hasNoteContent: Bool {
        summary != nil || !decisions.isEmpty || !openQuestions.isEmpty || !actionItems.isEmpty
    }
}

/// What each control does. Every closure has a do nothing default, so a gallery needs none.
public struct WNoteActions {
    public var back: () -> Void
    public var setMode: (WNoteMode) -> Void
    public var copyPlain: () -> Void
    public var copyMarkdown: () -> Void
    public var copySlack: () -> Void
    public var copyTranscript: () -> Void
    public var sendRecap: () -> Void
    public var toggleEdit: () -> Void
    public var regenerate: () -> Void
    public var selectSource: (String?) -> Void
    /// Opens the transcript at the line a source came from (its segment id).
    public var openSegment: (String) -> Void
    public var toggleActionItem: (String, Bool) -> Void
    /// Opens the transcript at the nearest line to a marked moment.
    public var openMark: (String) -> Void
    public var removeMark: (String) -> Void

    public init(
        back: @escaping () -> Void = {}, setMode: @escaping (WNoteMode) -> Void = { _ in }, copyPlain: @escaping () -> Void = {},
        copyMarkdown: @escaping () -> Void = {}, copySlack: @escaping () -> Void = {}, copyTranscript: @escaping () -> Void = {},
        sendRecap: @escaping () -> Void = {}, toggleEdit: @escaping () -> Void = {}, regenerate: @escaping () -> Void = {},
        selectSource: @escaping (String?) -> Void = { _ in }, openSegment: @escaping (String) -> Void = { _ in },
        toggleActionItem: @escaping (String, Bool) -> Void = { _, _ in }, openMark: @escaping (String) -> Void = { _ in },
        removeMark: @escaping (String) -> Void = { _ in }
    ) {
        self.back = back
        self.setMode = setMode
        self.copyPlain = copyPlain
        self.copyMarkdown = copyMarkdown
        self.copySlack = copySlack
        self.copyTranscript = copyTranscript
        self.sendRecap = sendRecap
        self.toggleEdit = toggleEdit
        self.regenerate = regenerate
        self.selectSource = selectSource
        self.openSegment = openSegment
        self.toggleActionItem = toggleActionItem
        self.openMark = openMark
        self.removeMark = removeMark
    }
}

// MARK: Speaker lanes, as pure functions

public struct WLaneSpan: Identifiable, Equatable, Sendable {
    public let id: String
    public var startCs: Int64
    public var endCs: Int64
    public var speaker: String
    public var time: String
}

public struct WLane: Identifiable, Equatable, Sendable {
    public let id: String
    public var speaker: String
    public var spans: [WLaneSpan]
}

/// The lanes of one part of the meeting, drawn on one time scale from `startCs` to `endCs`.
public struct WLaneGroup: Identifiable, Equatable, Sendable {
    public let id: Int
    public var divider: String?
    public var startCs: Int64
    public var endCs: Int64
    public var lanes: [WLane]
    /// Marked moments inside this part's range.
    public var ticks: [WNoteMark]

    public var spans: [WLaneSpan] { lanes.flatMap(\.spans) }
}

public enum WLaneMath {
    /// A line with no stored end runs until the next line starts, but never longer than this (30 seconds).
    public static let capCs: Int64 = 3000
    /// The last line has no next line to end at; it is drawn this long (5 seconds).
    public static let lastLineCs: Int64 = 500

    /// Where a line's span ends. A stored end wins, when it is after the start. Otherwise the line runs to the next
    /// line's start, at most `capCs` after its own; the last line runs `lastLineCs`. Never before the start.
    public static func spanEnd(start: Int64, storedEnd: Int64?, nextStart: Int64?) -> Int64 {
        if let storedEnd, storedEnd > start { return storedEnd }
        guard let nextStart else { return start + lastLineCs }
        return max(start, min(nextStart, start + capCs))
    }

    /// How far past a mark that falls after the last span the lane's range runs, so the tick is not on the edge (5 seconds).
    public static let tailCs: Int64 = 500

    /// The lanes, one group per part. Lanes are in order of first speech; a group's range starts at the meeting's
    /// beginning for the first part and at its first line for a later one, and ends where its last span ends. Every
    /// mark of the meeting is a tick in exactly one group: the last one that starts at or before it. A mark in the
    /// silence after a part's last span, or in the gap before the next part, stretches that part's range to reach it.
    public static func groups(segments: [WNoteSegment], marks: [WNoteMark]) -> [WLaneGroup] {
        let sorted = segments.enumerated().sorted { ($0.element.startCs, $0.offset) < ($1.element.startCs, $1.offset) }.map(\.element)
        var byPart: [Int: [WNoteSegment]] = [:]
        for segment in sorted { byPart[segment.partIndex, default: []].append(segment) }
        let partKeys = byPart.keys.sorted()
        var out: [WLaneGroup] = []
        for (position, part) in partKeys.enumerated() {
            let lines = byPart[part] ?? []
            var order: [String] = []
            var spansByKey: [String: [WLaneSpan]] = [:]
            var names: [String: String] = [:]
            var end: Int64 = lines.first?.startCs ?? 0
            for (index, line) in lines.enumerated() {
                let next = index + 1 < lines.count ? lines[index + 1].startCs : nil
                let stop = spanEnd(start: line.startCs, storedEnd: line.endCs, nextStart: next)
                if spansByKey[line.speakerKey] == nil { order.append(line.speakerKey); names[line.speakerKey] = line.speaker }
                spansByKey[line.speakerKey, default: []].append(
                    WLaneSpan(id: line.id, startCs: line.startCs, endCs: stop, speaker: line.speaker, time: line.time))
                end = max(end, stop)
            }
            let start = position == 0 ? 0 : (lines.first?.startCs ?? 0)
            out.append(WLaneGroup(
                id: part, divider: lines.first?.dividerBefore, startCs: start, endCs: max(end, start + 1),
                lanes: order.map { WLane(id: "\(part)-\($0)", speaker: names[$0] ?? $0, spans: spansByKey[$0] ?? []) }, ticks: []))
        }
        guard !out.isEmpty else { return out }
        for mark in marks.sorted(by: { ($0.atCs, $0.id) < ($1.atCs, $1.id) }) {
            let index = out.lastIndex { $0.startCs <= mark.atCs } ?? 0
            out[index].ticks.append(mark)
            if mark.atCs >= out[index].endCs { out[index].endCs = mark.atCs + tailCs }
        }
        return out
    }

    /// The id of the line whose start is closest to a time; used to open a marked moment.
    public static func nearestSegment(to atCs: Int64, in segments: [WNoteSegment]) -> String? {
        segments.min { abs($0.startCs - atCs) < abs($1.startCs - atCs) }?.id
    }

    /// "18:25" from centiseconds, and "62:03" past an hour. This is the format the transcript has always shown, so a
    /// source card, a mark, a transcript line and the lanes all say the same time.
    public static func clock(cs: Int64) -> String {
        let total = max(0, Int(cs / 100))
        return String(format: "%d:%02d", total / 60, total % 60)
    }

    /// Every span of every part, in time order within a part and parts in order.
    public static func timeOrdered(_ groups: [WLaneGroup]) -> [WLaneSpan] {
        groups.flatMap { $0.spans.sorted { ($0.startCs, $0.id) < ($1.startCs, $1.id) } }
    }

    /// The span `delta` places along in time order (the left and right arrow keys). Stays at the ends. With no
    /// current span it starts at the first, or the last when going back.
    public static func neighbor(of id: String?, by delta: Int, in groups: [WLaneGroup]) -> String? {
        let flat = timeOrdered(groups)
        guard !flat.isEmpty else { return nil }
        guard let id, let index = flat.firstIndex(where: { $0.id == id }) else { return delta >= 0 ? flat.first?.id : flat.last?.id }
        return flat[min(max(index + delta, 0), flat.count - 1)].id
    }

    /// The span in the next lane (`delta` 1) or the one above (`delta` -1) of the same part that starts nearest in time
    /// to this one (the up and down arrow keys). Stays put at the first and last lane.
    public static func otherLane(from id: String?, by delta: Int, in groups: [WLaneGroup]) -> String? {
        guard let id else { return neighbor(of: nil, by: delta, in: groups) }
        for group in groups {
            for (laneIndex, lane) in group.lanes.enumerated() {
                guard let here = lane.spans.first(where: { $0.id == id }) else { continue }
                let target = laneIndex + delta
                guard group.lanes.indices.contains(target) else { return id }
                return group.lanes[target].spans.min { abs($0.startCs - here.startCs) < abs($1.startCs - here.startCs) }?.id ?? id
            }
        }
        return id
    }

    /// The span `delta` places along in one lane, in time order (a screen reader's adjust action). Stays at the ends.
    /// With no current span it starts at the first, or the last when going back.
    public static func step(in lane: WLane, from id: String?, by delta: Int) -> String? {
        guard !lane.spans.isEmpty else { return nil }
        guard let id, let index = lane.spans.firstIndex(where: { $0.id == id }) else { return delta >= 0 ? lane.spans.first?.id : lane.spans.last?.id }
        return lane.spans[min(max(index + delta, 0), lane.spans.count - 1)].id
    }

    /// The span a click at `cs` lands on: the one that holds that time, else the nearest start or end within
    /// `toleranceCs` (a narrow span is hard to hit exactly).
    public static func span(at cs: Int64, in lane: WLane, toleranceCs: Int64) -> WLaneSpan? {
        if let inside = lane.spans.first(where: { $0.startCs <= cs && cs <= $0.endCs }) { return inside }
        var best: (span: WLaneSpan, distance: Int64)?
        for span in lane.spans {
            let distance = cs < span.startCs ? span.startCs - cs : cs - span.endCs
            if distance <= toleranceCs, distance < (best?.distance ?? Int64.max) { best = (span, distance) }
        }
        return best?.span
    }
}

// MARK: Margin cards

public enum WMarginLayout {
    /// The top of each margin card. A card sits level with its line (`anchors`) and is pushed down only as far as it
    /// must be so it never overlaps the card above (`heights`, with `gap` between) or the intro line (`minTop`).
    /// The selected card (`pinned`) is the one that grows, so it holds its place and its neighbours above it make room
    /// by moving up, never below `minTop`; if they cannot all fit above, the pinned card moves down as little as needed.
    public static func tops(anchors: [CGFloat], heights: [CGFloat], minTop: CGFloat, gap: CGFloat, pinned: Int? = nil) -> [CGFloat] {
        func height(_ index: Int) -> CGFloat { index < heights.count ? heights[index] : 0 }
        var tops = anchors.map { max($0, minTop) }
        if let pinned, tops.indices.contains(pinned) {
            var index = pinned - 1
            while index >= 0 {
                tops[index] = min(tops[index], tops[index + 1] - gap - height(index))
                index -= 1
            }
        }
        var floor = minTop
        for index in tops.indices {
            tops[index] = max(tops[index], floor)
            floor = tops[index] + height(index) + gap
        }
        return tops
    }

    /// How the page lays out for a width: the margin column beside the note, or folded away.
    public static func isWide(width: CGFloat, scale: CGFloat) -> Bool { width >= (440 + 300 + 24) * scale }
}

/// How the person's reading size setting (Settings, Meetings) scales the note and the transcript.
public enum WReadingScale {
    /// The setting's default, which is the page's standard size.
    public static let standardPoints: Double = 13

    /// 1 at the default, in proportion above and below, kept within what the setting offers (11 to 20 points).
    public static func scale(fontSize: Double, standard: Double = standardPoints) -> CGFloat {
        guard standard > 0, fontSize.isFinite else { return 1 }
        return CGFloat(min(max(fontSize, 11), 20) / standard)
    }
}
