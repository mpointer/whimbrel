import ApplicationServices
import AppKit
import CoreGraphics
import Foundation

/// FR-CAP-1..3: observes the frontmost window via the Accessibility API, extracts its
/// text on focus change and on debounced content-change notifications, and logs JSON
/// records to a local file. Falls back to Vision OCR when a window exposes no AX text.
///
/// The main actor only decides *whether* to read (running, frontmost app, exclusions) and
/// commits the result. The read itself (the Accessibility walk, the window image and Vision
/// OCR, redaction) runs on `ScreenTextReader`'s background queue: on the main thread a slow app
/// or a large window stalled every frame of Whimbrel's UI for as long as the read took
/// (perf review, 2026-11-27).
@MainActor
final class ScreenTextCaptureService: NSObject, ObservableObject {
    @Published private(set) var isRunning = false
    @Published private(set) var lastCapturedBundleId: String?
    @Published private(set) var lastCaptureSource: String?

    private let logger = JSONLLogger.screenText
    private let reader = ScreenTextReader()
    private var axObserver: AXObserver?
    private var debounceWorkItem: DispatchWorkItem?
    private var lastTextHash: [String: Int] = [:]
    private let debounceInterval: TimeInterval = 0.8
    /// One read at a time. A request that arrives while one is running is remembered and run once
    /// when it finishes, so a burst of notifications never queues a backlog of stale reads.
    private var readInFlight = false
    private var readRequested = false

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
        readRequested = false
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
        AXUIElementSetMessagingTimeout(appElement, Self.messagingTimeout)
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

    /// The most one accessibility request may wait for the frontmost app, in seconds. Without a
    /// limit, an app that stops answering (seen Nov 19 2026 with a test copy of Whimbrel) held the
    /// read for the system default of about 6 s. Matches `FocusedWindowReader.messagingTimeout`;
    /// `AppState.init` makes it the process-wide default at launch.
    nonisolated static let messagingTimeout: Float = 1.0

    /// Whether the frontmost app's text is read at all: never another copy of Whimbrel (a test or debug
    /// build has the same bundle id), whose windows are Whimbrel's own.
    nonisolated static func capturesText(of bundleId: String, ownBundleId: String? = Bundle.main.bundleIdentifier) -> Bool {
        bundleId != ownBundleId
    }

    private func captureFrontmostWindow() {
        guard isRunning,
              let app = NSWorkspace.shared.frontmostApplication,
              let bundleId = app.bundleIdentifier,
              Self.capturesText(of: bundleId),
              !CaptureExclusions.shared.isExcluded(bundleId) else { return }
        guard !readInFlight else {
            readRequested = true
            return
        }
        readInFlight = true
        let target = ScreenTextReader.Target(
            pid: app.processIdentifier,
            bundleId: bundleId,
            appName: app.localizedName,
            skipWebAreas: ChromiumAccessibility.bundleIDs.contains(bundleId),
            excludedDomains: CaptureExclusions.shared.excludedDomains
        )
        reader.read(target) { [weak self] reading in
            DispatchQueue.main.async { self?.finishRead(reading, of: bundleId) }
        }
    }

    /// Commits a finished read. Exclusions are checked again here, on the main actor, against the
    /// current settings: an app or site excluded while the read was running is still never written.
    private func finishRead(_ reading: ScreenTextReader.Reading?, of bundleId: String) {
        readInFlight = false
        defer {
            if readRequested {
                readRequested = false
                captureFrontmostWindow()
            }
        }
        guard isRunning, let reading, !CaptureExclusions.shared.isExcluded(bundleId) else { return }
        // An excluded site's page goes no further than the reader that saw it.
        if let url = reading.exclusionURL, CaptureExclusions.shared.isExcluded(url: url) { return }
        if lastTextHash[bundleId] == reading.textHash { return }
        lastTextHash[bundleId] = reading.textHash

        lastCapturedBundleId = bundleId
        lastCaptureSource = reading.source

        logger.append(ScreenTextRecord(
            timestamp: Date(),
            bundleId: bundleId,
            windowTitle: reading.redactedTitle,
            url: reading.redactedURL,
            text: reading.redactedText,
            source: reading.source
        ))
    }
}

