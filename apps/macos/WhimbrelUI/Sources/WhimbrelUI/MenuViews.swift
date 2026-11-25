import SwiftUI

// The menu bar menu (docs/ui-redesign/screens/menu-bar.md). One surface with sections and dividers, in this order:
// Recording (only while a meeting records), Memory, Needs attention (only when there is something), Next meeting,
// Start Meeting and Ask about this window, Working on, then Open Whimbrel, Settings and Quit.
// Recording and Memory are never hidden, shrunk or shown by color alone. The Recording and Memory rows stay at the top and
// Open, Settings and Quit at the bottom; when the menu is taller than the screen the part between them scrolls.

// MARK: Parts

extension View {
    /// A control the arrow keys can reach: it takes focus by its id, and the menu can scroll to it.
    fileprivate func menuControl(_ id: String, _ focus: FocusState<String?>.Binding) -> some View {
        self.focused(focus, equals: id).id(id)
    }
}

/// A button with a visible title and a separate spoken name. `WButton` sets its own label from the title, so a
/// button that needs "Pause recording" for a screen reader draws its own.
struct WSpokenButton: View {
    @Environment(\.wTheme) private var theme
    let title: String
    var spokenName: String?
    var glyph: String?
    var kind: WButtonKind = .secondary
    var help: String?
    /// Opens a web page: a screen reader says "link".
    var isLink = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: theme.space(6)) {
                if let glyph { WGlyph(name: glyph, size: 12, weight: .semibold) }
                Text(title).lineLimit(1)
            }
        }
        .buttonStyle(WButtonStyle(kind: kind, firm: true))
        .accessibilityLabel(spokenName ?? title)
        .accessibilityAddTraits(isLink ? .isLink : [])
        .accessibilityHint(help ?? "")
        .help(help ?? "")
    }
}

/// A full-width row that acts: a glyph, the words, a hover fill. The whole row is the target.
private struct WMenuRowStyle: ButtonStyle {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        WMenuRowBody(configuration: configuration, enabled: isEnabled)
    }
}

private struct WMenuRowBody: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var hovering = false
    let configuration: ButtonStyleConfiguration
    let enabled: Bool

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        configuration.label
            .padding(.horizontal, theme.space(8))
            .frame(maxWidth: .infinity, minHeight: WTokens.Size.target * k, alignment: .leading)
            .background(shape.fill(configuration.isPressed ? theme.selection : (hovering && enabled ? theme.selection.opacity(0.6) : .clear)))
            .overlay {
                if hovering && enabled && theme.isHighContrast { shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth) }
            }
            .opacity(enabled ? 1 : 0.45)
            .contentShape(shape)
            .onHover { hovering = $0 }
    }
}

/// One action row of the menu.
struct WMenuRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let glyph: String
    let title: String
    var spokenName: String?
    var hint: String?
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: theme.space(10)) {
                WGlyph(name: glyph, size: 13, weight: .medium).foregroundStyle(theme.textSecondary).frame(width: 18 * k)
                Text(title).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(2).fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: theme.space(6))
                if let hint { WKeycap(text: hint) }
            }
        }
        .buttonStyle(WMenuRowStyle())
        .accessibilityLabel(spokenName ?? title)
    }
}

/// A section: an optional label above, the rows below, the same inset on every section.
struct WMenuSection<Content: View>: View {
    @Environment(\.wTheme) private var theme
    let label: String?
    let content: Content

    init(label: String? = nil, @ViewBuilder content: () -> Content) {
        self.label = label
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            if let label {
                Text(label).wFont(.caption).foregroundStyle(theme.textSecondary)
                    .padding(.horizontal, theme.space(8))
                    .accessibilityAddTraits(.isHeader)
            }
            content
        }
        .padding(.horizontal, theme.space(6))
        .padding(.vertical, theme.space(6))
    }
}

// MARK: Height

