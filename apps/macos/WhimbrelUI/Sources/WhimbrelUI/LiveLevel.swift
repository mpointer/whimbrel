import Foundation

/// The level meter's numbers. The app reads a peak from the capture code about 8 times a second, turns it into
/// 0 to 1 here, draws it and throws it away. Nothing is stored.
public enum WLevelMath {
    /// The quietest peak the meter shows, in dB. It matches the silence warning's own floor (-50 dB) with a little room.
    public static let floorDB: Double = -54
    /// The loudest, in dB.
    public static let ceilingDB: Double = -6
    /// Below this the meter says there is no audio right now.
    public static let audioThreshold: Float = 0.1

    /// 0 for silence, up to 1 for a loud peak. `peak` is a linear sample peak, 0 to 1.
    public static func level(fromPeak peak: Float) -> Float {
        guard peak > 0 else { return 0 }
        let db = 20 * log10(Double(max(peak, 1e-9)))
        let scaled = (db - floorDB) / (ceilingDB - floorDB)
        return Float(min(1, max(0, scaled)))
    }

    public static func isAudio(_ level: Float) -> Bool { level >= audioThreshold }
}
