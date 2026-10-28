import SwiftUI

/// The numbers behind the pitch drawing, shared with the contrast test.
enum FieldStyle {
    /// Strength of the `accentPrimary` mowing stripes over the pitch (`backgroundCard`).
    static let stripeOpacity = 0.10
    /// Strength of the `border` chalk lines over the pitch. Labels may cross one.
    static let lineOpacity = 0.45
}

/// Where the markers and their labels go along the pitch. Pure, so the layout is testable.
///
/// Markers sit on the centre line in start order, spaced by time but never closer than `minGap`.
/// Labels alternate above and below the line, so a label can run to the next label in its own row,
/// two markers on: the widths below never overlap, whatever the times.
enum FieldGeometry {
    static let markerDiameter: CGFloat = 24
    /// Least distance between two marker centres.
    static let minGap: CGFloat = 44
    /// Gap between two labels in the same row.
    static let labelGap: CGFloat = 6
    /// Room kept to the right of the last marker's left edge for its label.
    static let minLabel: CGFloat = 96

    struct Slot: Equatable {
        let centerX: CGFloat
        /// Leading edge of the marker and of its label.
        let labelX: CGFloat
        let labelWidth: CGFloat
        /// 0 puts the label above the line, 1 below it.
        let row: Int
    }

    /// One slot per start (ascending), for a pitch `width` wide.
    static func slots(starts: [Date], width: CGFloat) -> [Slot] {
        let n = starts.count
        guard n > 0 else { return [] }
        let radius = markerDiameter / 2
        let lo = radius, hi = max(lo, width - minLabel)
        let gap = n > 1 ? min(minGap, (hi - lo) / CGFloat(n - 1)) : 0
        let first = starts[0].timeIntervalSinceReferenceDate, span = starts[n - 1].timeIntervalSinceReferenceDate - first
        // By time when the times differ, evenly when they do not.
        var x = starts.enumerated().map { i, start -> CGFloat in
            let fraction = span > 0 ? (start.timeIntervalSinceReferenceDate - first) / span : (n > 1 ? Double(i) / Double(n - 1) : 0)
            return lo + CGFloat(fraction) * (hi - lo)
        }
        for i in x.indices.dropFirst() { x[i] = max(x[i], x[i - 1] + gap) }
        x[n - 1] = min(x[n - 1], hi)
        for i in x.indices.dropLast().reversed() { x[i] = min(x[i], x[i + 1] - gap) }
        return x.enumerated().map { i, centerX in
            let labelX = centerX - radius
            let end = i + 2 < n ? x[i + 2] - radius - labelGap : width
            return Slot(centerX: centerX, labelX: labelX, labelWidth: max(0, end - labelX), row: i % 2)
        }
    }
}

/// One meeting on the pitch. It does what its schedule row does: Start for an upcoming event while
/// no meeting is active, Open for a meeting already held, nothing otherwise (`TodayRows` decides).
struct FieldMarker: Identifiable {
    enum Action {
        case start(TodayUpcoming)
        case open(Int64)
    }

    let id: String
    let start: Date
    let timeText: String
    let title: String
    let isNext: Bool
    let isHeld: Bool
    let action: Action?

    /// What VoiceOver reads: the verb Whimbrel's button would carry, then the meeting.
    var accessibilityLabel: String {
        let suffix = isNext ? ", up next" : ""
        switch action {
        case .start?: return "Start \(title), \(timeText)\(suffix)"
        case .open?: return "Open \(title), \(timeText)\(suffix)"
        case nil: return "\(title), \(timeText)\(suffix)"
        }
    }
}

/// Today's meetings split into pitch markers and the list under the pitch.
struct FieldDay {
    /// More meetings than this and the rest go to a list under the pitch.
    static let maxMarkers = 8