/// Three parts in a column: the top and the footer at their own height, the middle at its own height or, when the whole is
/// taller than `cap`, at what is left. Pure layout, so the menu fits the screen at any text size.
private struct WMenuColumn: Layout {
    var cap: CGFloat?

    private func sizes(_ subviews: Subviews, width: CGFloat?) -> [CGSize] {
        subviews.map { $0.sizeThatFits(ProposedViewSize(width: width, height: nil)) }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard subviews.count == 3 else { return .zero }
        let parts = sizes(subviews, width: proposal.width)
        let width = proposal.width ?? parts.map(\.width).max() ?? 0
        let middle = WMenuLayout.middleHeight(ideal: parts[1].height, pinnedTop: parts[0].height, footer: parts[2].height, cap: cap)
        return CGSize(width: width, height: parts[0].height + middle + parts[2].height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 3 else { return }
        let parts = sizes(subviews, width: bounds.width)
        let middle = WMenuLayout.middleHeight(ideal: parts[1].height, pinnedTop: parts[0].height, footer: parts[2].height, cap: cap)
        var y = bounds.minY
        for (index, height) in [parts[0].height, middle, parts[2].height].enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX, y: y), anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: height))
            y += height
        }
    }
}

// MARK: The scroll cue

struct WMenuContentMetrics: Equatable {
    var height: CGFloat
    var offset: CGFloat
}

/// Reports one measure of a view to a closure when it first shows and whenever it changes. Not a preference: a preference's
/// change is delivered on a later pass that a view shown off screen never gets, and this must also read right in the gallery.
private struct WMenuMeasure: View {
    var value: (GeometryProxy) -> CGSize
    var report: (CGSize) -> Void

    var body: some View {
        GeometryReader { proxy in
            let size = value(proxy)
            Color.clear
                .onAppear { report(size) }
                .onChange(of: size) { _, new in report(new) }
        }
    }
}

/// A chevron drawn as a shape, pointing down (or up).
struct WMenuChevron: Shape {
    var up = false
    func path(in rect: CGRect) -> Path {
        var path = Path()
        let (tip, side) = up ? (rect.minY, rect.maxY) : (rect.maxY, rect.minY)
        path.move(to: CGPoint(x: rect.minX, y: side))
        path.addLine(to: CGPoint(x: rect.midX, y: tip))
        path.addLine(to: CGPoint(x: rect.maxX, y: side))
        return path
    }
}

// MARK: The menu