/// Reads the frontmost window's text off the main thread, one read at a time on its own serial
/// queue. Everything it touches (Accessibility, CGWindow, Vision, the core's redaction) is safe off
/// the main thread; its only state, the OCR cache, is confined to `queue`.
final class ScreenTextReader: @unchecked Sendable {
    struct Target {
        let pid: pid_t
        let bundleId: String
        let appName: String?
        let skipWebAreas: Bool
        /// The excluded sites when the read started, so an excluded page is dropped before OCR.
        let excludedDomains: Set<String>
    }

    /// One read, credentials already redacted. `textHash` is of the unredacted text, as before.
    struct Reading {
        let redactedTitle: String
        let redactedURL: String?
        let redactedText: String
        let source: String
        let textHash: Int
        /// The page URL to check against excluded sites (`Extraction.url ?? exclusionURL`).
        let exclusionURL: String?
    }

    private let queue = DispatchQueue(label: "com.mpointer.kestrel.screen-text", qos: .utility)
    /// Per window: a fingerprint of the last image OCR read and the text it gave. A window whose
    /// pixels have not changed is not sent through Vision again. Only touched on `queue`.
    private var lastOCR: [CGWindowID: (fingerprint: UInt64, text: String?)] = [:]
    private static let ocrCacheLimit = 64

    func read(_ target: Target, completion: @escaping (Reading?) -> Void) {
        queue.async { [self] in
            completion(readNow(target))
        }
    }

    private func readNow(_ target: Target) -> Reading? {
        let timeout = ScreenTextCaptureService.messagingTimeout
        let appElement = AXUIElementCreateApplication(target.pid)
        // A limit on the app element applies to the elements read from it (the window and the walker's
        // children); the system-wide element sets the default for any other.
        AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), timeout)
        AXUIElementSetMessagingTimeout(appElement, timeout)
        var focusedWindowRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(appElement, kAXFocusedWindowAttribute as CFString, &focusedWindowRef) == .success,
              let focusedWindowRef else { return nil }
        let windowElement = focusedWindowRef as! AXUIElement
        AXUIElementSetMessagingTimeout(windowElement, timeout)

        var titleRef: CFTypeRef?
        AXUIElementCopyAttributeValue(windowElement, kAXTitleAttribute as CFString, &titleRef)
        let windowTitle = (titleRef as? String) ?? target.appName ?? target.bundleId

        let extraction = AXTextWalker.extract(from: windowElement, skipWebAreas: target.skipWebAreas)
        // An excluded site's page is dropped here, before OCR and before anything is redacted or
        // written; the main actor checks again against the current list when it commits.
        let exclusionURL = extraction.url ?? extraction.exclusionURL
        if let exclusionURL, CaptureExclusions.isExcluded(url: exclusionURL, domains: target.excludedDomains) { return nil }
        var text = extraction.text
        var source = "ax"

        if text.trimmingCharacters(in: .whitespacesAndNewlines).count < OCRFallback.minimumAccessibilityTextLength {
            let bounds = AXTextWalker.frame(of: windowElement)
            if let windowID = OCRFallback.windowID(forPID: target.pid, matchingBounds: bounds),
               let usableBounds = bounds,
               let ocrText = recognizeText(windowID: windowID, bounds: usableBounds),
               !ocrText.isEmpty {
                text = ocrText
                source = "ocr"
            }
        }

        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        // Credentials never reach disk: redacted before the capture log is written.
        return Reading(
            redactedTitle: CoreMemoryStore.redactSecrets(windowTitle),
            redactedURL: extraction.url.map(CoreMemoryStore.redactSecrets),
            redactedText: CoreMemoryStore.redactSecrets(text),
            source: source,
            textHash: text.hashValue,
            exclusionURL: exclusionURL
        )
    }

    /// OCR of the window, skipping Vision when the window's image is unchanged since its last OCR.
    private func recognizeText(windowID: CGWindowID, bounds: CGRect) -> String? {
        guard let image = OCRFallback.windowImage(windowID: windowID, bounds: bounds) else { return nil }
        let fingerprint = OCRFallback.fingerprint(of: image)
        if let fingerprint, let cached = lastOCR[windowID], cached.fingerprint == fingerprint {
            return cached.text
        }
        let text = TextRecognizer.recognize(image)
        if let fingerprint {
            if lastOCR.count >= Self.ocrCacheLimit { lastOCR.removeAll(keepingCapacity: true) }
            lastOCR[windowID] = (fingerprint, text)
        }
        return text
    }
}