    let markers: [FieldMarker]
    /// Meetings that did not make the pitch, ascending.
    let overflow: (upcoming: [TodayEventRowModel], held: [TodayMeetingRowModel])
    var hiddenCount: Int { overflow.upcoming.count + overflow.held.count }
    var moreLabel: String? { hiddenCount > 0 ? "+\(hiddenCount) more" : nil }

    private enum Entry {
        case event(TodayEventRowModel)
        case held(TodayMeetingRowModel)

        var start: Date {
            switch self {
            case let .event(e): return e.start
            case let .held(m): return Date(timeIntervalSince1970: Double(m.startUnixMs) / 1000)
            }
        }
    }

    /// Today's meetings: upcoming ones first (up next onward, ascending), then the latest held ones for
    /// the slots left, eight in all, shown in time order. Everything else is the list under the pitch.
    static func make(_ content: TodayContent) -> FieldDay {
        let schedule = TodayRows.schedule(content, upNextPlaced: false)
        // `schedule(upNextPlaced: false)` flags no row as next; up next is the first upcoming event.
        let nextId = TodayRows.upNext(content)?.id
        let upcoming = schedule.upcoming.prefix(maxMarkers)
        let spare = maxMarkers - upcoming.count
        let held = spare > 0 ? schedule.held.suffix(spare) : []
        let entries = (upcoming.map(Entry.event) + held.map(Entry.held)).sorted { $0.start < $1.start }
        let markers = entries.map { entry -> FieldMarker in
            switch entry {
            case let .event(e):
                return FieldMarker(
                    id: e.id, start: e.start, timeText: e.timeText, title: e.title, isNext: e.id == nextId, isHeld: false,
                    action: e.showsStart ? .start(e.event) : nil)
            case let .held(m):
                return FieldMarker(
                    id: "held-\(m.id)", start: entry.start, timeText: m.timeText, title: m.title, isNext: false, isHeld: true,
                    action: .open(m.id))
            }
        }
        return FieldDay(markers: markers, overflow: (Array(schedule.upcoming.dropFirst(maxMarkers)), Array(schedule.held.dropLast(held.count))))
    }
}

/// A pitch with today's meetings as markers along it, left to right by start time, a scoreboard
/// for up next, and the meetings held on earlier days as a results list. Places greeting, composer,
/// upNext, schedule and earlier; the rest (threads, recap, devices) is `TodayChrome`'s leftovers.
///
/// `.upNext` and `.schedule` are both placed. Controller ruling (Task 8): the pitch is a whole-day timeline,
/// so up next stays on it as the highlighted marker as well as on the scoreboard (the usual dropFirst is waived).
struct FieldTodayLayout: TodayArchetypeView {
    let content: TodayContent
    let actions: TodayActions
    let composer: AnyView?

    init(content: TodayContent, actions: TodayActions, composer: AnyView?) {
        self.content = content
        self.actions = actions
        self.composer = composer
    }

    static var placedSections: Set<TodaySection> { [.greeting, .composer, .upNext, .schedule, .earlier] }

    private var skin: Skin { SkinStore.shared.selected }
    private var voice: SkinVoice { skin.voice }
    private let timeWidth: CGFloat = 76
    private let pitchPadding: CGFloat = 12
    private let labelHeight: CGFloat = 36
    /// Space above and below a ball, so the up-next ring (3pt out) never touches a label.
    private let ringSlack: CGFloat = 4

