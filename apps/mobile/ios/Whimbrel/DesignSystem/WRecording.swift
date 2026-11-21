import SwiftUI

/// What a recording is doing. Every state has its own glyph and word, so it never relies on color.
enum WRecordingState: Equatable {
    case recording, paused, processing, off
    case problem(String)

    var glyph: String {
        switch self {
        case .recording: "record.circle.fill"
        case .paused: "pause.circle.fill"
        case .processing: "hourglass"
        case .off: "circle"
        case .problem: "exclamationmark.triangle.fill"
        }
    }

    func word(_ s: WStrings) -> String {
        switch self {
        case .recording: s.recording
        case .paused: s.paused
        case .processing: s.processing
        case .off: s.notRecording
        case .problem: s.problem
        }
    }

    var isLive: Bool { self == .recording || self == .paused }
}

/// The recording state, always on screen while a recording exists. `bar` runs the full width with
/// Pause and Stop; `pill` is the compact form that sits above the tab bar on other screens.
struct WRecordingIndicator: View {
    enum Size { case bar, pill }
    @Environment(\.wStrings) private var strings
    let state: WRecordingState
    let elapsedSeconds: Int
    var size: Size = .bar
    var onPauseResume: (() -> Void)?
    var onStop: (() -> Void)?

    private var tint: Color {
        switch state {
        case .recording: WFixed.recording.color
        case .paused: WFixed.paused.color
        case .problem: Color.theme(.error)
        case .processing, .off: Color.theme(.textSecondary)
        }
    }

    /// "Recording, 12 minutes, 47 seconds" or "Problem, Microphone not found".
    static func spokenLabel(state: WRecordingState, elapsedSeconds: Int, strings: WStrings) -> String {
        if case .problem(let reason) = state { return strings.spoken(strings.problem, reason) }
        return state.isLive ? strings.spoken(state.word(strings), WElapsed.spoken(elapsedSeconds)) : state.word(strings)
    }

    var body: some View {
        let filled = state == .recording
        HStack(spacing: WTokens.Space.sm) {
            HStack(spacing: WTokens.Space.sm) {
                Image(systemName: state.glyph).font(.title3).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 0) {
                    Text(state.word(strings)).wType(WTokens.TypeScale.bodyStrong)
                    if case .problem(let reason) = state {
                        Text(reason).wType(WTokens.TypeScale.caption)
                    }
                }
                Spacer(minLength: WTokens.Space.sm)
                if state.isLive {
                    Text(WElapsed.clock(elapsedSeconds))
                        .wType(size == .bar ? WTokens.TypeScale.title : WTokens.TypeScale.time)
                        .monospacedDigit()
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Self.spokenLabel(state: state, elapsedSeconds: elapsedSeconds, strings: strings))
            .accessibilityAddTraits(.updatesFrequently)
            if size == .bar, state.isLive {
                if let onPauseResume {
                    Button(state == .paused ? strings.resume : strings.pause, action: onPauseResume).buttonStyle(.w(.secondary))
                }
                if let onStop {
                    Button(strings.stop, action: onStop).buttonStyle(.w(.secondary))
                }
            }
        }
        .padding(.horizontal, WTokens.Space.lg)
        .padding(.vertical, WTokens.Space.sm)
        .frame(minHeight: WTokens.Size.target)
        .foregroundStyle(filled ? WFixed.onRecording.color : tint)
        .background(filled ? WFixed.recording.color : Color.theme(.backgroundCard),
                    in: RoundedRectangle(cornerRadius: size == .pill ? WTokens.Radius.pill : WTokens.Radius.card, style: .continuous))
        .overlay {
            if !filled {
                RoundedRectangle(cornerRadius: size == .pill ? WTokens.Radius.pill : WTokens.Radius.card, style: .continuous).strokeBorder(tint, lineWidth: 1.5)
            }
        }
    }
}

/// Shown at the start of every in-person recording. It never says anyone was told or agreed.
struct WConsentBanner: View {
    @Environment(\.wStrings) private var strings
    let onShowNotice: () -> Void
    let onDone: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.sm) {
            Label(strings.consentMessage, systemImage: "person.2.wave.2")
                .wType(WTokens.TypeScale.body)
                .foregroundStyle(Color.theme(.textPrimary))
            HStack(spacing: WTokens.Space.sm) {
                Button(strings.showNotice, action: onShowNotice).buttonStyle(.w(.secondary, fill: true))
                Button(strings.done, action: onDone).buttonStyle(.w(.secondary, fill: true))
            }
        }
        .padding(WTokens.Space.md)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(WFixed.recordingTint.color, in: RoundedRectangle(cornerRadius: WTokens.Radius.card, style: .continuous))
        .accessibilityElement(children: .contain)
    }
}
