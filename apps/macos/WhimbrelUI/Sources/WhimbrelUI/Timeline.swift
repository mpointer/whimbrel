import SwiftUI

// MARK: Meeting card

public enum WMeetingCardVariant: Sendable { case next, upcoming, past }

/// A meeting on the time line. Variants: next (large), upcoming (row), past (row with a one line gist).
public struct WMeetingCard: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let variant: WMeetingCardVariant
    let time: String
    let title: String
    let attendees: [WPerson]
    let eyebrow: String?
    let gist: String?
    let primaryTitle: String?
    let primaryGlyph: String?
    let primaryHint: String?
    let secondaryTitle: String?
    let onPrimary: (() -> Void)?
    let onSecondary: (() -> Void)?

    public init(
        variant: WMeetingCardVariant,
        time: String,
        title: String,
        attendees: [WPerson],
        eyebrow: String? = nil,
        gist: String? = nil,
        primaryTitle: String? = nil,
        primaryGlyph: String? = nil,
        primaryHint: String? = nil,
        onPrimary: (() -> Void)? = nil,
        secondaryTitle: String? = nil,
        onSecondary: (() -> Void)? = nil
    ) {
        self.variant = variant
        self.time = time
        self.title = title
        self.attendees = attendees
        self.eyebrow = eyebrow
        self.gist = gist
        self.primaryTitle = primaryTitle
        self.primaryGlyph = primaryGlyph
        self.primaryHint = primaryHint
        self.onPrimary = onPrimary
        self.secondaryTitle = secondaryTitle
        self.onSecondary = onSecondary
    }

    public var body: some View {
        Group {
            switch variant {
            case .next: next
            case .upcoming, .past: row
            }
        }
        .accessibilityElement(children: .contain)
    }

    private func eyebrowChip(_ text: String) -> some View {
        Text(text)
            .wFont(.secondary)
            .fontWeight(.semibold)
            .foregroundStyle(theme.accentPrimary)
            .padding(.horizontal, theme.space(8))
            .padding(.vertical, theme.space(2))
            .background(Capsule().fill(theme.selection))
            .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(theme.accentPrimary, style: theme.borderStroke.widened(theme.isHighContrast ? 1 : 0)).opacity(theme.isHighContrast ? 1 : 0))
    }

    private var chips: some View {
        WFlowLayout(spacing: theme.space(6), lineSpacing: theme.space(6)) {
            ForEach(attendees) { WSpeakerChip($0, size: variant == .next ? .regular : .small) }
        }
    }

    private var next: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            if let eyebrow { eyebrowChip(eyebrow) }
            Text(title).wFont(.title).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
            Text(time).wFont(.body).foregroundStyle(theme.textSecondary)
            chips
            HStack(spacing: theme.space(10)) {
                Spacer(minLength: 0)
                if let secondaryTitle, let onSecondary {
                    WAction(secondaryTitle, glyph: "doc", kind: .secondary, action: onSecondary)
                }
                if let primaryTitle, let onPrimary {
                    WAction(primaryTitle, glyph: primaryGlyph, shortcutHint: primaryHint, kind: .primary, action: onPrimary)
                }
            }
        }
        .padding(theme.space(16))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard(radius: .card, elevation: WTokens.Elevation.level2)
    }

    private var row: some View {
        HStack(alignment: .center, spacing: theme.space(12)) {
            VStack(alignment: .leading, spacing: theme.space(6)) {
                HStack(spacing: theme.space(8)) {
                    Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                    if variant == .upcoming { Text(time).wFont(.secondary).foregroundStyle(theme.textSecondary) }
                    if let eyebrow { eyebrowChip(eyebrow) }
                }
                if variant == .past, let gist {
                    Text(gist).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                }
                if !attendees.isEmpty { chips }
            }
            Spacer(minLength: theme.space(8))
            if let primaryTitle, let onPrimary {
                WAction(primaryTitle, kind: .quiet, action: onPrimary)
            }
        }
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(10))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard(radius: .card)
    }
}

// MARK: Memory block

