import Foundation

/// "Remind me to tell people I'm recording". Off by default; local to this Mac. With the redesign flag on an absent
/// setting means on; a stored choice always wins, so anyone who turned it off keeps that.
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

    /// The value when nothing is stored: on with the redesign flag, off without.
    static func defaultEnabled(stored: Bool?, redesign: Bool) -> Bool { stored ?? redesign }

    init(defaults: UserDefaults = .standard, redesign: Bool = false) {
        self.defaults = defaults
        enabled = Self.defaultEnabled(stored: defaults.object(forKey: Self.enabledKey) == nil ? nil : defaults.bool(forKey: Self.enabledKey), redesign: redesign)
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