    var body: some View {
        let sections = content.nonEmptySections()
        VStack(alignment: .leading, spacing: Spacing.lg) {
            header.skinBadge()
            if sections.contains(.composer), let composer { composer }
            if sections.contains(.upNext), let next = TodayRows.upNext(content) { scoreboard(next) }
            if sections.contains(.schedule) {
                let day = FieldDay.make(content)
                if !day.markers.isEmpty {
                    section(voice.text(.upcomingHeader, name: "")) {
                        pitch(day.markers)
                        if let more = day.moreLabel {
                            SkinText(more, role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
                            ForEach(TodayRows.scheduleItems(day.overflow)) { item in
                                switch item {
                                case let .event(event): eventRow(event)
                                case .separator: SkinText(TodayRows.heldSeparator, role: .label).foregroundStyle(Color.whimbrelTextSecondary)
                                case let .held(meeting): meetingRow(meeting)
                                }
                            }
                        }
                    }
                }
            }
            if sections.contains(.earlier) {
                section(voice.text(.earlierHeader, name: "")) {
                    ForEach(TodayRows.earlier(content), id: \.id) { meetingRow($0) }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: pieces

    /// A date under the greeting; with no usable profile name the date is the title. A diagonal-stripe band closes it.
    private var header: some View {
        let dateLine = content.now.formatted(.dateTime.weekday(.wide).month(.wide).day())
        let greeting = TodayLogic.greeting(name: content.name, now: content.now, calendar: .current, voice: voice)
        return VStack(alignment: .leading, spacing: Spacing.sm) {
            if greeting != nil {
                SkinText(skin.typography.caps.apply(dateLine), role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
            }
            SkinText(greeting ?? dateLine, role: .display, lines: 3, size: 34).foregroundStyle(Color.whimbrelTextPrimary)
            if skin.decoration.headerBand == .stripesDiagonal {
                DiagonalStripes().frame(height: 8).clipShape(Capsule()).accessibilityHidden(true)
            }
        }
    }

    private func section<C: View>(_ title: String, @ViewBuilder _ rows: () -> C) -> some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            chalk
            SkinText(skin.typography.caps.apply(title), role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
            rows()
        }
    }

    @ViewBuilder
    private var chalk: some View {
        if skin.decoration.dividers != .none {
            ChalkRule().stroke(Color.whimbrelBorder, style: StrokeStyle(lineWidth: 2, lineCap: .round, dash: [18, 4])).frame(height: 2).accessibilityHidden(true)
        }
    }

    // MARK: scoreboard

    /// Up next: the start time on split-flap tiles (the skin's numerals face), then the meeting and Start.
    private func scoreboard(_ event: TodayEventRowModel) -> some View {
        let shape = RoundedRectangle(cornerRadius: Radius.card)
        return VStack(alignment: .leading, spacing: Spacing.md) {
            SkinText(skin.typography.caps.apply(voice.text(.upNextHeader, name: "")), role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
            HStack(alignment: .top, spacing: Spacing.lg) {
                digits(event.timeText)
                VStack(alignment: .leading, spacing: 2) {
                    SkinText(event.title, role: .display, lines: 2, size: 20).foregroundStyle(Color.whimbrelTextPrimary)
                    if let relative = event.relative { SkinText(relative, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
                    if !event.attendees.isEmpty { SkinText(event.attendees, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
                    SkinPrepLine(prep: event.prep)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if event.showsStart {
                    Button("Start") { actions.startMeeting(event.event) }.buttonStyle(.whimbrelPrimary)
                }
            }
        }
        .padding(Spacing.lg * skin.shape.density.scale)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.whimbrelBackgroundCard, in: shape)
        .overlay(shape.stroke(Color.whimbrelAccentPrimary, lineWidth: skin.shape.borderWidth))
    }

    /// "11:00 AM" (the space may be a narrow no-break one) as a tile per digit, a colon, and the rest ("AM") in small type. One element for VoiceOver.
    private func digits(_ time: String) -> some View {
        let parts = time.split(maxSplits: 1, whereSeparator: \.isWhitespace).map(String.init)
        let face = skin.typography.numerals
        return HStack(alignment: .firstTextBaseline, spacing: Spacing.xs) {
            HStack(spacing: 3) {
                ForEach(Array((parts.first ?? time).enumerated()), id: \.offset) { _, char in
                    if char.isNumber {
                        Text(String(char))
                            .font(face.font(size: 30))
                            .foregroundStyle(Color.whimbrelTextOnAccent)
                            .frame(width: 24, height: 40)
                            .background {
                                ZStack {
                                    RoundedRectangle(cornerRadius: 4).fill(Color.whimbrelAccentPrimary)
                                    Rectangle().fill(Color.whimbrelBackgroundCard.opacity(0.5)).frame(height: 1)
                                }
                            }
                    } else {
                        Text(String(char)).font(face.font(size: 30)).foregroundStyle(Color.whimbrelTextPrimary)
                    }
                }
            }
            if parts.count > 1 { Text(parts[1]).font(face.font(size: 14)).foregroundStyle(Color.whimbrelTextSecondary) }
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(time)
    }

    // MARK: pitch

    private var pitchHeight: CGFloat { pitchPadding * 2 + labelHeight * 2 + FieldGeometry.markerDiameter + ringSlack * 2 }

    private func pitch(_ markers: [FieldMarker]) -> some View {
        let shape = RoundedRectangle(cornerRadius: skin.shape.radius)
        return GeometryReader { geo in
            let slots = FieldGeometry.slots(starts: markers.map(\.start), width: max(0, geo.size.width - pitchPadding * 2))
            ZStack(alignment: .topLeading) {
                PitchLines().accessibilityHidden(true)
                ForEach(Array(zip(markers, slots)), id: \.0.id) { marker, slot in
                    markerView(marker, slot)
                        .offset(x: pitchPadding + slot.labelX, y: pitchPadding + (slot.row == 0 ? 0 : labelHeight))
                }
            }
        }
        .frame(height: pitchHeight)
        .background(Color.whimbrelBackgroundCard, in: shape)
        .clipShape(shape)
        .overlay(shape.stroke(Color.whimbrelBorder, lineWidth: skin.shape.borderWidth))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Today's meetings, by start time")
    }

    /// The marker and its label in one control, so the label, the ball and the keyboard all do the same thing.
    @ViewBuilder
    private func markerView(_ marker: FieldMarker, _ slot: FieldGeometry.Slot) -> some View {
        let body = VStack(alignment: .leading, spacing: 0) {
            if slot.row == 0 { markerLabel(marker, slot) }
            ball(marker)
            if slot.row == 1 { markerLabel(marker, slot) }
        }
        .frame(width: slot.labelWidth, alignment: .leading)
        switch marker.action {
        case let .start(event)?:
            Button { actions.startMeeting(event) } label: { body }
                .buttonStyle(.plain)
                .accessibilityLabel(marker.accessibilityLabel)
        case let .open(id)?:
            Button { actions.openMeeting(id) } label: { body }
                .buttonStyle(.plain)
                .accessibilityLabel(marker.accessibilityLabel)
        case nil:
            body.accessibilityElement(children: .ignore).accessibilityLabel(marker.accessibilityLabel)
        }
    }

    /// Time over title, cut with an ellipsis (and a tooltip) at the label's width.
    private func markerLabel(_ marker: FieldMarker, _ slot: FieldGeometry.Slot) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            SkinText(marker.timeText, role: .numerals, size: 11).foregroundStyle(Color.whimbrelTextSecondary)
            SkinText(marker.title, role: .body, size: 12).foregroundStyle(marker.isHeld ? Color.whimbrelTextSecondary : Color.whimbrelTextPrimary)
        }
        .frame(width: slot.labelWidth, height: labelHeight, alignment: slot.row == 0 ? .bottomLeading : .topLeading)
    }

    /// Filled for a meeting to come, hollow with a tick for one held; up next has a highlight ring.
    private func ball(_ marker: FieldMarker) -> some View {
        let d = FieldGeometry.markerDiameter
        return ZStack {
            if marker.isHeld {
                Circle().fill(Color.whimbrelBackgroundCard)
                Circle().stroke(Color.whimbrelAccentPrimary, lineWidth: 2)
                Image(systemName: "checkmark").font(.system(size: 10, weight: .bold)).foregroundStyle(Color.whimbrelAccentPrimary)
            } else {
                Circle().fill(Color.whimbrelAccentPrimary)
                Image(systemName: "play.fill").font(.system(size: 9)).foregroundStyle(Color.whimbrelTextOnAccent)
            }
            if marker.isNext { Circle().stroke(Color.whimbrelAccentHighlight, lineWidth: 3).padding(-3) }
        }
        .frame(width: d, height: d)
        .padding(.vertical, ringSlack)
        .accessibilityHidden(true)
    }

    // MARK: rows (the list under the pitch, and results)

    private func eventRow(_ event: TodayEventRowModel) -> some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            SkinText(event.timeText, role: .numerals).foregroundStyle(Color.whimbrelTextSecondary).frame(width: timeWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                SkinText(event.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
                if !event.attendees.isEmpty { SkinText(event.attendees, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if event.showsStart {
                Button("Start") { actions.startMeeting(event.event) }.buttonStyle(.whimbrelPrimary)
            }
        }
    }

    private func meetingRow(_ meeting: TodayMeetingRowModel) -> some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            SkinText(meeting.timeText, role: .numerals).foregroundStyle(Color.whimbrelTextSecondary).frame(width: timeWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                SkinText(meeting.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
                if !meeting.subtitle.isEmpty { SkinText(meeting.subtitle, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button("Open") { actions.openMeeting(meeting.id) }.buttonStyle(.whimbrelQuiet)
        }
    }
}

/// The pitch markings: mowing stripes, the touchlines and halfway line, the centre circle and both
/// penalty boxes, in the palette's accent and border. Drawn, never a bitmap.
private struct PitchLines: View {
    var body: some View {
        Canvas { context, size in
            let stripes = 10, bandWidth = size.width / CGFloat(stripes)
            for i in stride(from: 0, to: stripes, by: 2) {
                context.fill(Path(CGRect(x: CGFloat(i) * bandWidth, y: 0, width: bandWidth, height: size.height)), with: .color(Color.whimbrelAccentPrimary.opacity(FieldStyle.stripeOpacity)))
            }
            let chalk = GraphicsContext.Shading.color(Color.whimbrelBorder)
            let inset: CGFloat = 6
            let field = CGRect(x: inset, y: inset, width: size.width - inset * 2, height: size.height - inset * 2)
            let boxHeight = field.height * 0.62
            var lines = Path()
            lines.addRect(field)
            lines.move(to: CGPoint(x: field.midX, y: field.minY)); lines.addLine(to: CGPoint(x: field.midX, y: field.maxY))
            let radius = min(field.height * 0.28, 34)
            lines.addEllipse(in: CGRect(x: field.midX - radius, y: field.midY - radius, width: radius * 2, height: radius * 2))
            lines.addRect(CGRect(x: field.minX, y: field.midY - boxHeight / 2, width: 56, height: boxHeight))
            lines.addRect(CGRect(x: field.maxX - 56, y: field.midY - boxHeight / 2, width: 56, height: boxHeight))
            context.opacity = FieldStyle.lineOpacity
            context.stroke(lines, with: chalk, lineWidth: 1.5)
        }
    }
}

/// Diagonal stripes in the accent colour, for the header band.
struct DiagonalStripes: View {
    var body: some View {
        Canvas { context, size in
            var path = Path()
            for x in stride(from: -size.height, to: size.width, by: 10) {
                path.move(to: CGPoint(x: x, y: size.height)); path.addLine(to: CGPoint(x: x + size.height, y: 0))
            }
            context.stroke(path, with: .color(Color.whimbrelAccentPrimary), lineWidth: 3)
        }
    }
}

/// A horizontal line across its rect, for a chalk stroke.
private struct ChalkRule: Shape {
    func path(in rect: CGRect) -> Path {
        Path { $0.move(to: CGPoint(x: rect.minX, y: rect.midY)); $0.addLine(to: CGPoint(x: rect.maxX, y: rect.midY)) }
    }
}
