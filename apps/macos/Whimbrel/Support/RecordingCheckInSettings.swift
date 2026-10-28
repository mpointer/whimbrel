import Foundation

/// "Ask if I'm still recording after N hours". 0 = Off. Local to this Mac.
final class RecordingCheckInSettings: ObservableObject {
    static let allowedHours = [0, 1, 2, 3, 4]
    private static let hoursKey = "com.mpointer.kestrel.recordingCheckInHours"
    private let defaults: UserDefaults

    @Published var hours: Int {
        didSet { defaults.set(hours, forKey: Self.hoursKey) }
    }

    var firstPromptAfter: TimeInterval? { hours == 0 ? nil : TimeInterval(hours) * 3600 }

    /// The check-in for a recording that started at `startedAt`. In DEBUG builds, positive
    /// integer seconds under `debugCheckInSeconds` (first prompt and re-ask) and
    /// `debugCheckInCountdownSeconds` shorten it for the manual pass.
    func makeCheckIn(startedAt: Date) -> RecordingCheckIn {
        #if DEBUG
        let after = debugSeconds("com.mpointer.kestrel.debugCheckInSeconds")
        let countdown = debugSeconds("com.mpointer.kestrel.debugCheckInCountdownSeconds")
        return RecordingCheckIn(startedAt: startedAt, firstPromptAfter: after ?? firstPromptAfter,
                                countdown: countdown ?? RecordingCheckIn.defaultCountdown,
                                reaskAfter: after ?? RecordingCheckIn.defaultReaskAfter)
        #else
        return RecordingCheckIn(startedAt: startedAt, firstPromptAfter: firstPromptAfter)
        #endif
    }

    #if DEBUG
    private func debugSeconds(_ key: String) -> TimeInterval? {
        let value = defaults.integer(forKey: key)
        return value > 0 ? TimeInterval(value) : nil
    }
    #endif

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let stored = defaults.object(forKey: Self.hoursKey) as? Int
        hours = stored.flatMap { Self.allowedHours.contains($0) ? $0 : nil } ?? 2
    }
}
