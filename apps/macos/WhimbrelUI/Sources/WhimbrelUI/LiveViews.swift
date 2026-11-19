import SwiftUI

// The live meeting screen and the in-meeting window. The screen takes Home's place inside the shell while a
// meeting records. The shell's own recording bar is not drawn on it: the bar is the first thing in the main
// column. The notes editor, the alerts and the ask field are slots, because the app already has them.

// MARK: The short time line

struct WLiveTimeline: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let stops: [WLiveStop]
    /// While paused nothing is recording, so the start of the recording is not drawn as the red recording dot.
    var paused = false

    var body: some View {
        WSpine {
            ForEach(Array(stops.enumerated()), id: \.element.id) { index, stop in
                WSpineStop(
                    time: stop.time, caption: stop.caption, node: node(stop.kind),
                    connector: connector(stop.kind, index: index), connectsFromAbove: index > 0
                ) {
                    Color.clear.frame(height: stop.kind == .next ? 40 * k : (stop.kind == .now ? 140 * k : 10 * k))
                }
            }
        }
        .padding(.top, theme.space(4))
    }

    private func node(_ kind: WLiveStopKind) -> WSpineNode {
        switch kind {
        case .next: return .upcoming
        case .now: return .now
        case .started: return paused ? .past : .recording
        }
    }

    /// The line above a stop: dashed to the next meeting, then red from now back to the start.
    private func connector(_ kind: WLiveStopKind, index: Int) -> WSpineConnector {
        switch kind {
        case .next: return .none
        case .now: return .dashed
        case .started: return paused ? .solid : .recording
        }
    }
}

// MARK: The screen

