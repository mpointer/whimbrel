import Foundation

/// A growing 16 kHz mono buffer for one channel, with absolute sample indexes so chunk
/// timestamps stay anchored to the start of the recording even after samples are taken
/// or the oldest backlog is dropped.
struct LiveChannelBuffer {
    static let sampleRate = 16_000
    /// The most recent 10 minutes are kept when live work falls behind.
    static let defaultMaxBacklogSamples = 10 * 60 * sampleRate

    private(set) var samples: [Float] = []
    /// Absolute index (in 16 kHz samples since the start of the recording) of `samples[0]`.
    private(set) var startSample = 0
    private let maxBacklogSamples: Int

    init(maxBacklogSamples: Int = LiveChannelBuffer.defaultMaxBacklogSamples) {
        self.maxBacklogSamples = maxBacklogSamples
    }

    mutating func append(_ new: [Float]) {
        samples.append(contentsOf: new)
        if samples.count > maxBacklogSamples {
            let drop = samples.count - maxBacklogSamples
            samples.removeFirst(drop)
            startSample += drop
        }
    }

    /// Removes the first `count` samples and returns them with their absolute start index.
    /// `keepOverlap` samples at the end of the taken range stay at the front of the buffer
    /// for the next chunk (a hard cut re-reads a little so a boundary word is whole).
    mutating func take(count: Int, keepOverlap: Int) -> (samples: [Float], startSample: Int) {
        let n = min(max(count, 0), samples.count)
        let taken = Array(samples.prefix(n))
        let takenStart = startSample
        let advance = max(n - max(keepOverlap, 0), 0)
        samples.removeFirst(advance)
        startSample += advance
        return (taken, takenStart)
    }

    static func centiseconds(forSample sample: Int, sampleRate: Int = LiveChannelBuffer.sampleRate) -> Int64 {
        Int64(sample) * 100 / Int64(sampleRate)
    }
}

enum LivePausePolicy {
    /// Live work pauses in Low Power Mode and under serious or critical thermal pressure;
    /// the audio keeps being read into the (capped) buffer meanwhile.
    static func shouldPause(lowPowerMode: Bool, thermal: ProcessInfo.ThermalState) -> Bool {
        lowPowerMode || thermal == .serious || thermal == .critical
    }
}

enum LiveCadence {
    /// If a chunk took more than half its duration to transcribe, double the polling
    /// interval (capped at 2x the base); a fast chunk steps it back to the base.
    static func nextInterval(base: Double, current: Double, chunkSeconds: Double, tookSeconds: Double) -> Double {
        if tookSeconds > chunkSeconds / 2 { return min(current * 2, base * 2) }
        return base
    }

    /// About half the cores, at least one, so live work never competes with capture.
    static func threads(forCores cores: Int) -> Int32 {
        Int32(max(1, cores / 2))
    }
}
