import ApplicationServices
import AppKit
import Foundation

/// FR-CAP-1..3: observes the frontmost window via the Accessibility API, extracts its
/// text on focus change and on debounced content-change notifications, and logs JSON
/// records to a local file. Falls back to Vision OCR when a window exposes no AX text.
@MainActor
final class ScreenTextCaptureService: NSObject, ObservableObject {
    @Published private(set) var isRunning = false
    @Published private(set) var lastCapturedBundleId: String?
    @Published private(set) var lastCaptureSource: String?

    private let logger = JSONLLogger.screenText
    private var axObserver: AXObserver?
    private var debounceWorkItem: DispatchWorkItem?
    private var lastTextHash: [String: Int] = [:]
    private let debounceInterval: TimeInterval = 0.8

    func start() {
        guard !isRunning else { return }
        isRunning = true

        NSWorkspace.shared.notificationCenter.addObserver(
            self,
            selector: #selector(activeAppChanged),
            name: NSWorkspace.didActivateApplicationNotification,
            object: nil
        )

        if let app = NSWorkspace.shared.frontmostApplication {
            attach(to: app)
            captureFrontmostWindow()
        }
    }

    func stop() {
        guard isRunning else { return }
        isRunning = false
        NSWorkspace.shared.notificationCenter.removeObserver(self, name: NSWorkspace.didActivateApplicationNotification, object: nil)
        detachObserver()
        debounceWorkItem?.cancel()
    }

    @objc private func activeAppChanged(_ note: Notification) {
        guard isRunning, let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else { return }
        attach(to: app)
        captureFrontmostWindow()
    }

    private func attach(to app: NSRunningApplication) {
        detachObserver()
        let pid = app.processIdentifier

        var observer: AXObserver?
        let callback: AXObserverCallback = { _, _, _, refcon in
            guard let refcon else { return }
            let service = Unmanaged<ScreenTextCaptureService>.fromOpaque(refcon).takeUnretainedValue()
            Task { @MainActor in
                service.scheduleCapture()
            }
        }
        guard AXObserverCreate(pid, callback, &observer) == .success, let observer else { return }
        axObserver = observer

        let appElement = AXUIElementCreateApplication(pid)
        let refcon = Unmanaged.passUnretained(self).toOpaque()
        for name in [kAXFocusedWindowChangedNotification, kAXValueChangedNotification, kAXTitleChangedNotification] {
            AXObserverAddNotification(observer, appElement, name as CFString, refcon)
        }
        CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(observer), .defaultMode)
    }

    private func detachObserver() {
        if let axObserver {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(axObserver), .defaultMode)
        }
        axObserver = nil
    }

    private func scheduleCapture() {
        debounceWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.captureFrontmostWindow()
        }
        debounceWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + debounceInterval, execute: work)
    }

    /// The most one accessibility request may wait for the frontmost app, in seconds. This runs on the main
    /// thread: without a limit, an app that stops answering (seen Nov 19 2026 with a test copy of Whimbrel)
    /// froze Whimbrel with it. Matches `FocusedWindowReader.messagingTimeout`.
    static let messagingTimeout: Float = 1.0

    /// Whether the frontmost app's text is read at all: never another copy of Whimbrel (a test or debug
    /// build has the same bundle id), whose windows are Whimbrel's own.
    static func capturesText(of bundleId: String, ownBundleId: String? = Bundle.main.bundleIdentifier) -> Bool {
        bundleId != ownBundleId
    }

    private func captureFrontmostWindow() {
        guard isRunning,
              let app = NSWorkspace.shared.frontmostApplication,
              let bundleId = app.bundleIdentifier,
              Self.capturesText(of: bundleId),
              !CaptureExclusions.shared.isExcluded(bundleId) else { return }

        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        // A limit on the app element applies to the elements read from it (the window and the walker's
        // children); the system-wide element sets the default for any other.
        AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), Self.messagingTimeout)
        AXUIElementSetMessagingTimeout(appElement, Self.messagingTimeout)
        var focusedWindowRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement, kAXFocusedWindowAttribute as CFString, &focusedWindowRef) == .success,
              let focusedWindowRef else { return }
        let windowElement = focusedWindowRef as! AXUIElement
        AXUIElementSetMessagingTimeout(windowElement, Self.messagingTimeout)

        var titleRef: CFTypeRef?
        AXUIElementCopyAttributeValue(windowElement, kAXTitleAttribute as CFString, &titleRef)
        let windowTitle = (titleRef as? String) ?? app.localizedName ?? bundleId

        let extraction = AXTextWalker.extract(from: windowElement, skipWebAreas: ChromiumAccessibility.bundleIDs.contains(bundleId))
        // An excluded site's page is dropped here, before OCR and before anything is
        // written; the text the walker already read goes no further.
        if let url = extraction.url ?? extraction.exclusionURL, CaptureExclusions.shared.isExcluded(url: url) { return }
        var text = extraction.text
        var source = "ax"

        if text.trimmingCharacters(in: .whitespacesAndNewlines).count < OCRFallback.minimumAccessibilityTextLength {
            let bounds = AXTextWalker.frame(of: windowElement)
            if let windowID = OCRFallback.windowID(forPID: app.processIdentifier, matchingBounds: bounds),
               let usableBounds = bounds,
               let ocrText = OCRFallback.recognizeText(windowID: windowID, bounds: usableBounds),
               !ocrText.isEmpty {
                text = ocrText
                source = "ocr"
            }
        }

        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }

        let hash = text.hashValue
        if lastTextHash[bundleId] == hash { return }
        lastTextHash[bundleId] = hash

        lastCapturedBundleId = bundleId
        lastCaptureSource = source

        // Credentials never reach disk: redacted before the capture log is written.
        logger.append(ScreenTextRecord(
            timestamp: Date(),
            bundleId: bundleId,
            windowTitle: CoreMemoryStore.redactSecrets(windowTitle),
            url: extraction.url.map(CoreMemoryStore.redactSecrets),
            text: CoreMemoryStore.redactSecrets(text),
            source: source
        ))
    }
}
