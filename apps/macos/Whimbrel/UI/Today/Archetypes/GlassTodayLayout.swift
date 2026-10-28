import SwiftUI

/// The numbers behind the glass look, shared with the contrast test.
enum GlassStyle {
    /// How much of `backgroundCard` covers the material. `.ultraThinMaterial` shows what is behind
    /// it, so the tint is mostly opaque: text contrast is then set by the palette, not the backdrop.
    static let cardOpacity = 0.9
    /// Strength of `accentPrimary` at the top of the gradient (it fades to `backgroundPage`).
    static let glowOpacity = 0.3

    /// `fg` at `alpha` over opaque `bg`, as 0xRRGGBB.
    static func composite(_ fg: UInt32, alpha: Double, over bg: UInt32) -> UInt32 {
        func channel(_ shift: UInt32) -> UInt32 {
            UInt32((Double((fg >> shift) & 0xFF) * alpha + Double((bg >> shift) & 0xFF) * (1 - alpha)).rounded())
        }
        return channel(16) << 16 | channel(8) << 8 | channel(0)
    }

    /// The glow's colours, top to bottom in tenths, over opaque `under` (the page, or a scrimmed background
    /// image): `accent` at `glowOpacity` fading to opaque `page`, interpolated unpremultiplied (its middle
    /// carries a little more accent than its top).
    static func glow(accent: UInt32, page: UInt32, over under: UInt32) -> [UInt32] {
        (0...10).map { step in
            let t = Double(step) / 10
            let colour = composite(page, alpha: t, over: accent)
            return composite(colour, alpha: glowOpacity + (1 - glowOpacity) * t, over: under)
        }
    }
}

/// Translucent cards (`.ultraThinMaterial` under a `backgroundCard` tint) over a gradient from
/// `accentPrimary` to `backgroundPage`. A big thin greeting, a pill-framed Ask, then Up next,
/// Later today and the recap as cards. Places greeting, composer, upNext, schedule and recap; the
/// rest (earlier, threads, devices) is `TodayChrome`'s leftovers list.
struct GlassTodayLayout: TodayArchetypeView {
    let content: TodayContent
    let actions: TodayActions
    let composer: AnyView?

    init(content: TodayContent, actions: TodayActions, composer: AnyView?) {
        self.content = content
        self.actions = actions
        self.composer = composer
    }

    static var placedSections: Set<TodaySection> { [.greeting, .composer, .upNext, .schedule, .recap] }

    private var skin: Skin { SkinStore.shared.selected }
    private var voice: SkinVoice { skin.voice }
    private let timeWidth: CGFloat = 72

    var body: some View {
        let sections = content.nonEmptySections()
        VStack(alignment: .leading, spacing: Spacing.lg) {
            header.skinBadge()
            if sections.contains(.composer), let composer { pill(composer) }
            if sections.contains(.upNext), let next = TodayRows.upNext(content) {
                card(voice.text(.upNextHeader, name: "")) { eventRow(next) }
            }
            if sections.contains(.schedule) {
                // `.upNext` is placed above and shows `upcomingToday.first`, so the schedule lists the rest.
                let schedule = TodayRows.schedule(content, upNextPlaced: true)
                if !schedule.upcoming.isEmpty || !schedule.held.isEmpty {
                    card(voice.text(.upcomingHeader, name: "")) {
                        ForEach(TodayRows.scheduleItems(schedule)) { item in
                            switch item {
                            case let .event(event): eventRow(event)
                            case .separator: SkinText(TodayRows.heldSeparator, role: .label).foregroundStyle(Color.whimbrelTextSecondary)
                            case let .held(meeting): meetingRow(meeting)
                            }
                        }
                    }
                }
            }
            if sections.contains(.recap) {
                card(voice.text(.recapHeader, name: "")) { recap }
            }
        }
        .padding(Spacing.lg)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            LinearGradient(
                colors: [Color.whimbrelAccentPrimary.opacity(GlassStyle.glowOpacity), Color.whimbrelBackgroundPage],
                startPoint: .top, endPoint: .bottom
            )
            .clipShape(RoundedRectangle(cornerRadius: Radius.card + Spacing.sm))
        )
    }

    // MARK: pieces

    /// A thin display greeting; with no usable profile name the date is the title.
    private var header: some View {
        let dateLine = content.now.formatted(.dateTime.weekday(.wide).month(.wide).day())
        let greeting = TodayLogic.greeting(name: content.name, now: content.now, calendar: .current, voice: voice)
        return VStack(alignment: .leading, spacing: Spacing.xs) {
            if greeting != nil {
                SkinText(dateLine, role: .label, size: 13).foregroundStyle(Color.whimbrelTextSecondary)
            }
            SkinText(greeting ?? dateLine, role: .display, lines: 3, size: 44).foregroundStyle(Color.whimbrelTextPrimary)
        }
        .padding(.vertical, Spacing.md)
    }

    /// The real composer, as is: its own card at the skin's radius (24) is the pill.
    private func pill(_ composer: AnyView) -> some View {
        composer

    }

    private func card<C: View>(_ title: String, @ViewBuilder _ rows: () -> C) -> some View {
        let shape = RoundedRectangle(cornerRadius: Radius.card)
        return VStack(alignment: .leading, spacing: Spacing.md) {
            SkinText(title, role: .label, size: 12).foregroundStyle(Color.whimbrelTextSecondary)
            rows()
        }
        .padding(Spacing.lg * skin.shape.density.scale)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background { glass(shape) }
        .clipShape(shape)
        .overlay(shape.stroke(Color.whimbrelBorder, lineWidth: skin.shape.borderWidth))
    }

    /// Material first, then the palette tint on top: see `GlassStyle.cardOpacity`.
    private func glass<S: Shape>(_ shape: S) -> some View {
        ZStack {
            shape.fill(.ultraThinMaterial)
            shape.fill(Color.whimbrelBackgroundCard.opacity(GlassStyle.cardOpacity))
        }
    }

    private func eventRow(_ event: TodayEventRowModel) -> some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            SkinText(event.timeText, role: .numerals)
                .foregroundStyle(Color.whimbrelTextSecondary)
                .frame(width: timeWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                SkinText(event.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
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

    private func meetingRow(_ meeting: TodayMeetingRowModel) -> some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            SkinText(meeting.timeText, role: .numerals)
                .foregroundStyle(Color.whimbrelTextSecondary)
                .frame(width: timeWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                SkinText(meeting.title, lines: 2).foregroundStyle(Color.whimbrelTextPrimary)
                if !meeting.subtitle.isEmpty { SkinText(meeting.subtitle, role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button("Open") { actions.openMeeting(meeting.id) }.buttonStyle(.whimbrelQuiet)
        }
    }

    @ViewBuilder
    private var recap: some View {
        switch TodayRows.recap(content) {
        case let .brief(text, origin):
            SkinText(text, lines: .max).foregroundStyle(Color.whimbrelTextPrimary)
            if let origin { SkinText("From \(origin)", role: .label).foregroundStyle(Color.whimbrelTextSecondary) }
        case let .digest(text):
            SkinText(text, lines: .max).foregroundStyle(Color.whimbrelTextPrimary)
        case let .failed(message, retryEnabled):
            SkinText(message, lines: 6).foregroundStyle(Color.whimbrelError)
            Button("Try again") { actions.generateDigest() }
                .buttonStyle(.whimbrelSecondary)
                .disabled(!retryEnabled)
        case .none:
            EmptyView()
        }
    }

}
