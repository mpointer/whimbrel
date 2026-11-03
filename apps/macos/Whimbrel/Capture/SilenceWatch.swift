import Foundation

/// Pure decision logic for the "Whimbrel isn't hearing anything" warning. The caller feeds a
/// level sample every few seconds and acts on the returned `Event`; nothing here touches
/// audio or UI. Levels are dBFS peaks (see `decibels(fromPeak:)`).
struct SilenceWatch {
    /// Below this on every track counts as silence.
    static let thresholdDB = -50.0
    /// Continuous silence before the warning.
    static let silenceBeforeWarn: TimeInterval = 3 * 60
    /// Sound needed after a warning before it can fire again.
    static let soundToRearm: TimeInterval = 30

    enum Event: Equatable { case warn, clear }

    private var silentSince: Date?
    private var soundSince: Date?
    private var warned = false
    private var armed = true

    /// `systemDB` is nil when there is no system track (Room audio, mic-only): the mic alone decides.
    mutating func feed(at now: Date, systemDB: Double?, micDB: Double) -> Event? {
        let quiet = micDB < Self.thresholdDB && (systemDB ?? -.infinity) < Self.thresholdDB
        if quiet {
            soundSince = nil
            let since = silentSince ?? now
            silentSince = since
            guard armed, now.timeIntervalSince(since) >= Self.silenceBeforeWarn else { return nil }
            armed = false
            warned = true
            return .warn
        }
        silentSince = nil
        let since = soundSince ?? now
        soundSince = since
        if !armed, now.timeIntervalSince(since) >= Self.soundToRearm { armed = true }
        guard warned else { return nil }
        warned = false
        return .clear
    }

    static func decibels(fromPeak peak: Float) -> Double {
        20 * log10(max(Double(peak), 1e-9))
    }
}

/// The loudest sample seen since the last `take()`. Written from an audio thread, read from main.
final class PeakMeter: @unchecked Sendable {
    private let lock = NSLock()
    private var peak: Float = 0

    func record(_ value: Float) {
        lock.lock(); peak = max(peak, value); lock.unlock()
    }

    func take() -> Float {
        lock.lock(); defer { lock.unlock() }
        let value = peak
        peak = 0
        return value
    }
}
