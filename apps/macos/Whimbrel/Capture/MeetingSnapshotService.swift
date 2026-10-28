import AppKit
import OSLog
import ScreenCaptureKit
import Vision

/// Where meeting frames come from. nil means nothing to capture this tick (no permission, excluded
/// app, window gone, capture failed).
protocol SnapshotFrameSource { func captureFrame() async -> CGImage? }

/// Samples the meeting window every `interval` while a meeting records and saves one PNG per shared
/// slide (plus a DB row with its OCR text). See docs/superpowers/specs for meeting snapshots.
@MainActor
final class MeetingSnapshotService {
    static let interval: TimeInterval = 5
    /// Posted on the main actor after a slide is saved or replaced; `userInfo["meetingId"]` is the Int64 id.
    static let didChange = Notification.Name("com.mpointer.kestrel.meetingSnapshots.didChange")
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "meeting-snapshots")

    private let meetingId: Int64
    private let meetingStart: Date
    private let meetingTitle: String
    private let source: SnapshotFrameSource
    private let storage: SnapshotStorage
    private let store: CoreMemoryStore
    private let faceDetector: (CGImage) -> [CGRect]
    private let textRecognizer: (CGImage) -> String?

    private var timer: Timer?
    private var stopped = false
    private var ticking = false
    private var previousSample: FrameFeatures?
    private var lastAnalysis: Analysis?
    private var lastKept: (features: FrameFeatures, id: Int64, url: URL, captured: Date)?
    private var folder: URL?
    private(set) var keptCount = 0
    /// Test seam: runs on the main actor right after a PNG is written, before the store call.
    var afterFileWritten: (() -> Void)?

    init(meetingId: Int64, meetingStart: Date, meetingTitle: String,
         source: SnapshotFrameSource, storage: SnapshotStorage,
         store: CoreMemoryStore, faceDetector: @escaping (CGImage) -> [CGRect] = VisionFaces.detect,
         textRecognizer: @escaping (CGImage) -> String? = TextRecognizer.recognize,
         existingFolder: URL? = nil) {
        self.meetingId = meetingId
        self.meetingStart = meetingStart
        self.meetingTitle = meetingTitle
        self.source = source
        self.storage = storage
        self.store = store
        self.faceDetector = faceDetector
        self.textRecognizer = textRecognizer
        // A continued meeting keeps writing into its first part's folder.
        self.folder = existingFolder
    }

    /// Also resumes after `stop()` (the setting turned back on mid-meeting).
    func start() {
        stopped = false
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: Self.interval, repeats: true) { [weak self] _ in
            Task { @MainActor in await self?.tick() }
        }
    }

    func stop() {
        stopped = true
        timer?.invalidate()
        timer = nil
    }

    func tick() async {
        guard !stopped, !ticking else { return }
        ticking = true
        defer { ticking = false }

        guard let frame = await source.captureFrame(), !stopped else { return }
        let captured = Date()
        let (faceDetector, textRecognizer, previous) = (faceDetector, textRecognizer, lastAnalysis)
        guard let analysis = await Task.detached(priority: .utility, operation: {
            Self.analyze(frame, previous: previous, faceDetector: faceDetector, textRecognizer: textRecognizer)
        }).value, !stopped else { return }
        lastAnalysis = analysis
        guard let current = analysis.features, let image = frame.cropping(to: current.crop) else { return }

        let decision = SnapshotFrameFilter.decide(current: current, previousSample: previousSample, lastKept: lastKept?.features)
        previousSample = current
        do {
            switch decision {
            case .skip, .waitForStable:
                return
            case .keepNew:
                try await keep(image, current, captured: captured)
            case .replaceLast:
                try await replaceLast(image, current)
            }
        } catch {
            Self.log.error("Snapshot not saved: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// One frame's analysis. `features` is nil for junk and participant-grid frames.
    struct Analysis {
        let width: Int
        let height: Int
        let luma: Luma
        let features: FrameFeatures?
    }

    /// Junk check, face-aware crop, then features of the cropped image. A frame that hasn't
    /// changed since `previous` reuses its result: no face detection, no OCR. nil only when the
    /// frame can't be read at all.
    nonisolated static func analyze(_ frame: CGImage, previous: Analysis?, faceDetector: (CGImage) -> [CGRect],
                                    textRecognizer: (CGImage) -> String?) -> Analysis? {
        guard let luma = Luma(thumbnailOf: frame) else { return nil }
        // Keep comparing against the analysed frame, so slow drift still triggers a fresh look.
        // Same pixel size too: a same-aspect resize matches the thumbnail but not the crop.
        if let previous, previous.width == frame.width, previous.height == frame.height, previous.luma.matches(luma, tolerance: SnapshotFrameFilter.unchangedPixelStep) { return previous }
        guard !SnapshotFrameFilter.isJunk(frame),
              let crop = SnapshotFrameFilter.contentCrop(frame, faces: faceDetector(frame)),
              let image = frame.cropping(to: crop) else {
            return Analysis(width: frame.width, height: frame.height, luma: luma, features: nil)
        }
        return Analysis(width: frame.width, height: frame.height, luma: luma, features: FrameFeatures(hash: SnapshotFrameFilter.dHash(image),
                                                            text: textRecognizer(image) ?? "", crop: crop))
    }

    private func keep(_ image: CGImage, _ features: FrameFeatures, captured: Date) async throws {
        let (folder, url) = try await write(image, captured: captured)
        self.folder = folder
        guard !discardIfStopped(url) else { return }
        let (store, meetingId) = (store, meetingId)
        let id: Int64? = await onStore {
            store.setMeetingSnapshotsDir(meetingId: meetingId, dir: folder.path)
            return store.insertMeetingSnapshot(meetingId: meetingId, capturedUnixMs: Int64(captured.timeIntervalSince1970 * 1000),
                                               filePath: url.path, width: image.width, height: image.height, ocrText: features.text)
        }
        guard let id else {
            try? FileManager.default.removeItem(at: url)
            throw CocoaError(.fileWriteUnknown)
        }
        lastKept = (features, id, url, captured)
        keptCount += 1
        postChange()
    }

    private func replaceLast(_ image: CGImage, _ features: FrameFeatures) async throws {
        guard let last = lastKept else { return }
        // Named by the slide's first appearance, so the folder stays in meeting order.
        let (_, url) = try await write(image, captured: last.captured)
        guard !discardIfStopped(url) else { return }
        let store = store
        let replaced = await onStore {
            store.replaceMeetingSnapshot(id: last.id, filePath: url.path, width: image.width, height: image.height, ocrText: features.text)
        }
        guard replaced else {
            try? FileManager.default.removeItem(at: url)
            throw CocoaError(.fileWriteUnknown)
        }
        try? FileManager.default.removeItem(at: last.url)
        lastKept = (features, last.id, url, last.captured)
        postChange()
    }

    private func postChange() {
        NotificationCenter.default.post(name: Self.didChange, object: nil, userInfo: ["meetingId": meetingId])
    }

    /// Writes the PNG into the meeting folder, creating the folder on first use.
    private func write(_ image: CGImage, captured: Date) async throws -> (folder: URL, file: URL) {
        let (storage, existing, start, title) = (storage, folder, meetingStart, meetingTitle)
        return try await Task.detached(priority: .utility) {
            let folder = try existing ?? storage.makeMeetingFolder(start: start, title: title)
            return (folder, try storage.writePNG(image, to: folder, captured: captured))
        }.value
    }

    /// A meeting that ended while the PNG was being written gets no new file or row.
    private func discardIfStopped(_ url: URL) -> Bool {
        afterFileWritten?()
        guard stopped else { return false }
        try? FileManager.default.removeItem(at: url)
        return true
    }

    /// `CoreMemoryStore` is not thread-safe; every call goes through its shared access queue.
    private func onStore<T>(_ body: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async { continuation.resume(returning: body()) }
        }
    }
}

enum VisionFaces {
    static func detect(_ image: CGImage) -> [CGRect] {
        let request = VNDetectFaceRectanglesRequest()
        do {
            try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
        } catch {
            return []
        }
        return (request.results ?? []).map { pixelRect($0.boundingBox, width: image.width, height: image.height) }
    }

    /// Vision's normalized, bottom-left-origin box → source pixels with a top-left origin.
    static func pixelRect(_ box: CGRect, width: Int, height: Int) -> CGRect {
        let w = CGFloat(width), h = CGFloat(height)
        return CGRect(x: box.minX * w, y: (1 - box.minY - box.height) * h, width: box.width * w, height: box.height * h)
    }
}

/// Captures the meeting app's window with ScreenCaptureKit. Never captures a whole display: the
/// user's own screen share is only seen through the meeting app's self-preview.
struct WindowFrameSource: SnapshotFrameSource {
    /// What to capture for a meeting with no detected app: a meeting app, or the frontmost
    /// browser window on meet.google.com.
    enum Target: Equatable { case app(String), meetBrowser, none }

    /// The detected meeting app; nil for calendar and manual meetings. Either way the target is
    /// resolved per tick (`resolveTarget`).
    let appBundleID: String?
    /// The meeting's conference provider, used when there is no usable detected app.
    let provider: MeetingProvider?
    let browserWindowProvider: () -> AXUIElement?

    static let zoomBundleID = "us.zoom.xos"
    static let slackBundleID = "com.tinyspeck.slackmacgap"

    /// The detected app, unless it is Slack: the detector reports the first running known app,
    /// and Slack is usually running for chat, not the call, so a Slack detection resolves like a
    /// calendar meeting. Then the link's app if it is running; with no usable link, the frontmost
    /// meeting app (never Slack) or a frontmost Meet tab. Otherwise nothing: guessing risks
    /// capturing something outside the meeting.
    static func resolveTarget(detected: String?, provider: MeetingProvider?, frontmost: String?, running: Set<String>,
                              meetWindowPresent: @autoclosure () -> Bool) -> Target {
        if let detected, detected != slackBundleID { return .app(detected) }
        let known = MeetingHeuristicDetector.knownMeetingAppBundleIDs
        switch provider {
        case .zoom:
            return running.contains(zoomBundleID) ? .app(zoomBundleID) : .none
        case .teams:
            let teams = known.filter { $0.hasPrefix("com.microsoft.teams") }
            if let frontmost, teams.contains(frontmost) { return .app(frontmost) }
            return teams.sorted().first(where: running.contains).map(Target.app) ?? .none
        case .googleMeet:
            return meetWindowPresent() ? .meetBrowser : .none
        case .webex, nil:
            if let frontmost, frontmost != slackBundleID, known.contains(frontmost) { return .app(frontmost) }
            return meetWindowPresent() ? .meetBrowser : .none
        }
    }

    func captureFrame() async -> CGImage? {
        let (bundleID, ax) = await MainActor.run { () -> (String?, AXUIElement?) in
            // Asked every tick, without prompting: permission can be revoked mid-meeting.
            guard CGPreflightScreenCaptureAccess() else { return (nil, nil) }
            let frontmost = NSWorkspace.shared.frontmostApplication?.bundleIdentifier
            let running = Set(NSWorkspace.shared.runningApplications.compactMap(\.bundleIdentifier))
            // ponytail: walks the frontmost browser's AX tree every tick when no detected app is used; cache per tick if it shows up in profiles.
            lazy var meetWindow = MeetingHeuristicDetector.resolveFrontmostGoogleMeetWindow()
            switch Self.resolveTarget(detected: appBundleID, provider: provider, frontmost: frontmost, running: running,
                                      meetWindowPresent: meetWindow != nil) {
            case .app(let id):
                return (id, BrowserMeetingCheck.knownBrowserBundleIDs.contains(id) ? browserWindowProvider() : nil)
            case .meetBrowser: return (frontmost, meetWindow)
            case .none: return (nil, nil)
            }
        }
        guard let bundleID,
              let content = try? await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        else { return nil }
        let windows = content.windows.filter { $0.owningApplication?.bundleIdentifier == bundleID && $0.isOnScreen }

        let window: SCWindow?
        if BrowserMeetingCheck.knownBrowserBundleIDs.contains(bundleID) {
            // A browser has many windows; only the one the meeting was detected in will do, and
            // only while its active tab is still the meeting (the title follows the tab).
            guard let ax, Self.browserShowsMeeting(url: BrowserMeetingCheck.currentURL(of: ax)) else { return nil }
            let title = Self.string(ax, kAXTitleAttribute)
            let frame = Self.frame(ax)
            window = windows.first { title?.isEmpty == false && $0.title == title }
                ?? windows.first { w in frame.map { Self.close(w.frame, $0) } ?? false }
        } else {
            // Layer 0 only: menu-bar extras and floating toolbars are not the meeting window.
            let normal = windows.filter { $0.windowLayer == 0 }
            window = Self.chooseWindow(bundleID: bundleID, windows: normal.map { ($0.title, $0.frame.width * $0.frame.height) })
                .map { normal[$0] }
        }
        guard let window, let owner = window.owningApplication?.bundleIdentifier,
              await !MainActor.run(body: { CaptureExclusions.shared.isExcluded(owner) }) else { return nil }

        let filter = SCContentFilter(desktopIndependentWindow: window)
        let info = SCShareableContent.info(for: filter)
        let config = SCStreamConfiguration()
        config.width = Int(info.contentRect.width * CGFloat(info.pointPixelScale))
        config.height = Int(info.contentRect.height * CGFloat(info.pointPixelScale))
        config.showsCursor = false
        config.ignoreShadowsSingleWindow = true
        return try? await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
    }

    /// Whether a browser window's active tab is a meeting (Meet, Teams or Zoom web).
    static func browserShowsMeeting(url: String?) -> Bool {
        MeetingHeuristicDetector.meetingURLProvider(url) != nil
    }

    /// Title prefixes of a native app's meeting window (R13). Best-known conventions, not
    /// verified against every app version; the manual pass checks them:
    /// - Zoom: "Zoom Meeting" (also "Zoom Meeting 40-Minutes") and "Zoom Webinar". The hub is
    ///   "Zoom" / "Zoom Workplace"; while the user shares, the meeting window is hidden.
    /// - Teams (new and classic): ad-hoc "Meeting with …", "Meeting in …", "Call with …",
    ///   "Meet now". The main window is "<Section> | … | Microsoft Teams". A scheduled
    ///   meeting's window carries the event subject, so it matches nothing and is not captured.
    /// - FaceTime, Webex: no pattern; the one-window rule applies.
    static let meetingWindowTitlePrefixes: [String: [String]] = {
        let teams = ["Meeting with ", "Meeting in ", "Call with ", "Meet now"]
        return [zoomBundleID: ["Zoom Meeting", "Zoom Webinar"], "com.microsoft.teams2": teams, "com.microsoft.teams": teams]
    }()

    /// The meeting window among an app's on-screen windows: the largest whose title matches the
    /// app's meeting pattern. An app with no known pattern may use its only window (R15); an app
    /// with patterns never does, since a lone non-matching window is its hub or main window
    /// (e.g. the Zoom hub while the user shares). Otherwise none: capture nothing rather than guess.
    static func chooseWindow(bundleID: String, windows: [(title: String?, area: CGFloat)]) -> Int? {
        let prefixes = meetingWindowTitlePrefixes[bundleID] ?? []
        let matching = windows.indices.filter { i in prefixes.contains { windows[i].title?.hasPrefix($0) == true } }
        if let best = matching.max(by: { windows[$0].area < windows[$1].area }) { return best }
        return prefixes.isEmpty && windows.count == 1 ? 0 : nil
    }

    private static func string(_ element: AXUIElement, _ attribute: String) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else { return nil }
        return value as? String
    }

    /// AX position/size are in the same top-left-origin global points as `SCWindow.frame`.
    private static func frame(_ element: AXUIElement) -> CGRect? {
        var pos: CFTypeRef?, size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &pos) == .success,
              AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &size) == .success,
              let pos, let size else { return nil }
        var origin = CGPoint.zero, extent = CGSize.zero
        // A misbehaving app can hand back any CF type here; never force-cast it.
        guard CFGetTypeID(pos) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID(),
              AXValueGetValue(pos as! AXValue, .cgPoint, &origin), AXValueGetValue(size as! AXValue, .cgSize, &extent)
        else { return nil }
        return CGRect(origin: origin, size: extent)
    }

    private static func close(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) <= 2 && abs(a.minY - b.minY) <= 2 && abs(a.width - b.width) <= 2 && abs(a.height - b.height) <= 2
    }
}
