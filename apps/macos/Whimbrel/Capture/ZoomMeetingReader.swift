import AppKit
import ApplicationServices
import Foundation

/// Reads what Zoom's windows show, matched on the labels in the recorded fixtures
/// (WhimbrelTests/Fixtures/ZoomAX, Zoom Workplace 6.x, English UI).
enum ZoomMeetingReader {
    static let bundleId = "us.zoom.xos"

    /// Pure: every Zoom window → the state. Notices win over the meeting window behind them (a removal
    /// notice can sit over the last frame of the meeting). An unknown mic or camera reads as on, so Away
    /// Mode leaves rather than stay with the mic possibly live.
    static func state(from windows: [AXNodeSnapshot]) -> ZoomWindowState {
        let texts = windows.flatMap(labels)
        func shows(_ phrase: String) -> Bool { texts.contains { $0.localizedCaseInsensitiveContains(phrase) } }
        if shows("removed you from this meeting") { return .removed }
        if shows("host ended this meeting") || shows("meeting has been ended") { return .ended }
        if shows("Enter meeting passcode") { return .passcodePrompt }
        if shows("host will admit you") || shows("Waiting for the host") { return .waitingRoom }
        if shows("Sign in to join") { return .signInRequired }
        let meeting = windows.filter { ZoomLeaver.isMeetingWindowTitle($0.title) }
        guard !meeting.isEmpty else { return .notOpen }
        let buttons = meeting.flatMap { buttonLabels($0) }
        let micMuted = buttons.contains { ["Unmute", "Unmute me"].contains($0) } && !buttons.contains { ["Mute", "Mute me"].contains($0) }
        let videoOff = buttons.contains("Start video") || meeting.flatMap(labels).contains { $0.contains("currently off") && $0.hasPrefix("Video") }
        return .inMeeting(micMuted: micMuted, videoOff: videoOff)
    }

    /// The live state of Zoom's windows; `.notOpen` when Zoom isn't running or can't be read.
    static func currentState() -> ZoomWindowState {
        state(from: zoomWindows().map { AXNodeSnapshot.capture($0) })
    }

    // MARK: - Tree helpers

    static func labels(_ node: AXNodeSnapshot) -> [String] {
        [node.title, node.value, node.description].compactMap { $0 } + node.children.flatMap(labels)
    }

    private static func buttonLabels(_ node: AXNodeSnapshot) -> [String] {
        let own = node.role == "AXButton" ? [node.title, node.description].compactMap { $0 } : []
        return own + node.children.flatMap(buttonLabels)
    }

    static func zoomWindows() -> [AXUIElement] {
        guard let zoom = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).first else { return [] }
        var ref: CFTypeRef?
        guard AXUIElementCopyAttributeValue(AXUIElementCreateApplication(zoom.processIdentifier), kAXWindowsAttribute as CFString, &ref) == .success,
              let windows = ref as? [AXUIElement] else { return [] }
        return windows
    }

    static func meetingWindow() -> AXUIElement? {
        zoomWindows().first { ZoomLeaver.isMeetingWindowTitle(AXTextWalker.stringAttribute($0, kAXTitleAttribute)) }
    }

    /// Depth-first search of a live element for the first node that matches.
    static func find(in element: AXUIElement, depth: Int = 40, where match: (AXUIElement) -> Bool) -> AXUIElement? {
        if match(element) { return element }
        guard depth > 0 else { return nil }
        var ref: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &ref) == .success,
              let children = ref as? [AXUIElement] else { return nil }
        for child in children {
            if let hit = find(in: child, depth: depth - 1, where: match) { return hit }
        }
        return nil
    }

    static func role(_ e: AXUIElement) -> String? { AXTextWalker.stringAttribute(e, kAXRoleAttribute) }

    static func label(_ e: AXUIElement) -> [String] {
        [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute].compactMap { AXTextWalker.stringAttribute(e, $0) }
    }
}

/// Posts to the meeting chat ("Message everyone"), never to Zoom's AI chat: it only searches the meeting
/// window. Opens the chat panel first if needed, and confirms the message shows in the chat within 5 s.
final class ZoomChatPoster: ZoomChatPosting {
    func post(_ text: String) async -> Bool {
        guard let window = ZoomMeetingReader.meetingWindow() else { return false }
        let isCompose: (AXUIElement) -> Bool = { e in
            ZoomMeetingReader.role(e) == "AXTextArea" && ZoomMeetingReader.label(e).contains("Message everyone")
        }
        if ZoomMeetingReader.find(in: window, where: isCompose) == nil {
            guard let chat = ZoomMeetingReader.find(in: window, where: { ZoomMeetingReader.label($0).contains("Chat") }) else { return false }
            AXUIElementPerformAction(chat, kAXPressAction as CFString)
            try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
        guard let field = ZoomMeetingReader.find(in: window, where: isCompose) else { return false }
        AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
        guard AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, text as CFString) == .success else { return false }
        guard let send = ZoomMeetingReader.find(in: window, where: {
            ZoomMeetingReader.role($0) == "AXButton" && ZoomMeetingReader.label($0).contains("Send")
        }) else { return false }
        AXUIElementPerformAction(send, kAXPressAction as CFString)
        for _ in 0..<10 {
            try? await Task.sleep(nanoseconds: 500_000_000)
            let shown = ZoomMeetingReader.find(in: window, where: { e in
                ZoomMeetingReader.role(e) != "AXTextArea" && ZoomMeetingReader.label(e).contains { $0.contains(text) }
            })
            if shown != nil { return true }
        }
        return false
    }
}

/// The live reader the Away Mode coordinator polls.
struct LiveZoomStateReader: ZoomStateReading {
    func currentState() -> ZoomWindowState { ZoomMeetingReader.currentState() }
}