public struct WLiveMeetingView<Warnings: View, Editor: View, Alerts: View, Ask: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.wScrollsContent) private var scrolls
    let model: WLiveMeetingModel
    let actions: WLiveActions
    let warnings: Warnings
    let editor: Editor
    let alerts: Alerts
    let ask: Ask

    public init(
        model: WLiveMeetingModel, actions: WLiveActions,
        @ViewBuilder warnings: () -> Warnings, @ViewBuilder editor: () -> Editor,
        @ViewBuilder alerts: () -> Alerts, @ViewBuilder ask: () -> Ask
    ) {
        self.model = model
        self.actions = actions
        self.warnings = warnings()
        self.editor = editor()
        self.alerts = alerts()
        self.ask = ask()
    }

    /// The three columns need room: the time line, a main column that holds the bar and the note card, and the
    /// transcript. With less room the transcript goes under the main column, and with less still the time line goes.
    enum Arrangement: Equatable { case three, stacked, stackedWithoutTimeline }

    func arrangement(width: CGFloat) -> Arrangement {
        let gap = theme.space(16)
        let timeline = model.stops.isEmpty ? 0 : WTokens.Size.spine * k + gap
        let mainMin = 440 * k
        let side = model.showsTranscript ? 320 * k + gap : 0
        if width >= timeline + mainMin + side { return .three }
        if width >= timeline + mainMin { return .stacked }
        return .stackedWithoutTimeline
    }

    public var body: some View {
        GeometryReader { geo in
            let layout = arrangement(width: geo.size.width - theme.space(40))
            HStack(alignment: .top, spacing: theme.space(16)) {
                if !model.stops.isEmpty, layout != .stackedWithoutTimeline {
                    WLiveTimeline(stops: model.stops, paused: model.state == .paused || model.state == .resuming).frame(width: WTokens.Size.spine * k)
                }
                if layout == .three {
                    column(main)
                    if model.showsTranscript { column(side).frame(width: 320 * k) }
                } else {
                    column(VStack(alignment: .leading, spacing: theme.space(12)) {
                        main
                        if model.showsTranscript { side }
                    })
                }
            }
            .padding(.horizontal, theme.space(20))
            .padding(.bottom, theme.space(16))
            .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func column<C: View>(_ content: C) -> some View {
        if scrolls {
            ScrollView { content.frame(maxWidth: .infinity, alignment: .topLeading) }.scrollIndicators(.automatic)
        } else {
            content.frame(maxWidth: .infinity, alignment: .topLeading)
        }
    }

    // MARK: Main column

    private var main: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            if let bar = model.state.barState {
                VStack(alignment: .leading, spacing: theme.space(4)) {
                    WRecordingIndicator(
                        state: bar, elapsedSeconds: model.recordedSeconds, size: .bar, level: model.level,
                        onPause: actions.pause, onResume: actions.resume, onStop: actions.stop)
                    if case .processing(let stage?) = model.state {
                        Text(stage).wFont(.secondary).foregroundStyle(theme.textSecondary).padding(.horizontal, theme.space(14))
                    }
                }
            }
            if model.noticeVisible {
                WConsentBanner(isCopied: model.noticeCopied, onCopy: actions.copyNotice, onDone: actions.dismissNotice)
            }
            warnings
            noteCard
        }
        .padding(.top, theme.space(4))
    }

    private var noteCard: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            HStack(alignment: .top, spacing: theme.space(12)) {
                VStack(alignment: .leading, spacing: theme.space(4)) {
                    Text(model.title).wFont(.pageTitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                    if !model.detail.isEmpty {
                        Text(model.detail).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    }
                }
                Spacer(minLength: theme.space(8))
                if model.state != .readOnly {
                    WButton(strings.live.markThisMoment, glyph: "flag", shortcutHint: "\u{21E7}\u{2318}M", kind: .secondary, action: actions.mark)
                        .fixedSize()
                        .disabled(!model.state.allowsMarking)
                }
            }
            editor
                .frame(minHeight: 120 * k, alignment: .topLeading)
                .accessibilityLabel(strings.live.notes)
            if !model.marks.isEmpty { marks }
            Spacer(minLength: 0)
            footer
        }
        .padding(theme.space(20))
        .frame(maxWidth: .infinity, minHeight: 360 * k, alignment: .topLeading)
        .wCard(radius: .surface)
    }

    private var marks: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            HStack(spacing: theme.space(8)) {
                WGlyph(name: "flag", size: 13, weight: .semibold).foregroundStyle(theme.textSecondary)
                Text(strings.live.momentsYouMarked).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                Text(strings.live.count(model.marks.count)).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
            ForEach(model.marks) { mark in
                HStack(spacing: theme.space(10)) {
                    Text(mark.timeOfDay).wFont(.time).foregroundStyle(theme.accentPrimary)
                        .padding(.horizontal, theme.space(10)).frame(minHeight: 24 * k)
                        .background(Capsule().fill(theme.backgroundCard))
                        .overlay(Capsule().strokeBorder(theme.accentPrimary, lineWidth: theme.isHighContrast ? 1.5 : 1))
                    Text(mark.label).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(2)
                    Spacer(minLength: 0)
                    if model.state != .readOnly {
                        Button { actions.removeMark(mark.id) } label: {
                            WGlyph(name: "xmark", size: 11, weight: .semibold).foregroundStyle(theme.textSecondary)
                                .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(strings.live.removeMomentAt(mark.timeOfDay))
                    }
                }
                .padding(.horizontal, theme.space(10)).padding(.vertical, theme.space(4))
                .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.selection.opacity(0.45)))
                .accessibilityElement(children: .contain)
                .accessibilityLabel(strings.joinDetails([strings.live.momentAt(mark.timeOfDay), mark.label]))
            }
        }
    }

    private var footer: some View {
        HStack(spacing: theme.space(14)) {
            HStack(spacing: theme.space(6)) {
                WGlyph(name: "mic", size: 11)
                Text(strings.live.audioStaysOnMac).wFont(.secondary)
            }
            HStack(spacing: theme.space(6)) {
                WGlyph(name: "sparkles", size: 11)
                Text(strings.live.tidiesNotes).wFont(.secondary)
            }
            Spacer(minLength: 0)
        }
        .foregroundStyle(theme.textSecondary)
        .frame(maxWidth: .infinity, alignment: .center)
    }

    // MARK: Right column

    private var side: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            WTranscriptView(accessory: model.transcriptAccessory, entries: model.transcript)
            alerts
            ask
        }
        .padding(.top, theme.space(4))
    }
}