public struct WMenuView<Accessory: View, Extras: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.controlActiveState) private var activeState
    @FocusState private var focus: String?
    @State private var content = WMenuContentMetrics(height: 0, offset: 0)
    @State private var viewport: CGFloat = 0
    let model: WMenuModel
    let actions: WMenuActions
    let memoryGate: (AnyView) -> AnyView
    let startGate: (AnyView) -> AnyView
    let accessory: (String) -> Accessory
    let extras: Extras
    /// The tallest the menu may be, the visible height of the screen. Nil leaves it as tall as its content.
    let maxHeight: CGFloat?

    /// `memoryGate` and `startGate` wrap the memory control and the Start Meeting row, so the app can switch them off
    /// without a store (the controls that start capture or recording). `accessory` draws extra controls under a need,
    /// by its id (the app's Sync now). `extras` goes above Quit (developer-only rows).
    public init(
        model: WMenuModel, actions: WMenuActions, maxHeight: CGFloat? = nil,
        memoryGate: @escaping (AnyView) -> AnyView = { $0 }, startGate: @escaping (AnyView) -> AnyView = { $0 },
        @ViewBuilder accessory: @escaping (String) -> Accessory, @ViewBuilder extras: () -> Extras
    ) {
        self.maxHeight = maxHeight
        self.model = model
        self.actions = actions
        self.memoryGate = memoryGate
        self.startGate = startGate
        self.accessory = accessory
        self.extras = extras()
    }

    private static var scrollSpace: String { "wmenu.scroll" }

    private var scrollCue: WMenuScrollCue {
        WMenuScrollCue.cue(contentHeight: content.height, viewportHeight: viewport, offset: content.offset)
    }

    /// The fade and the chevron at one edge of the scrolling middle.
    private func scrollCueEdge(top: Bool) -> some View {
        let fade = LinearGradient(
            colors: [theme.backgroundPage.opacity(0), theme.backgroundPage.opacity(theme.isHighContrast ? 1 : 0.96)],
            startPoint: top ? .bottom : .top, endPoint: top ? .top : .bottom)
        return ZStack(alignment: top ? .top : .bottom) {
            fade
            WMenuChevron(up: top)
                .stroke(theme.isHighContrast ? theme.textPrimary : theme.textSecondary, style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
                .frame(width: 14 * min(k, 1.6), height: 7 * min(k, 1.6))
                .padding(top ? .top : .bottom, theme.space(4))
        }
        .frame(height: 30 * min(k, 1.6))
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    /// The menu's width: 320 points, widened with large text so a row never clips.
    public static var width: CGFloat { 320 }

    public var body: some View {
        WMenuColumn(cap: maxHeight) {
            // Pinned at the top: a recording is never scrolled out of sight, and neither is the memory state.
            VStack(alignment: .leading, spacing: 0) {
                if let recording = model.recording {
                    WMenuSection { recordingRows(recording) }
                    divider
                }
                WMenuSection { memoryRow }
                divider
            }
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    VStack(alignment: .leading, spacing: 0) { middle }
                        .background(WMenuMeasure(
                            value: { CGSize(width: $0.size.height, height: -$0.frame(in: .named(Self.scrollSpace)).minY) },
                            report: { content = WMenuContentMetrics(height: $0.width, offset: $0.height) }))
                }
                .coordinateSpace(name: Self.scrollSpace)
                .scrollBounceBehavior(.basedOnSize)
                .background(WMenuMeasure(value: { CGSize(width: 0, height: $0.size.height) }, report: { viewport = $0.height }))
                // The overlay scrollers hide, so a row cut at the edge needs a cue of its own: a fade and a chevron (a shape, so it
                // survives high contrast and needs no words). Not a control and not read out; the scroll view is.
                .overlay(alignment: .bottom) { if scrollCue.below { scrollCueEdge(top: false) } }
                .overlay(alignment: .top) { if scrollCue.above { scrollCueEdge(top: true) } }
                .onChange(of: focus) { _, id in if let id { proxy.scrollTo(id) } }
            }
            VStack(alignment: .leading, spacing: 0) {
                divider
                WMenuSection {
                    WMenuRow(glyph: "macwindow", title: strings.menu.openWhimbrel, action: actions.openWhimbrel)
                        .menuControl(WMenuFocusID.open, $focus)
                    WMenuRow(glyph: "gearshape", title: strings.menu.settings, action: actions.openSettings)
                        .menuControl(WMenuFocusID.settings, $focus)
                    extras
                    WMenuRow(glyph: "power", title: strings.menu.quit, action: actions.quit)
                        .menuControl(WMenuFocusID.quit, $focus)
                }
            }
        }
        .frame(width: Self.width * min(k, 1.6), alignment: .leading)
        .background(theme.backgroundPage)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.menu.menuName)
        // Something has the keyboard as soon as the menu shows, so the arrow keys work without a click.
        .onAppear { focus = model.initialFocus }
        // A recording that starts or stops while the menu is open adds or removes controls: focus stays on the one it was on, and
        // moves only when that control is gone.
        .onChange(of: model.focusOrder) { _, order in focus = WMenuFocus.kept(focus, in: order, fallback: model.initialFocus) }
        .onChange(of: activeState) { _, state in if state != .inactive, focus == nil || !model.focusOrder.contains(focus ?? "") { focus = model.initialFocus } }
        // Arrow keys move through the controls that are on. Escape closes the window the system draws.
        .onMoveCommand { direction in
            let order = model.focusOrder
            switch direction {
            case .down: focus = WMenuFocus.move(from: focus, in: order, .down)
            case .up: focus = WMenuFocus.move(from: focus, in: order, .up)
            default: break
            }
        }
    }

    /// Needs attention, the next meeting, Start Meeting, Ask about this window and what Whimbrel is working on: the part that
    /// scrolls when the menu is taller than the screen.
    @ViewBuilder
    private var middle: some View {
        if model.showsNeeds {
            WMenuSection(label: strings.menu.needsAttention) {
                ForEach(model.needs) { need in needRow(need) }
            }
            divider
        }
        if let next = model.next {
            WMenuSection(label: strings.menu.nextMeeting) { nextRows(next) }
            divider
        }
        WMenuSection {
            if model.offersStart {
                startGate(AnyView(
                    WMenuRow(glyph: "record.circle", title: strings.menu.startMeeting, action: actions.startMeeting)
                        .menuControl(WMenuFocusID.start, $focus)))
                if let caption = model.startCaption {
                    Text(caption).wFont(.secondary).foregroundStyle(theme.textSecondary)
                        .padding(.leading, theme.space(8) + 28 * k).padding(.trailing, theme.space(8)).padding(.bottom, theme.space(4))
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            WMenuRow(glyph: "sparkles", title: strings.menu.askAboutWindow, action: actions.askAboutWindow)
                .menuControl(WMenuFocusID.ask, $focus)
        }
        if let stage = model.workingOn {
            divider
            WMenuSection(label: strings.menu.workingOn) {
                HStack(spacing: theme.space(10)) {
                    WGlyph(name: "circle.dotted", size: 13, weight: .medium).foregroundStyle(theme.accentPrimary).frame(width: 18 * k)
                    Text(stage).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, theme.space(8))
                .frame(minHeight: WTokens.Size.target * k)
                .accessibilityElement(children: .combine)
            }
        }
    }

    private var divider: some View {
        Rectangle().fill(theme.border.opacity(theme.isHighContrast ? 1 : 0.7)).frame(height: max(1, theme.borderWidth))
            .accessibilityHidden(true)
    }

    // MARK: Recording

    @ViewBuilder
    private func recordingRows(_ recording: WMenuRecording) -> some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            // The state, the time and the title, then the buttons under them: the same shape in every state, so a recording that
            // turns into a problem does not jump.
            VStack(alignment: .leading, spacing: theme.space(6)) {
                recordingState(recording)
                HStack(spacing: theme.space(8)) { recordingButtons(recording) }
            }
            .padding(.horizontal, theme.space(8))
            if let streams = WMenuText.streamsLine(recording, strings: strings) {
                Text(streams).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, theme.space(8))
            }
            if let wait = WMenuText.stopWaitsLine(recording, strings: strings) {
                Text(wait).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, theme.space(8))
            }
            if let reason = WMenuText.problemLine(recording, strings: strings) {
                WErrorLine(text: reason).padding(.horizontal, theme.space(8))
            }
            WMenuRow(glyph: "macwindow.on.rectangle", title: strings.live.showInMeetingWindow, action: actions.showMeetingWindow)
                .menuControl(WMenuFocusID.meetingWindow, $focus)
        }
    }

    /// The state in words that say a recording is open, the recorded time and the meeting's title. Whatever the state.
    private func recordingState(_ recording: WMenuRecording) -> some View {
        let ink: Color = {
            switch recording.state {
            case .recording: return theme.recording
            case .problem: return theme.error
            default: return theme.textPrimary
            }
        }()
        return VStack(alignment: .leading, spacing: theme.space(2)) {
            HStack(spacing: theme.space(5)) {
                WGlyph(name: recording.state.glyph, size: 10, weight: .semibold)
                Text(WMenuText.stateWord(recording.state, strings: strings)).wFont(.secondary).fontWeight(.semibold)
                if let seconds = recording.elapsedSeconds {
                    Text(WRecordingText.clock(seconds: seconds)).wFont(.time).font(WTextRole.secondary.font(scale: k, style: theme.type).monospacedDigit())
                }
            }
            .foregroundStyle(ink)
            if let title = WMenuText.titleLine(recording) {
                Text(title).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(2).fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(WMenuText.spokenState(recording, strings: strings))
        .onChange(of: recording.state) { _, state in
            AccessibilityNotification.Announcement(WMenuText.stateWord(state, strings: strings)).post()
        }
    }

    @ViewBuilder
    private func recordingButtons(_ recording: WMenuRecording) -> some View {
        if recording.showsPauseButton {
            if recording.offersResume {
                WSpokenButton(title: strings.resume, spokenName: strings.menu.resumeRecording, glyph: "play.fill", action: actions.resumeRecording)
                    .disabled(!recording.canTogglePause)
                    .menuControl(WMenuFocusID.pause, $focus)
            } else {
                WSpokenButton(title: strings.pause, spokenName: strings.menu.pauseRecording, glyph: "pause.fill", action: actions.pauseRecording)
                    .disabled(!recording.canTogglePause)
                    .menuControl(WMenuFocusID.pause, $focus)
            }
        }
        if recording.offersStop {
            if recording.isSaving {
                // Stop was asked for and the meeting is still being saved: nothing is left to stop. Off, so it takes no focus.
                WSpokenButton(title: strings.menu.saving, action: {}).disabled(true).environment(\.wDisabledButtonLook, true)
            } else {
                WSpokenButton(title: strings.stop, spokenName: strings.menu.stopRecording, glyph: "stop.fill", action: actions.stopRecording)
                    .disabled(!recording.canStop)
                    .menuControl(WMenuFocusID.stop, $focus)
            }
        }
    }

    // MARK: Memory

    private var memoryRow: some View {
        let paused: Bool = { if case .pausedUntil = model.memory { return true } else { return false } }()
        return memoryGate(AnyView(
            WCaptureState(state: model.memory, spokenName: WMenuText.memoryName(model.memory, strings: strings), onWhatIsCaptured: actions.whatIsCaptured) {
                if model.memory != .off {
                    WPauseChoices(isPaused: paused, onPause: actions.pauseMemory, onResume: actions.resumeMemory)
                    Divider()
                }
                Button(model.memory == .off ? strings.menu.turnOnMemory : strings.menu.turnOffMemory, action: actions.toggleMemory)
            }
            .menuControl(WMenuFocusID.memory, $focus)
            .padding(.horizontal, theme.space(4))))
    }

    // MARK: Needs attention

    private func needRow(_ need: WMenuNeed) -> some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                WGlyph(name: need.kind == .sync ? "arrow.triangle.2.circlepath" : "exclamationmark.triangle.fill", size: 12, weight: .semibold)
                    .foregroundStyle(theme.isDark ? theme.warning : theme.textPrimary)
                    .frame(width: 18 * k)
                VStack(alignment: .leading, spacing: theme.space(2)) {
                    Text(need.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                    if let detail = need.detail {
                        Text(detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 0)
            }
            .accessibilityElement(children: .combine)
            let hasButtons = !need.actions.isEmpty
            if hasButtons || need.kind == .sync {
                HStack(spacing: theme.space(8)) {
                    ForEach(Array(need.actions.enumerated()), id: \.offset) { index, action in
                        WSpokenButton(title: action.title, spokenName: action.spokenName, kind: .secondary, isLink: action.isLink, action: action.run)
                            .menuControl(WMenuFocusID.need(need.id, index), $focus)
                    }
                    accessory(need.id)
                        .menuControl(WMenuFocusID.need(need.id, need.actions.count), $focus)
                }
                .padding(.leading, theme.space(26) * k)
            }
        }
        .padding(.horizontal, theme.space(8))
        .padding(.vertical, theme.space(6))
    }

    // MARK: Next meeting

    @ViewBuilder
    private func nextRows(_ next: WMenuNext) -> some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            VStack(alignment: .leading, spacing: theme.space(2)) {
                Text(next.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).lineLimit(2).fixedSize(horizontal: false, vertical: true)
                Text(next.time).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
            .accessibilityElement(children: .combine)
            if next.topic != nil || next.hasBrief {
                HStack(spacing: theme.space(8)) {
                    if let topic = next.topic { topicChip(topic) }
                    if next.hasBrief {
                        WSpokenButton(title: strings.menu.openBrief, spokenName: strings.menu.openBriefFor(next.title), action: actions.openBrief)
                            .menuControl(WMenuFocusID.brief, $focus)
                    }
                }
            }
        }
        .padding(.horizontal, theme.space(8))
        .padding(.vertical, theme.space(4))
    }

    private func topicChip(_ topic: String) -> some View {
        Button(action: actions.openTopic) {
            HStack(spacing: theme.space(5)) {
                WGlyph(name: "tag", size: 11, weight: .semibold)
                Text(topic).wFont(.secondary).lineLimit(1).truncationMode(.tail)
            }
            .foregroundStyle(theme.textPrimary)
            .padding(.horizontal, theme.space(10))
            .frame(minHeight: WTokens.Size.target * k)
            .background(Capsule().fill(theme.selection.opacity(0.6)))
            .overlay(Capsule().wBorder(theme.firmEdge, style: theme.borderStroke.widened(1)))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(strings.menu.openTopic(topic))
        .menuControl(WMenuFocusID.topic, $focus)
    }
}

extension WMenuView where Accessory == EmptyView, Extras == EmptyView {
    public init(model: WMenuModel, actions: WMenuActions) {
        self.init(model: model, actions: actions, accessory: { _ in EmptyView() }, extras: { EmptyView() })
    }
}

// MARK: The menu before the library opens

/// What the launch menu says and offers: the same surface as the menu, with two rows.
public struct WLaunchMenuModel {
    public var message: String
    /// The library waits for the person (a key, a recovery): the glyph is a warning, not a clock.
    public var waitingForUser: Bool
    public var showTitle: String
    public var quitTitle: String

    public init(message: String, waitingForUser: Bool, showTitle: String, quitTitle: String) {
        self.message = message
        self.waitingForUser = waitingForUser
        self.showTitle = showTitle
        self.quitTitle = quitTitle
    }
}

public struct WLaunchMenuView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let model: WLaunchMenuModel
    let onShow: () -> Void
    let onQuit: () -> Void

    public init(model: WLaunchMenuModel, onShow: @escaping () -> Void, onQuit: @escaping () -> Void) {
        self.model = model
        self.onShow = onShow
        self.onQuit = onQuit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            WMenuSection {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
                    WGlyph(name: model.waitingForUser ? "exclamationmark.triangle.fill" : "circle.dotted", size: 13, weight: .semibold)
                        .foregroundStyle(model.waitingForUser && !theme.isDark ? theme.textPrimary : (model.waitingForUser ? theme.warning : theme.accentPrimary))
                        .frame(width: 18 * k)
                    Text(model.message).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, theme.space(8))
                .padding(.vertical, theme.space(6))
                .accessibilityElement(children: .combine)
            }
            Rectangle().fill(theme.border.opacity(theme.isHighContrast ? 1 : 0.7)).frame(height: max(1, theme.borderWidth)).accessibilityHidden(true)
            WMenuSection {
                WMenuRow(glyph: "macwindow", title: model.showTitle, action: onShow)
                WMenuRow(glyph: "power", title: model.quitTitle, action: onQuit)
            }
        }
        .frame(width: WMenuView<EmptyView, EmptyView>.width * min(k, 1.6), alignment: .leading)
        .background(theme.backgroundPage)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.menu.menuName)
    }
}
