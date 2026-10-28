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

    private let logger = JSONLLogger(url: SpikePaths.screenTextLog)
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

    private func captureFrontmostWindow() {
        guard isRunning,
              let app = NSWorkspace.shared.frontmostApplication,
              let bundleId = app.bundleIdentifier,
              !CaptureExclusions.shared.isExcluded(bundleId) else { return }

        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        var focusedWindowRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement, kAXFocusedWindowAttribute as CFString, &focusedWindowRef) == .success,
              let focusedWindowRef else { return }
        let windowElement = focusedWindowRef as! AXUIElement

        var titleRef: CFTypeRef?
        AXUIElementCopyAttributeValue(windowElement, kAXTitleAttribute as CFString, &titleRef)
        let windowTitle = (titleRef as? String) ?? app.localizedName ?? bundleId

        let extraction = AXTextWalker.extract(from: windowElement)
        var text = extraction.text
        var source = "ax"

        if text.trimmingCharacters(in: .whitespacesAndNewlines).count < 8 {
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
