import SwiftUI

// MARK: Recording indicator

public enum WRecordingState: Equatable, Sendable {
    case recording, paused, processing, off
    /// The recording is starting: the controls wait. Used by the live meeting screen.
    case starting
    /// Capture is starting again after a pause, until audio arrives.
    case resuming
    /// A short reason, for example "Microphone not found".
    case problem(String)

    /// The word shown beside the glyph. Never color alone.
    public func word(using strings: WStrings) -> String {
        switch self {
        case .recording: return strings.recording
        case .paused: return strings.paused
        case .processing: return strings.processing
        case .off: return strings.off
        case .problem: return strings.problem
        case .starting: return strings.starting
        case .resuming: return strings.resuming
        }
    }

    /// What a screen reader says once when the state changes: "Recording paused", "Recording".
    public func announcement(using strings: WStrings) -> String {
        switch self {
        case .paused: return strings.recordingPaused
        case .recording: return strings.recording
        default: return word(using: strings)
        }
    }

    /// The English word, for ids and tests. Components use `word(using:)`.
    public var word: String { word(using: .english) }

    /// A different shape for each state, so the state does not rest on color.
    public var glyph: String {
        switch self {
        case .recording: return "circle.fill"
        case .paused: return "pause.circle.fill"
        case .processing: return "circle.dotted"
        case .off: return "circle"
        case .problem: return "exclamationmark.triangle.fill"
        case .starting, .resuming: return "circle.dotted"
        }
    }
}

public enum WRecordingSize: Sendable { case bar, pill, menu }

/// Plain text for the recording indicator: the accessible label, the spoken time and the clock.
public enum WRecordingText {
    /// "14 minutes 8 seconds", "1 hour 2 minutes", "1 second".
    public static func spoken(seconds: Int, strings: WStrings = .english) -> String {
        let total = max(0, seconds)
        let hours = total / 3600, minutes = (total % 3600) / 60, secs = total % 60
        var parts: [String] = []
        if hours > 0 { parts.append(strings.hours(hours)) }
        if minutes > 0 { parts.append(strings.minutes(minutes)) }
        if secs > 0 || parts.isEmpty { parts.append(strings.seconds(secs)) }
        return strings.joinDuration(parts)
    }

    /// "14:08" or "1:02:03".
    public static func clock(seconds: Int) -> String {
        let total = max(0, seconds)
        let hours = total / 3600, minutes = (total % 3600) / 60, secs = total % 60
        if hours > 0 { return String(format: "%d:%02d:%02d", hours, minutes, secs) }
        return String(format: "%d:%02d", minutes, secs)
    }

    /// The label a screen reader gets, for example "Recording, 14 minutes 8 seconds".
    public static func label(state: WRecordingState, elapsedSeconds: Int?, strings: WStrings = .english) -> String {
        let word = state.word(using: strings)
        switch state {
        case .recording, .paused, .resuming:
            guard let elapsedSeconds else { return word }
            return strings.joinDetails([word, spoken(seconds: elapsedSeconds, strings: strings)])
        case .processing: return strings.processing
        case .off: return strings.off
        case .starting: return strings.starting
        case .problem(let reason): return strings.joinDetails([word, reason])
        }
    }
}

