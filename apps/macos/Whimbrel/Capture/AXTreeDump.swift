import AppKit
import ApplicationServices
import Foundation
import OSLog

/// A Codable copy of an Accessibility subtree: what `ZoomMeetingReader` reads, and (in DEBUG builds)
/// what the dump below records as test fixtures.
struct AXNodeSnapshot: Codable, Equatable {
    var role: String?
    var subrole: String?
    var title: String?
    var value: String?
    var description: String?
    var identifier: String?
    var children: [AXNodeSnapshot]

    /// Same depth limit and traversal shape as `AXTextWalker`; no node cap, since a
    /// fixture wants the whole tree.
    static func capture(_ element: AXUIElement, maxDepth: Int = 40) -> AXNodeSnapshot {
        func attr(_ name: String) -> String? { AXTextWalker.stringAttribute(element, name) }
        var children: [AXNodeSnapshot] = []
        var childrenRef: CFTypeRef?
        if maxDepth > 1,
           AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
           let kids = childrenRef as? [AXUIElement] {
            children = kids.map { capture($0, maxDepth: maxDepth - 1) }
        }
        return AXNodeSnapshot(
            role: attr(kAXRoleAttribute), subrole: attr(kAXSubroleAttribute), title: attr(kAXTitleAttribute),
            value: attr(kAXValueAttribute), description: attr(kAXDescriptionAttribute),
            identifier: attr(kAXIdentifierAttribute), children: children
        )
    }
}

#if DEBUG
enum AXTreeDump {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "ax-dump")

    /// Asks for a label, then writes every Zoom window to `~/Desktop/zoom-ax-<label>-<timestamp>.json`.
    @MainActor static func dumpZoomWindows() {
        guard AXIsProcessTrusted() else { log.error("AX dump skipped: Accessibility permission is missing."); return }
        guard let zoom = NSRunningApplication.runningApplications(withBundleIdentifier: "us.zoom.xos").first else {
            log.error("AX dump skipped: Zoom (us.zoom.xos) is not running."); return
        }

        let alert = NSAlert()
        alert.messageText = "Dump Zoom window (AX)"
        alert.informativeText = "Short label for this recording, e.g. chat-panel-open."
        alert.addButton(withTitle: "Dump")
        alert.addButton(withTitle: "Cancel")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        alert.accessoryView = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let label = field.stringValue.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: " ", with: "-")

        let app = AXUIElementCreateApplication(zoom.processIdentifier)
        var windowsRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &windowsRef) == .success,
              let windows = windowsRef as? [AXUIElement], !windows.isEmpty else {
            log.error("AX dump skipped: Zoom has no readable windows."); return
        }

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let stamp = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
        let url = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Desktop/zoom-ax-\(label.isEmpty ? "unlabeled" : label)-\(stamp).json")
        do {
            try encoder.encode(windows.map { AXNodeSnapshot.capture($0) }).write(to: url)
            log.notice("AX dump wrote \(url.path, privacy: .public)")
        } catch {
            log.error("AX dump failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// For notices that close too fast to dump by hand (removed by the host, meeting ended): for `seconds`,
    /// snapshots every Zoom window twice a second and writes `zoom-ax-auto-<n>-<timestamp>.json` each time
    /// what they show changes.
    @MainActor static func recordZoomWindows(seconds: Int = 60) {
        guard AXIsProcessTrusted() else { log.error("AX record skipped: Accessibility permission is missing."); return }
        Task { @MainActor in
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            var last: [AXNodeSnapshot]? = nil
            var n = 0
            for _ in 0..<(seconds * 2) {
                if let zoom = NSRunningApplication.runningApplications(withBundleIdentifier: "us.zoom.xos").first {
                    let app = AXUIElementCreateApplication(zoom.processIdentifier)
                    var windowsRef: CFTypeRef?
                    if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &windowsRef) == .success,
                       let windows = windowsRef as? [AXUIElement] {
                        let snap = windows.map { AXNodeSnapshot.capture($0) }
                        if snap != last {
                            last = snap
                            n += 1
                            let stamp = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
                            let url = FileManager.default.homeDirectoryForCurrentUser
                                .appendingPathComponent("Desktop/zoom-ax-auto-\(n)-\(stamp).json")
                            try? encoder.encode(snap).write(to: url)
                        }
                    }
                }
                try? await Task.sleep(nanoseconds: 500_000_000)
            }
            log.notice("AX record finished: \(n, privacy: .public) snapshots")
        }
    }
}
#endif
