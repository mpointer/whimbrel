import Foundation

/// Meeting Prep's lead time. Local to this Mac (not part of settings sync).
final class MeetingPrepSettings: ObservableObject {
    private static let leadKey = "com.mpointer.kestrel.meetingPrepLeadMinutes"
    static let range: ClosedRange<Double> = 5...120

    private let defaults: UserDefaults

    @Published var leadMinutes: Double {
        // Assigning inside the property's own `didSet` does not call it again, so the
        // clamped value is stored here, once.
        didSet {
            let clamped = Self.clamp(leadMinutes)
            if clamped != leadMinutes { leadMinutes = clamped }
            defaults.set(leadMinutes, forKey: Self.leadKey)
        }
    }

    private static func clamp(_ minutes: Double) -> Double {
        min(max(minutes, range.lowerBound), range.upperBound)
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let stored = defaults.object(forKey: Self.leadKey) as? Double
        leadMinutes = Self.clamp(stored ?? 30)
    }
}