/// Shows whether Whimbrel is recording: a glyph, the word and the elapsed time. Sizes: a full width
/// bar with Pause and Stop slots, a pill, and a compact menu form. State changes are announced.
public struct WRecordingIndicator: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let state: WRecordingState
    let elapsedSeconds: Int?
    let size: WRecordingSize
    let onPause: (() -> Void)?
    let onResume: (() -> Void)?
    let onStop: (() -> Void)?
    /// The live level, 0 to 1, from the capture code. Nil draws the still waveform, as before.
    let level: Float?
    /// What the bar offers where Stop is while the recording is still starting. `.unavailable` (the default) is a Stop that is
    /// drawn and off, as before; `.stop` is a Stop that works (audio is on, the meeting is not saved yet); `.saving` replaces
    /// Stop with a disabled "Saving..." once a Stop has been asked for.
    let startingStop: WStopOffer

    public init(
        state: WRecordingState,
        elapsedSeconds: Int? = nil,
        size: WRecordingSize = .bar,
        level: Float? = nil,
        onPause: (() -> Void)? = nil,
        onResume: (() -> Void)? = nil,
        onStop: (() -> Void)? = nil,
        startingStop: WStopOffer = .unavailable
    ) {
        self.startingStop = startingStop
        self.level = level
        self.state = state
        self.elapsedSeconds = elapsedSeconds
        self.size = size
        self.onPause = onPause
        self.onResume = onResume
        self.onStop = onStop
    }

    private var label: String { WRecordingText.label(state: state, elapsedSeconds: elapsedSeconds, strings: strings) }

    private var showsTime: Bool {
        switch state {
        case .recording, .paused, .resuming: return elapsedSeconds != nil
        default: return false
        }
    }

    private func stateInk(onRecordingFill: Bool) -> Color {
        if onRecordingFill { return theme.onRecording }
        switch state {
        case .recording: return theme.recording
        case .paused: return theme.paused
        case .processing, .starting, .resuming: return theme.accentPrimary
        case .off: return theme.textSecondary
        case .problem: return theme.error
        }
    }

    public var body: some View {
        Group {
            switch size {
            case .bar: bar
            case .pill: pill
            case .menu: menu
            }
        }
        .onChange(of: state) { _, _ in
            AccessibilityNotification.Announcement(state.announcement(using: strings)).post()
        }
    }

    // MARK: Bar

    private var bar: some View {
        let filled = state == .recording
        let ink = stateInk(onRecordingFill: filled)
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        return HStack(spacing: theme.space(12)) {
            HStack(spacing: theme.space(10)) {
                WGlyph(name: state.glyph, size: 18, weight: .semibold)
                Text(state.word(using: strings)).wFont(.subtitle).lineLimit(1).fixedSize()
                if showsTime, let elapsedSeconds {
                    Text(WRecordingText.clock(seconds: elapsedSeconds)).wFont(.title).lineLimit(1).fixedSize()
                }
                if case .problem(let reason) = state {
                    Text(reason).wFont(.body).foregroundStyle(theme.textSecondary).lineLimit(2)
                }
                if state == .recording {
                    if let level { ViewThatFits(in: .horizontal) { WLevelMeter(level: level, color: ink.opacity(0.9)); Color.clear.frame(width: 0, height: 0) } } else { WWaveform(color: ink.opacity(0.9)).accessibilityHidden(true) }
                }
            }
            .foregroundStyle(ink)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(label)
            .accessibilityAddTraits(state == .recording ? .updatesFrequently : [])
            Spacer(minLength: theme.space(8))
            barButtons(filled: filled)
        }
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(10))
        .frame(minHeight: 52 * k)
        .background(shape.fill(filled ? theme.recording : theme.backgroundCard))
        .overlay(shape.wBorder(filled ? theme.recording : stateInk(onRecordingFill: false).opacity(theme.isHighContrast ? 1 : 0.5), style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func barButtons(filled: Bool) -> some View {
        HStack(spacing: theme.space(8)) {
            if state == .starting, let onPause {
                WButton(strings.pause, glyph: "pause.fill", kind: .secondary, action: onPause).disabled(true)
            }
            if state == .recording, let onPause {
                Button(action: onPause) { barButtonLabel(strings.pause, glyph: "pause.fill") }
                    .buttonStyle(WOnRecordingButtonStyle(solid: false))
            }
            if state == .paused || state == .resuming, let resume = onResume ?? onPause {
                WButton(strings.resume, glyph: "play.fill", kind: .secondary, action: resume).disabled(state == .resuming)
            }
            if state == .starting, startingStop == .saving {
                WButton(strings.menu.saving, kind: .secondary, action: {}).disabled(true).environment(\.wDisabledButtonLook, true)
            } else if let onStop, state == .recording || state == .paused || state == .starting || state == .resuming || state.isProblem {
                if filled {
                    Button(action: onStop) { barButtonLabel(strings.stop, glyph: "stop.fill") }
                        .buttonStyle(WOnRecordingButtonStyle(solid: true))
                } else {
                    WButton(strings.stop, glyph: "stop.fill", kind: .secondary, action: onStop).disabled(state == .starting && startingStop != .stop)
                }
            }
        }
    }

    private func barButtonLabel(_ title: String, glyph: String) -> some View {
        HStack(spacing: theme.space(6)) {
            WGlyph(name: glyph, size: 11, weight: .semibold)
            Text(title)
        }
    }

    // MARK: Pill and menu

    private var pill: some View {
        let ink = stateInk(onRecordingFill: false)
        return HStack(spacing: theme.space(6)) {
            WGlyph(name: state.glyph, size: 11, weight: .semibold).foregroundStyle(ink)
            Text(state.word(using: strings))
                .wFont(.bodyStrong)
                .foregroundStyle(state == .recording ? theme.recording : theme.textPrimary)
            if showsTime, let elapsedSeconds {
                Text(WRecordingText.clock(seconds: elapsedSeconds)).wFont(.time).foregroundStyle(theme.textPrimary)
            }
            if case .problem(let reason) = state {
                Text(reason).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
            }
        }
        .padding(.horizontal, theme.space(10))
        .padding(.vertical, theme.space(4))
        .frame(minHeight: 24 * k)
        .background(Capsule().fill(state == .recording ? theme.recordingTint : theme.backgroundCard))
        .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(ink.opacity(theme.isHighContrast ? 1 : 0.55), style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
    }

    private var menu: some View {
        HStack(spacing: theme.space(5)) {
            WGlyph(name: state.glyph, size: 10, weight: .semibold)
            Text(state.word(using: strings)).wFont(.secondary).fontWeight(.semibold)
            if showsTime, let elapsedSeconds {
                Text(WRecordingText.clock(seconds: elapsedSeconds)).wFont(.time).font(WTextRole.secondary.font(scale: k, style: theme.type).monospacedDigit())
            }
        }
        .foregroundStyle(stateInk(onRecordingFill: false))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
    }
}

/// Pause and Stop on the red recording bar: light text on red, or a light fill with red text.
struct WOnRecordingButtonStyle: ButtonStyle {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let solid: Bool

    func makeBody(configuration: Configuration) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        configuration.label
            .wFont(.bodyStrong)
            .foregroundStyle(solid ? theme.recording : theme.onRecording)
            .padding(.horizontal, theme.space(12))
            .frame(minHeight: WTokens.Size.target * k)
            .background(shape.fill(solid ? theme.onRecording : theme.onRecording.opacity(0.16)))
            .overlay(shape.strokeBorder(theme.onRecording.opacity(solid ? 0 : 0.7), lineWidth: 1))
            .opacity(configuration.isPressed ? 0.8 : 1)
            .contentShape(shape)
    }
}

/// A still waveform. Decoration only.
struct WWaveform: View {
    @Environment(\.wTextScale) private var k
    let color: Color
    private let heights: [CGFloat] = [5, 11, 17, 9, 21, 14, 8, 19, 12, 6, 16, 10, 20, 7, 13, 5]

    var body: some View {
        HStack(alignment: .center, spacing: 2.5 * k) {
            ForEach(Array(heights.enumerated()), id: \.offset) { _, h in
                Capsule().fill(color).frame(width: 2.5 * k, height: h * k)
            }
        }
        .frame(height: 24 * k)
    }
}

// MARK: Capture state

public enum WMemoryState: Equatable, Sendable {
    case on
    /// Memory is switched on but screen text is not being saved: the permission is off, or capture is not running. Never
    /// shown as plain "Memory on".
    case onNotCapturing
    /// The time the pause ends, for example "10:52".
    case pausedUntil(String)
    case off

    public func title(using strings: WStrings) -> String {
        switch self {
        case .on: return strings.memoryOn
        case .onNotCapturing: return strings.memoryNotCapturing
        case .pausedUntil(let time): return strings.pausedUntil(time)
        case .off: return strings.memoryOff
        }
    }

    /// The English title, for ids and tests. Components use `title(using:)`.
    public var title: String { title(using: .english) }

    var glyph: String {
        switch self {
        case .on: return "record.circle"
        case .onNotCapturing: return "exclamationmark.triangle"
        case .pausedUntil: return "pause.circle"
        case .off: return "circle.slash"
        }
    }
}

/// Work memory state. A button that opens a menu of pause choices (a slot) and "What is captured".
public struct WCaptureState<Choices: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let state: WMemoryState
    let onWhatIsCaptured: () -> Void
    let choices: Choices
    /// The name a screen reader gives the control, when it should differ from the words shown.
    let spokenName: String?

    public init(state: WMemoryState, spokenName: String? = nil, onWhatIsCaptured: @escaping () -> Void, @ViewBuilder pauseChoices: () -> Choices) {
        self.state = state
        self.spokenName = spokenName
        self.onWhatIsCaptured = onWhatIsCaptured
        self.choices = pauseChoices()
    }

    public var body: some View {
        let ink = state == .on ? theme.accentPrimary : theme.textPrimary
        Menu {
            choices
            Divider()
            Button(action: onWhatIsCaptured) { Label(strings.whatIsCaptured, systemImage: "info.circle") }
        } label: {
            HStack(spacing: theme.space(6)) {
                WGlyph(name: state.glyph, size: 13, weight: .semibold)
                Text(state.title(using: strings)).wFont(.bodyStrong).lineLimit(1)
                WGlyph(name: "chevron.down", size: 9, weight: .bold)
            }
            .foregroundStyle(ink)
            .padding(.horizontal, theme.space(12))
            .frame(minHeight: WTokens.Size.target * k)
            .background(Capsule().fill(state == .on ? theme.selection.opacity(0.6) : theme.backgroundCard))
            .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(state == .on ? theme.accentPrimary : theme.firmEdge, style: theme.borderStroke.widened(1)))
            .padding(2)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(spokenName ?? state.title(using: strings))
        .accessibilityHint(strings.opensPauseChoices)
    }
}

