import ApplicationServices
import AppKit
import Foundation

enum WindowReadResult: Equatable {
    case read(WindowContext)
    case excluded(appName: String)
    case unreadable(appName: String)
    case notPermitted
}

/// Source of the focused window's text. A seam so the reader's exclusion and fallback
/// rules are testable without Accessibility.
///
/// `Sendable` because the reader runs the synchronous methods off the main thread: AX IPC to
/// an unresponsive app and Vision OCR can block for seconds.
protocol FocusedElementProvider: Sendable {
    func frontmostApp() -> (bundleId: String, name: String, pid: pid_t)?
    func selectedText(pid: pid_t) -> String?
    func windowText(pid: pid_t) -> (title: String, text: String, url: String?)?
    func ocrText(pid: pid_t) async -> String?
    var accessibilityTrusted: Bool { get }
    /// Screen Recording permission; OCR is skipped without it.
    var screenCaptureAllowed: Bool { get }
}

/// On-demand read of the focused window for the hotkey panel's "ask about this window".
/// Exclusions are checked before any text is read; window text is never logged.
@MainActor
final class FocusedWindowReader {
    private let provider: FocusedElementProvider
    private let exclusions: CaptureExclusions

    init(provider: FocusedElementProvider, exclusions: CaptureExclusions = .shared) {
        self.provider = provider
        self.exclusions = exclusions
    }

    /// `frontmost` is the app that was frontmost before the panel opened.
    func read(frontmost: (bundleId: String, name: String, pid: pid_t)) async -> WindowReadResult {
        if exclusions.isExcluded(frontmost.bundleId) { return .excluded(appName: frontmost.name) }
        guard provider.accessibilityTrusted else { return .notPermitted }

        let pid = frontmost.pid
        let provider = self.provider
        let window = await Task.detached(priority: .userInitiated) { provider.windowText(pid: pid) }.value
        // A listed site excludes both its page text and any selection inside it; the
        // selection is not even read once the URL is excluded.
        if let url = window?.url, exclusions.isExcluded(url: url) { return .excluded(appName: frontmost.name) }

        let title = CoreMemoryStore.redactSecrets(window?.title ?? frontmost.name)

        let selection = await Task.detached(priority: .userInitiated) { provider.selectedText(pid: pid) }.value
        if let selection, !selection.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return .read(context(frontmost, title, selection, isSelection: true, source: .selection))
        }

        var text = window?.text ?? ""
        var source = WindowContext.Source.accessibility
        if text.trimmingCharacters(in: .whitespacesAndNewlines).count < OCRFallback.minimumAccessibilityTextLength,
           provider.screenCaptureAllowed,
           let ocr = await Task.detached(priority: .userInitiated, operation: { await provider.ocrText(pid: pid) }).value,
           !ocr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            text = ocr
            source = .ocr
        }
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return .unreadable(appName: frontmost.name)
        }
        return .read(context(frontmost, title, text, isSelection: false, source: source))
    }

    private func context(_ app: (bundleId: String, name: String, pid: pid_t), _ title: String, _ text: String,
                         isSelection: Bool, source: WindowContext.Source) -> WindowContext {
        WindowContext(appName: app.name, title: title, text: CoreMemoryStore.redactSecrets(text),
                      isSelection: isSelection, source: source)
    }
}

/// Real provider: system-wide focused element for the selection, `AXTextWalker` on the
/// app's focused window for visible text, `OCRFallback` for windows with no AX text.
/// Stateless (no stored properties), so safe to use from any thread.
struct AXFocusedElementProvider: FocusedElementProvider {
    /// Bounds each AX call so an unresponsive app cannot stall a read for the ~6 s default.
    private static let messagingTimeout: Float = 1.0

    var accessibilityTrusted: Bool { AXIsProcessTrusted() }
    var screenCaptureAllowed: Bool { CGPreflightScreenCaptureAccess() }

    func frontmostApp() -> (bundleId: String, name: String, pid: pid_t)? {
        guard let app = NSWorkspace.shared.frontmostApplication, let id = app.bundleIdentifier else { return nil }
        return (id, app.localizedName ?? id, app.processIdentifier)
    }

    private func appElement(_ pid: pid_t) -> AXUIElement {
        let element = AXUIElementCreateApplication(pid)
        AXUIElementSetMessagingTimeout(element, Self.messagingTimeout)
        return element
    }

    func selectedText(pid: pid_t) -> String? {
        // The target app's own focused element: the system-wide one belongs to the panel
        // while it is key, and the app's stays valid while it is inactive.
        var focusedRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement(pid), kAXFocusedUIElementAttribute as CFString, &focusedRef) == .success,
              let focusedRef, CFGetTypeID(focusedRef) == AXUIElementGetTypeID() else { return nil }
        let element = focusedRef as! AXUIElement
        AXUIElementSetMessagingTimeout(element, Self.messagingTimeout)
        // Password fields are never read.
        if AXTextWalker.stringAttribute(element, kAXSubroleAttribute) == kAXSecureTextFieldSubrole { return nil }
        return AXTextWalker.stringAttribute(element, kAXSelectedTextAttribute)
    }

    func windowText(pid: pid_t) -> (title: String, text: String, url: String?)? {
        guard let window = focusedWindow(pid: pid) else { return nil }
        let bundleId = NSRunningApplication(processIdentifier: pid)?.bundleIdentifier ?? ""
        let extraction = AXTextWalker.extract(from: window, skipWebAreas: ChromiumAccessibility.bundleIDs.contains(bundleId))
        return (AXTextWalker.stringAttribute(window, kAXTitleAttribute) ?? "",
                extraction.text,
                extraction.url ?? extraction.exclusionURL)
    }

    func ocrText(pid: pid_t) async -> String? {
        guard let window = focusedWindow(pid: pid) else { return nil }
        let bounds = AXTextWalker.frame(of: window)
        guard let bounds, let id = OCRFallback.windowID(forPID: pid, matchingBounds: bounds) else { return nil }
        return OCRFallback.recognizeText(windowID: id, bounds: bounds)
    }

    private func focusedWindow(pid: pid_t) -> AXUIElement? {
        var ref: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement(pid), kAXFocusedWindowAttribute as CFString, &ref) == .success,
              let ref, CFGetTypeID(ref) == AXUIElementGetTypeID() else { return nil }
        let window = ref as! AXUIElement
        AXUIElementSetMessagingTimeout(window, Self.messagingTimeout)
        return window
    }
}