public struct WMemoryItem: Identifiable, Equatable, Sendable {
    public let id: String
    public let app: WAppKind
    public let glyph: String
    public let name: String
    public init(app: WAppKind, glyph: String, name: String) {
        self.id = "\(app.rawValue)-\(glyph)-\(name)"
        self.app = app
        self.glyph = glyph
        self.name = name
    }
}

/// A time range and the apps and items seen in it.
public struct WMemoryBlock: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let timeRange: String
    let customLabel: String?
    let items: [WMemoryItem]

    private var label: String { customLabel ?? strings.memory }

    public init(timeRange: String, label: String? = nil, items: [WMemoryItem]) {
        self.timeRange = timeRange
        self.customLabel = label
        self.items = items
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            HStack(spacing: theme.space(8)) {
                Text(timeRange).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                Text(label).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
            WFlowLayout(spacing: theme.space(14), lineSpacing: theme.space(4)) {
                ForEach(items) { item in
                    HStack(spacing: theme.space(6)) {
                        Circle().fill(theme.appColor(item.app)).frame(width: 7, height: 7).accessibilityHidden(true)
                        WGlyph(name: item.glyph, size: 12).foregroundStyle(theme.textSecondary)
                        Text(item.name).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(1)
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(item.name)
                }
            }
        }
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(10))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard(radius: .card)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.joinDetails([label, timeRange]))
    }
}

// MARK: Transcript

public enum WTranscriptLineState: Sendable { case normal, cited, live, paused, resuming }

