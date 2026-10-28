import Foundation
import OSLog

/// FR-MEM-4: per-data-class retention settings, persisted in `UserDefaults` (matching
/// `MeetingTriggerService`'s pattern for user-tunable settings). Meetings default to a
/// longer window than raw screen/audio capture items — they're higher-value (a
/// transcript/summary can hang off one) — but this is just a sensible default, not
/// enforced anywhere; the user can set either to "keep forever."
@MainActor
final class RetentionSettings: ObservableObject {
    private static let captureItemDaysKey = "com.mpointer.kestrel.retentionCaptureItemDays"
    private static let meetingDaysKey = "com.mpointer.kestrel.retentionMeetingDays"
    /// `UserDefaults` has no way to distinguish "never set" from "set to 0" for
    /// integers, so store the "keep forever" flag as a separate bool per class rather
    /// than overloading a sentinel value in the day count itself.
    private static let captureItemKeepForeverKey = "com.mpointer.kestrel.retentionCaptureItemKeepForever"
    private static let meetingKeepForeverKey = "com.mpointer.kestrel.retentionMeetingKeepForever"
    private static let conversationDaysKey = "com.mpointer.kestrel.retentionConversationDays"
    private static let conversationKeepForeverKey = "com.mpointer.kestrel.retentionConversationKeepForever"

    private let log = Logger(subsystem: "com.mpointer.kestrel", category: "retention")

    @Published var captureItemDays: Int {
        didSet {
            UserDefaults.standard.set(captureItemDays, forKey: Self.captureItemDaysKey)
            SettingsFieldTimestamps.touch("retention.captureItemDays")
        }
    }
    @Published var captureItemKeepForever: Bool {
        didSet {
            UserDefaults.standard.set(captureItemKeepForever, forKey: Self.captureItemKeepForeverKey)
            SettingsFieldTimestamps.touch("retention.captureItemKeepForever")
        }
    }
    @Published var meetingDays: Int {
        didSet {
            UserDefaults.standard.set(meetingDays, forKey: Self.meetingDaysKey)
            SettingsFieldTimestamps.touch("retention.meetingDays")
        }
    }
    @Published var meetingKeepForever: Bool {
        didSet {
            UserDefaults.standard.set(meetingKeepForever, forKey: Self.meetingKeepForeverKey)
            SettingsFieldTimestamps.touch("retention.meetingKeepForever")
        }
    }
    @Published var conversationDays: Int {
        didSet {
            UserDefaults.standard.set(conversationDays, forKey: Self.conversationDaysKey)
            SettingsFieldTimestamps.touch("retention.conversationDays")
        }
    }
    @Published var conversationKeepForever: Bool {
        didSet {
            UserDefaults.standard.set(conversationKeepForever, forKey: Self.conversationKeepForeverKey)
            SettingsFieldTimestamps.touch("retention.conversationKeepForever")
        }
    }

    init() {
        let defaults = UserDefaults.standard
        let storedCaptureDays = defaults.integer(forKey: Self.captureItemDaysKey)
        captureItemDays = storedCaptureDays > 0 ? storedCaptureDays : 30
        captureItemKeepForever = defaults.bool(forKey: Self.captureItemKeepForeverKey)

        let storedMeetingDays = defaults.integer(forKey: Self.meetingDaysKey)
        meetingDays = storedMeetingDays > 0 ? storedMeetingDays : 90
        meetingKeepForever = defaults.bool(forKey: Self.meetingKeepForeverKey)

        let storedConversationDays = defaults.integer(forKey: Self.conversationDaysKey)
        conversationDays = storedConversationDays > 0 ? storedConversationDays : 30
        conversationKeepForever = defaults.bool(forKey: Self.conversationKeepForeverKey)
    }

    /// Runs retention now against `store`, using the current settings, calling
    /// `completion` with nil if the store is unavailable or the underlying DB call
    /// failed. Async (not a synchronous return) because the underlying
    /// `applyRetention` call must serialize through `CoreMemoryStore.accessQueue` — the
    /// same shared queue background meeting-summarization/digest calls use — and those
    /// can run for many seconds; a synchronous `.sync` here would beachball the app for
    /// an unbounded time if one of those happened to be in flight.
    func applyNow(store: CoreMemoryStore?, completion: @escaping (CoreRetentionResult?) -> Void) {
        guard let store else {
            completion(nil)
            return
        }
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let capturedCaptureItemDays = captureItemKeepForever ? nil : captureItemDays
        let capturedMeetingDays = meetingKeepForever ? nil : meetingDays
        let capturedConversationDays = conversationKeepForever ? nil : conversationDays
        CoreMemoryStore.accessQueue.async { [weak self] in
            let result = store.applyRetention(
                captureItemDays: capturedCaptureItemDays,
                meetingDays: capturedMeetingDays,
                conversationDays: capturedConversationDays,
                nowUnixMs: nowMs,
                recordingsDir: SpikePaths.root
            )
            DispatchQueue.main.async {
                if let result {
                    self?.log.info(
                        "Applied retention: \(result.captureItemsDeleted, privacy: .public) capture items, \(result.meetingsDeleted, privacy: .public) meetings deleted (\(result.deletedMeetingFiles.count, privacy: .public) meeting file set(s) cleaned up)"
                    )
                } else {
                    self?.log.error("Retention run failed (store unavailable or DB error)")
                }
                completion(result)
            }
        }
    }
}