// MARK: The in-meeting window

/// The content of the small always-on-top window. Wide enough to hold Pause and Stop at 200 percent text.
public struct WMeetingBar<Ask: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let model: WMeetingBarModel
    let actions: WLiveActions
    let ask: Ask
    let onClose: (() -> Void)?

    public static var width: CGFloat { 300 }

    public init(model: WMeetingBarModel, actions: WLiveActions, onClose: (() -> Void)? = nil, @ViewBuilder ask: () -> Ask) {
        self.model = model
        self.actions = actions
        self.onClose = onClose
        self.ask = ask()
    }

    private var state: WRecordingState { model.state.barState ?? .off }
    private var live: Bool { model.state == .recording }

    private var ink: Color {
        switch state {
        case .recording: return theme.recording
        case .problem: return theme.error
        case .paused, .processing, .starting, .resuming: return theme.textPrimary
        case .off: return theme.textSecondary
        }
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            HStack(spacing: theme.space(6)) {
                Text(model.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).lineLimit(1)
                Spacer(minLength: 0)
                if let onClose {
                    Button(action: onClose) {
                        WGlyph(name: "xmark", size: 10, weight: .semibold).foregroundStyle(theme.textSecondary)
                            .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(strings.live.closeWindow)
                }
            }
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                WGlyph(name: state.glyph, size: 14, weight: .semibold).foregroundStyle(ink)
                Text(state.word(using: strings)).wFont(.subtitle).foregroundStyle(ink).lineLimit(1)
                Spacer(minLength: theme.space(6))
                if showsTime {
                    Text(WRecordingText.clock(seconds: model.recordedSeconds)).wFont(.pageTitle).foregroundStyle(theme.textPrimary)
                        .monospacedDigit()
                        .accessibilityLabel(WRecordingText.spoken(seconds: model.recordedSeconds, strings: strings))
                }
            }
            .accessibilityElement(children: .combine)
            if case .problem(let reason) = model.state {
                Text(reason).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            if live || model.state == .paused || model.state == .resuming {
                WLevelMeter(level: model.level, color: live ? theme.recording : theme.textSecondary)
            }
            controls
            ask
            HStack(spacing: theme.space(6)) {
                WGlyph(name: "mic", size: 11)
                Text(strings.live.audioStaysOnMac).wFont(.secondary)
            }
            .foregroundStyle(theme.textSecondary)
        }
        .padding(theme.space(14))
        .frame(width: Self.width * min(k, 1.6), alignment: .leading)
        .wCard(radius: .surface, elevation: WTokens.Elevation.level2)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.live.meetingWindowName)
    }

    private var showsTime: Bool {
        switch model.state {
        case .recording, .paused, .resuming: return true
        default: return false
        }
    }

    @ViewBuilder
    private var controls: some View {
        HStack(spacing: theme.space(8)) {
            switch model.state {
            case .recording:
                WButton(strings.pause, glyph: "pause.fill", kind: .secondary, action: actions.pause)
            case .paused:
                WButton(strings.resume, glyph: "play.fill", kind: .secondary, action: actions.resume)
            case .resuming:
                WButton(strings.resume, glyph: "play.fill", kind: .secondary, action: actions.resume).disabled(true)
            case .starting:
                WButton(strings.pause, glyph: "pause.fill", kind: .secondary, action: actions.pause).disabled(true)
            default:
                EmptyView()
            }
            switch model.state {
            case .recording, .paused, .resuming, .problem, .starting:
                WButton(strings.stop, glyph: "stop.fill", kind: .primary, action: actions.stop).disabled(model.state == .starting)
            default:
                EmptyView()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
