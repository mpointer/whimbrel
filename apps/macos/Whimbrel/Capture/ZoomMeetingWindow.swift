import Foundation

/// What Zoom's meeting window is showing right now. Shared by the Away Mode
/// coordinator and the live Zoom readers.
enum ZoomWindowState: Equatable {
    case notOpen, waitingRoom, passcodePrompt, signInRequired
    case inMeeting(micMuted: Bool, videoOff: Bool)
    case ended, removed
}

protocol ZoomStateReading { func currentState() -> ZoomWindowState }
protocol ZoomChatPosting { func post(_ text: String) async -> Bool }
protocol ZoomLeaving { func leave() async -> Bool }