public struct WTranscriptLine: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let time: String
    let speaker: WPerson
    let text: String
    let state: WTranscriptLineState

    public init(time: String, speaker: WPerson, text: String, state: WTranscriptLineState = .normal) {
        self.time = time
        self.speaker = speaker
        self.text = text
        self.state = state
    }

    public var body: some View {
        HStack(alignment: .top, spacing: theme.space(10)) {
            Text(time).wFont(.time).fontWeight(.regular).foregroundStyle(theme.textSecondary)
                .frame(width: 46 * k, alignment: .leading)
            VStack(alignment: .leading, spacing: theme.space(3)) {
                HStack(spacing: theme.space(6)) {
                    Text(speaker.initials).wFont(.caption).fontWeight(.bold)
                        .foregroundStyle(theme.textPrimary)
                        .frame(width: 20 * k, height: 20 * k)
                        .background(Circle().fill(theme.border.opacity(theme.isHighContrast ? 0 : 0.55)))
                        .overlay(Circle().strokeBorder(theme.border, lineWidth: theme.isHighContrast ? 1 : 0))
                    Text(speaker.name).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                    if state == .cited {
                        Text(strings.cited).wFont(.caption).fontWeight(.semibold).foregroundStyle(theme.accentPrimary)
                    }
                }
                Text(text).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                if state == .live {
                    HStack(spacing: theme.space(4)) {
                        WGlyph(name: "waveform", size: 11, weight: .semibold)
                        Text(strings.listening).wFont(.caption).fontWeight(.semibold)
                    }
                    .foregroundStyle(theme.accentPrimary)
                } else if state == .paused || state == .resuming {
                    // A still mark: nothing moves while the recording is paused.
                    HStack(spacing: theme.space(4)) {
                        WGlyph(name: state == .paused ? "pause.fill" : "circle.dotted", size: 11, weight: .semibold)
                        Text(state == .paused ? strings.paused : strings.resuming).wFont(.caption).fontWeight(.semibold)
                    }
                    .foregroundStyle(theme.textSecondary)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, theme.space(10))
        .padding(.vertical, theme.space(8))
        .background(
            RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
                .fill(state == .cited ? theme.highlight : .clear)
        )
        .overlay {
            if state == .cited {
                RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
                    .strokeBorder(theme.accentPrimary, lineWidth: theme.isHighContrast ? 1.5 : 0)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(strings.joinDetails([time, speaker.name, text] + (state == .live ? [strings.spokenListening] : []) + (state == .paused ? [strings.paused] : []) + (state == .resuming ? [strings.resuming] : []) + (state == .cited ? [strings.spokenCited] : [])))
    }
}

public struct WTranscriptEntry: Identifiable, Sendable {
    public let id: String
    public let time: String
    public let speaker: WPerson
    public let text: String
    public let state: WTranscriptLineState
    /// `id` must be unique in a list. Pass the stored line's id: the shown time is only to the minute, so time and speaker
    /// alone repeat for every line one person says within a minute, and SwiftUI then draws the first such line in each row.
    public init(id: String? = nil, time: String, speaker: WPerson, text: String, state: WTranscriptLineState = .normal) {
        self.id = id ?? "\(time)-\(speaker.name)-\(text)"
        self.time = time
        self.speaker = speaker
        self.text = text
        self.state = state
    }
}

/// The transcript card: a title, an optional note on the right, and the lines.
public struct WTranscriptView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wTranscriptViewport) private var viewport
    @State private var follow = WTranscriptFollow()
    private static let bottomId = "wTranscriptBottom"
    let customTitle: String?
    let accessory: String?
    let entries: [WTranscriptEntry]
    /// An entry to show as cited and scroll to (a live-alert jump, or a line the ask bar cited). Nil shows none.
    let highlightedId: String?

    private var title: String { customTitle ?? strings.liveTranscript }

    public init(title: String? = nil, accessory: String? = nil, entries: [WTranscriptEntry], highlightedId: String? = nil) {
        self.customTitle = title
        self.accessory = accessory
        self.entries = entries
        self.highlightedId = highlightedId
    }

    /// An entry shown as cited when it is the highlighted one, whatever it said before.
    static func shownState(_ entry: WTranscriptEntry, highlightedId: String?) -> WTranscriptLineState {
        entry.id == highlightedId ? .cited : entry.state
    }

    public var body: some View {
        ScrollViewReader { proxy in
            card
                .onChange(of: highlightedId) { _, id in
                    follow.highlight(active: id != nil)
                    scroll(to: id, proxy)
                }
                .onAppear {
                    follow.highlight(active: highlightedId != nil)
                    scroll(to: highlightedId, proxy)
                }
                .onChange(of: entries.last?.id) { _, _ in
                    // Only inside a scroll view. Only the scroll position changes; accessibility focus is not moved.
                    guard viewport > 0, follow.newLine() else { return }
                    if reduceMotion { proxy.scrollTo(Self.bottomId, anchor: .bottom) } else { withAnimation { proxy.scrollTo(Self.bottomId, anchor: .bottom) } }
                }
                .onPreferenceChange(WTranscriptBottomKey.self) { top in
                    if let top, viewport > 0 { follow.position(distanceBelow: top - viewport) }
                }
        }
    }

    private func scroll(to id: String?, _ proxy: ScrollViewProxy) {
        guard let id, entries.contains(where: { $0.id == id }) else { return }
        if reduceMotion { proxy.scrollTo(id, anchor: .center) } else { withAnimation { proxy.scrollTo(id, anchor: .center) } }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: theme.space(6)) {
                WGlyph(name: "waveform", size: 13, weight: .semibold).foregroundStyle(theme.textSecondary)
                Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                Spacer(minLength: theme.space(8))
                if let accessory { Text(accessory).wFont(.secondary).foregroundStyle(theme.textSecondary) }
            }
            .padding(.horizontal, theme.space(14))
            .padding(.vertical, theme.space(10))
            ForEach(entries) { entry in
                Divider().overlay(theme.border.opacity(0.6))
                WTranscriptLine(time: entry.time, speaker: entry.speaker, text: entry.text, state: Self.shownState(entry, highlightedId: highlightedId))
                    .padding(.horizontal, theme.space(4))
                    .id(entry.id)
            }
            // The end of the transcript: where "at the bottom" is measured, and what a new line scrolls to.
            Color.clear.frame(height: 1).id(Self.bottomId).accessibilityHidden(true)
                .background(GeometryReader { geo in
                    Color.clear.preference(key: WTranscriptBottomKey.self, value: geo.frame(in: .named(WTranscriptScrollSpace.name)).minY)
                })
        }
        .padding(.bottom, theme.space(6))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard(radius: .card)
        .accessibilityElement(children: .contain)
    }
}