// MARK: Consent banner

/// Every string the banner can show, in English. It never says anyone was told or agreed.
/// The banner itself reads the same four strings from `WStrings` in the environment.
public enum WConsentStrings {
    public static func texts(in strings: WStrings) -> [String] {
        [strings.consentMessage, strings.copyNotice, strings.copied, strings.done]
    }
    public static let message = WStrings.english.consentMessage
    public static let copyNotice = WStrings.english.copyNotice
    public static let copied = WStrings.english.copied
    public static let done = WStrings.english.done
    public static let all: [String] = texts(in: .english)
}

public struct WConsentBanner: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let isCopied: Bool
    let onCopy: () -> Void
    let onDone: () -> Void

    public init(isCopied: Bool = false, onCopy: @escaping () -> Void, onDone: @escaping () -> Void) {
        self.isCopied = isCopied
        self.onCopy = onCopy
        self.onDone = onDone
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        HStack(spacing: theme.space(12)) {
            WGlyph(name: "info.circle", size: 16).foregroundStyle(theme.recording)
            Text(strings.consentMessage)
                .wFont(.bodyStrong)
                .foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: theme.space(8))
            WButton(isCopied ? strings.copied : strings.copyNotice, glyph: isCopied ? "checkmark" : "doc.on.doc", kind: .secondary, action: onCopy)
            WButton(strings.done, kind: .primary, action: onDone)
        }
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(10))
        .background(shape.fill(theme.backgroundCard))
        .overlay(alignment: .leading) {
            Rectangle().fill(theme.recording).frame(width: 3).clipShape(UnevenRoundedRectangle(topLeadingRadius: theme.shape.cardRadius, bottomLeadingRadius: theme.shape.cardRadius))
        }
        .overlay(shape.wBorder(theme.border.opacity(0.7), style: theme.borderStroke))
        .wShadow(WTokens.Elevation.level1)
        .accessibilityElement(children: .contain)
        .onChange(of: isCopied) { _, copied in
            if copied { AccessibilityNotification.Announcement(strings.copied).post() }
        }
    }
}

extension WRecordingState {
    var isProblem: Bool { if case .problem = self { return true } else { return false } }
}
