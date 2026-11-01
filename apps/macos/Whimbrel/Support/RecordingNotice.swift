import Foundation

/// "Remind me to tell people I'm recording". Off by default; local to this Mac.
final class RecordingNoticeSettings: ObservableObject {
    private static let enabledKey = "com.mpointer.kestrel.recordingNoticeEnabled"
    private static let textKey = "com.mpointer.kestrel.recordingNoticeText"
    private let defaults: UserDefaults

    @Published var enabled: Bool {
        didSet { defaults.set(enabled, forKey: Self.enabledKey) }
    }
    /// Empty means the default wording.
    @Published var customText: String {
        didSet { defaults.set(customText, forKey: Self.textKey) }
    }
    /// The meeting whose card was dismissed, so a rebuilt view doesn't bring it back.
    @Published var dismissedMeetingId: Int64?

    var text: String {
        customText.isEmpty ? String(localized: "I'm recording this meeting with Whimbrel, on my own computer, for my notes.") : customText
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.bool(forKey: Self.enabledKey)
        customText = defaults.string(forKey: Self.textKey) ?? ""
    }
}

/// A chat line where the platform allows it, otherwise a reminder to say it.
enum RecordingNotice {
    enum Delivery: Equatable { case posted, remind }

    static func deliver(_ text: String, chat: ZoomChatPosting?) async -> Delivery {
        guard let chat else { return .remind }
        return await chat.post(text) ? .posted : .remind
    }
}
