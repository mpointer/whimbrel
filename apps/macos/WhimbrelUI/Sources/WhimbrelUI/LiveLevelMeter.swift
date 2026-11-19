import SwiftUI

/// The level meter: a row of bars that follows the live level, or a still mark under Reduce Motion that shows
/// only "audio" or "no audio". Decoration with a name: it is not announced as it changes.
public struct WLevelMeter: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let level: Float
    let color: Color
    /// A history to start from, for a still picture (the gallery).
    let seed: [Float]
    @State private var history: [Float] = []

    static let barCount = 16

    public init(level: Float, color: Color? = nil, seed: [Float] = []) {
        self.level = level
        self.color = color ?? .primary
        self.seed = seed
        _history = State(initialValue: seed)
    }

    /// A fixed shape the first frame is drawn with, scaled by the level, until the history has filled.
    private static let shape: [Float] = [0.2, 0.45, 0.7, 0.4, 0.9, 0.6, 0.3, 0.8, 0.5, 0.25, 0.65, 0.4, 0.85, 0.3, 0.55, 0.2]

    private var shown: [Float] {
        guard history.count >= Self.barCount else {
            // Not enough readings yet: the shape at the current level, with what has come in on the right.
            let base = Self.shape.map { $0 * level }
            return Array(base.prefix(Self.barCount - history.count)) + history
        }
        return Array(history.suffix(Self.barCount))
    }

    private var name: String { WLevelMath.isAudio(level) ? strings.live.audioArriving : strings.live.noAudioNow }

    public var body: some View {
        Group {
            if reduceMotion { still } else { bars }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(name)
        .accessibilityAddTraits(.isImage)
        .onChange(of: level) { _, new in
            history.append(new)
            if history.count > Self.barCount { history.removeFirst(history.count - Self.barCount) }
        }
    }

    private var bars: some View {
        HStack(alignment: .center, spacing: 2.5 * k) {
            ForEach(Array(shown.enumerated()), id: \.offset) { index, value in
                let height = (3 + CGFloat(value) * 21) * k
                Capsule().fill(color.opacity(value > 0 ? 1 : 0.35)).frame(width: 2.5 * k, height: height)
                    .animation(.easeOut(duration: 0.12), value: value)
                    .id(index)
            }
        }
        .frame(height: 24 * k)
    }

    /// Reduce Motion: one still mark. A wave glyph for audio, a flat line for none.
    private var still: some View {
        HStack(spacing: theme.space(4)) {
            WGlyph(name: WLevelMath.isAudio(level) ? "waveform" : "minus", size: 14, weight: .semibold)
        }
        .foregroundStyle(color)
        .frame(height: 24 * k)
    }
}
