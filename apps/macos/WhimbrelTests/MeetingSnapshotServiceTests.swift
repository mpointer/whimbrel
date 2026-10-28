import XCTest
@testable import Whimbrel

/// Scripted frames: each tick takes the next entry; nil (or running out) means "nothing to capture".
private final class FakeFrameSource: SnapshotFrameSource {
    var frames: [(image: CGImage, faces: [CGRect])?]
    private(set) var currentFaces: [CGRect] = []

    init(_ frames: [(image: CGImage, faces: [CGRect])?]) { self.frames = frames }

    func captureFrame() async -> CGImage? {
        guard !frames.isEmpty, let next = frames.removeFirst() else { return nil }
        currentFaces = next.faces
        return next.image
    }
}

@MainActor
final class MeetingSnapshotServiceTests: XCTestCase {
    private let roadmap = ["Roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search"]
    private var tempDir: URL!
    private var store: CoreMemoryStore!
    private var meetingId: Int64!

    override func setUp() async throws {
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        store = CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
        meetingId = store.insertMeeting(CoreNewMeeting(
            deviceId: "dev", startUnixMs: 1000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .calendar, audioPath: nil, status: .recording
        ))!
    }

    override func tearDown() async throws {
        store = nil
        try? FileManager.default.removeItem(at: tempDir)
    }

    private var root: URL { tempDir.appendingPathComponent("root", isDirectory: true) }

    private func makeService(_ source: FakeFrameSource) -> MeetingSnapshotService {
        MeetingSnapshotService(
            meetingId: meetingId, meetingStart: Date(timeIntervalSince1970: 1), meetingTitle: "Standup",
            source: source,
            storage: SnapshotStorage(root: root, fallbackRoot: tempDir.appendingPathComponent("fallback", isDirectory: true)),
            store: store, faceDetector: { _ in source.currentFaces }
        )
    }

    private func run(_ service: MeetingSnapshotService, ticks: Int) async {
        for _ in 0..<ticks { await service.tick() }
    }

    /// PNGs in every meeting folder under the root (the marker file is not a snapshot).
    private func pngs() -> [URL] {
        let folders = (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        return folders.flatMap { (try? FileManager.default.contentsOfDirectory(at: $0, includingPropertiesForKeys: nil)) ?? [] }
            .filter { $0.pathExtension == "png" }
            .map { $0.resolvingSymlinksInPath() }  // listings say /private/var; the service stored /var
    }

    private func frame(_ image: CGImage) -> (image: CGImage, faces: [CGRect])? { (image, []) }

    /// Counts Vision calls; runs on the analysis task, one tick at a time.
    private final class VisionCalls { var faces = 0, ocr = 0 }

    private func makeCountingService(_ source: FakeFrameSource, _ calls: VisionCalls) -> MeetingSnapshotService {
        MeetingSnapshotService(
            meetingId: meetingId, meetingStart: Date(timeIntervalSince1970: 1), meetingTitle: "Standup",
            source: source,
            storage: SnapshotStorage(root: root, fallbackRoot: tempDir.appendingPathComponent("fallback", isDirectory: true)),
            store: store,
            faceDetector: { _ in calls.faces += 1; return source.currentFaces },
            textRecognizer: { calls.ocr += 1; return TextRecognizer.recognize($0) }
        )
    }

    /// I7: a frame that hasn't changed since the last tick reuses that tick's analysis.
    func testUnchangedFrameSkipsFaceDetectionAndOCR() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let calls = VisionCalls()
        let service = makeCountingService(FakeFrameSource(Array(repeating: frame(slide), count: 4)), calls)
        await run(service, ticks: 4)

        XCTAssertEqual(calls.ocr, 1)
        XCTAssertEqual(calls.faces, 1)
        XCTAssertEqual(service.keptCount, 1)
        XCTAssertTrue(store.listMeetingSnapshots(meetingId: meetingId).first?.ocrText.contains("Calendar sync") ?? false)
    }

    /// …but a build (one more bullet) is a changed frame: it is analyzed and replaces the last file.
    func testBuildIsStillAnalyzedAfterReuse() async {
        let two = SnapshotFixtures.slideBuild(roadmap, shown: 2), three = SnapshotFixtures.slideBuild(roadmap, shown: 3)
        let calls = VisionCalls()
        let service = makeCountingService(FakeFrameSource([frame(two), frame(two), frame(three), frame(three)]), calls)
        await run(service, ticks: 4)

        XCTAssertEqual(calls.ocr, 2)
        let rows = store.listMeetingSnapshots(meetingId: meetingId)
        XCTAssertEqual(rows.count, 1)
        XCTAssertTrue(rows.first?.ocrText.contains("Calendar sync") ?? false, rows.first?.ocrText ?? "no row")
    }

    /// A resize that keeps the aspect ratio can leave the luma thumbnail unchanged while the pixel
    /// size changes: the old crop would be wrong, so the frame is analyzed again.
    func testResizedFrameIsReanalyzed() async {
        let calls = VisionCalls()
        // Blocks aligned to the thumbnail grid downsample identically at both sizes.
        let small = checkerboard(width: 1280, height: 720, block: 80)
        let large = checkerboard(width: 2560, height: 1440, block: 160)
        let service = makeCountingService(FakeFrameSource([frame(small), frame(small), frame(large), frame(large)]), calls)
        await run(service, ticks: 4)

        XCTAssertEqual(calls.faces, 2)
    }

    private func checkerboard(width: Int, height: Int, block: Int) -> CGImage {
        let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.setFillColor(gray: 1, alpha: 1)
        ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
        ctx.setFillColor(gray: 0, alpha: 1)
        for y in stride(from: 0, to: height, by: block) {
            for x in stride(from: 0, to: width, by: block) where (x / block + y / block) % 2 == 0 {
                ctx.fill(CGRect(x: x, y: y, width: block, height: block))
            }
        }
        return ctx.makeImage()!
    }

    /// A build keeps the file name of the slide's first appearance, not the build's time.
    func testBuildKeepsFirstAppearanceTimeInName() async {
        let two = SnapshotFixtures.slideBuild(roadmap, shown: 2), three = SnapshotFixtures.slideBuild(roadmap, shown: 3)
        let service = makeService(FakeFrameSource([frame(two), frame(two), frame(three), frame(three)]))
        await run(service, ticks: 2)
        let first = pngs().first?.lastPathComponent
        try? await Task.sleep(nanoseconds: 1_100_000_000)
        await run(service, ticks: 2)

        let name = pngs().first?.lastPathComponent ?? ""
        XCTAssertTrue(name.hasPrefix(String((first ?? "x").prefix(8))), "\(name) vs \(first ?? "nil")")
        let row = store.listMeetingSnapshots(meetingId: meetingId).first
        XCTAssertEqual(row?.filePath, pngs().first?.path)
    }

    /// A continued meeting writes into its existing folder rather than making a new one.
    func testExistingFolderIsWrittenInto() async throws {
        let existing = root.appendingPathComponent("earlier-part", isDirectory: true)
        try FileManager.default.createDirectory(at: existing, withIntermediateDirectories: true)
        let slide = SnapshotFixtures.slide(roadmap)
        let service = MeetingSnapshotService(
            meetingId: meetingId, meetingStart: Date(timeIntervalSince1970: 1), meetingTitle: "Standup",
            source: FakeFrameSource([frame(slide), frame(slide)]),
            storage: SnapshotStorage(root: root, fallbackRoot: tempDir.appendingPathComponent("fallback", isDirectory: true)),
            store: store, faceDetector: { _ in [] }, existingFolder: existing
        )
        await run(service, ticks: 2)

        XCTAssertEqual(service.keptCount, 1)
        XCTAssertEqual(pngs().map { $0.deletingLastPathComponent().lastPathComponent }, ["earlier-part"])
        XCTAssertEqual(store.meetingSnapshotsInfo(meetingId: meetingId)?.dir, existing.path)
    }

    /// Turning snapshots off pauses a meeting's service; turning them back on resumes it.
    func testStartAfterStopResumesTicks() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let service = makeService(FakeFrameSource(Array(repeating: frame(slide), count: 3)))
        service.stop()
        service.start()
        defer { service.stop() }
        await run(service, ticks: 3)

        XCTAssertEqual(service.keptCount, 1)
    }

    /// The meeting page refreshes its strip when a slide is saved.
    func testKeepingASlidePostsChange() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let service = makeService(FakeFrameSource(Array(repeating: frame(slide), count: 3)))
        let id = meetingId!
        let posted = expectation(forNotification: MeetingSnapshotService.didChange, object: nil) { note in
            (note.userInfo?["meetingId"] as? Int64) == id
        }
        await run(service, ticks: 3)
        await fulfillment(of: [posted], timeout: 1)
    }

    func testStableNewSlideIsKeptOnceAcrossTicks() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let service = makeService(FakeFrameSource(Array(repeating: frame(slide), count: 4)))
        await run(service, ticks: 4)

        XCTAssertEqual(service.keptCount, 1)
        XCTAssertEqual(pngs().count, 1)
        let rows = store.listMeetingSnapshots(meetingId: meetingId)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows.first?.filePath, pngs().first?.path)
        XCTAssertEqual(rows.first?.width, slide.width)
        XCTAssertTrue(rows.first?.ocrText.contains("Calendar sync") ?? false)
        XCTAssertEqual(store.meetingSnapshotsInfo(meetingId: meetingId)?.dir, pngs().first?.deletingLastPathComponent().path)
    }

    func testBuildReplacesPreviousFile() async {
        let two = SnapshotFixtures.slideBuild(roadmap, shown: 2), three = SnapshotFixtures.slideBuild(roadmap, shown: 3)
        let service = makeService(FakeFrameSource([frame(two), frame(two), frame(three), frame(three)]))
        await run(service, ticks: 4)

        XCTAssertEqual(pngs().count, 1)
        let rows = store.listMeetingSnapshots(meetingId: meetingId)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows.first?.filePath, pngs().first?.path)
        XCTAssertTrue(rows.first?.ocrText.contains("Calendar sync") ?? false, rows.first?.ocrText ?? "no row")
    }

    func testFaceGridAndBlankFramesKeepNothing() async {
        let grid = SnapshotFixtures.faceGrid()
        let blank = SnapshotFixtures.solid()
        let service = makeService(FakeFrameSource([grid, grid, grid, frame(blank), frame(blank), frame(blank)]))
        await run(service, ticks: 6)

        XCTAssertEqual(service.keptCount, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
        XCTAssertTrue(store.listMeetingSnapshots(meetingId: meetingId).isEmpty)
        XCTAssertNil(store.meetingSnapshotsInfo(meetingId: meetingId)?.dir)
    }

    func testCaptureFailureSkipsTickWithoutStopping() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let service = makeService(FakeFrameSource([nil, nil, nil, frame(slide), frame(slide)]))
        await run(service, ticks: 4)
        XCTAssertEqual(service.keptCount, 0)  // first sample only; not yet stable
        await service.tick()
        XCTAssertEqual(service.keptCount, 1)
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId).count, 1)
    }

    func testStopPreventsFurtherTicks() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let service = makeService(FakeFrameSource(Array(repeating: frame(slide), count: 3)))
        service.start()
        service.stop()
        await run(service, ticks: 3)

        XCTAssertEqual(service.keptCount, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
    }

    func testStopWhileWritingNewSlideLeavesNoFileOrRow() async {
        let slide = SnapshotFixtures.slide(roadmap)
        let service = makeService(FakeFrameSource([frame(slide), frame(slide)]))
        await service.tick()
        service.afterFileWritten = { [unowned service] in service.stop() }
        await service.tick()

        XCTAssertEqual(service.keptCount, 0)
        XCTAssertTrue(pngs().isEmpty)
        XCTAssertTrue(store.listMeetingSnapshots(meetingId: meetingId).isEmpty)
    }

    func testStopWhileWritingBuildKeepsPreviousFileAndRow() async {
        let two = SnapshotFixtures.slideBuild(roadmap, shown: 2), three = SnapshotFixtures.slideBuild(roadmap, shown: 3)
        let service = makeService(FakeFrameSource([frame(two), frame(two), frame(three), frame(three)]))
        await run(service, ticks: 3)
        let before = store.listMeetingSnapshots(meetingId: meetingId)
        XCTAssertEqual(before.count, 1)
        service.afterFileWritten = { [unowned service] in service.stop() }
        await service.tick()

        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId), before)
        XCTAssertEqual(pngs().map(\.path), before.map(\.filePath))
        XCTAssertFalse(before.first?.ocrText.contains("Calendar sync") ?? true)
    }

    // MARK: - Target resolution for meetings with no detected app

    private typealias Target = WindowFrameSource.Target
    private func target(_ provider: MeetingProvider?, frontmost: String?, running: Set<String> = [],
                        meet: Bool = false, detected: String? = nil) -> Target {
        WindowFrameSource.resolveTarget(detected: detected, provider: provider, frontmost: frontmost,
                                        running: running, meetWindowPresent: meet)
    }

    private let slack = "com.tinyspeck.slackmacgap", zoom = "us.zoom.xos"

    /// C1: Slack is the first running known app far more often than it is the call, so a
    /// Slack detection resolves like a calendar meeting and never targets Slack itself.
    func testDetectedSlackNeverTargetsSlack() {
        XCTAssertEqual(target(nil, frontmost: slack, running: [slack, zoom], detected: slack), Target.none)
        XCTAssertEqual(target(nil, frontmost: zoom, running: [slack, zoom], detected: slack), .app(zoom))
        XCTAssertEqual(target(nil, frontmost: "com.apple.Safari", running: [slack], meet: true, detected: slack), .meetBrowser)
    }

    func testDetectedNonSlackAppIsTargetedDirectly() {
        XCTAssertEqual(target(nil, frontmost: slack, running: [slack, zoom], detected: zoom), .app(zoom))
        XCTAssertEqual(target(.googleMeet, frontmost: nil, detected: "com.apple.Safari"), .app("com.apple.Safari"))
    }

    // MARK: - Browser meeting gate (C2)

    func testBrowserWindowIsCapturedOnlyOnAMeetingURL() {
        XCTAssertTrue(WindowFrameSource.browserShowsMeeting(url: "https://meet.google.com/abc-defg-hij"))
        XCTAssertFalse(WindowFrameSource.browserShowsMeeting(url: "https://mail.google.com/mail/u/0/#inbox"))
        XCTAssertFalse(WindowFrameSource.browserShowsMeeting(url: "https://news.example.com/?next=meet.google.com"))
        XCTAssertFalse(WindowFrameSource.browserShowsMeeting(url: nil))
    }

    // MARK: - Native-app window choice (I5, R13)

    private func choose(_ bundleID: String, _ windows: [(String?, CGFloat)]) -> Int? {
        WindowFrameSource.chooseWindow(bundleID: bundleID, windows: windows.map { (title: $0.0, area: $0.1) })
    }

    func testZoomPicksMeetingWindowOverLargerHub() {
        XCTAssertEqual(choose(zoom, [("Zoom Workplace", 2_000_000), ("Zoom Meeting", 1_000_000)]), 1)
        XCTAssertEqual(choose(zoom, [("Zoom Webinar", 900_000), ("Zoom", 2_000_000)]), 0)
    }

    func testTeamsPicksMeetingWindowOverMainWindow() {
        XCTAssertEqual(choose("com.microsoft.teams2", [("Chat | Jane Doe | Microsoft Teams", 2_000_000),
                                                      ("Meeting with Jane Doe | Microsoft Teams", 1_000_000)]), 1)
        XCTAssertEqual(choose("com.microsoft.teams2", [("Calendar | Microsoft Teams", 2_000_000),
                                                      ("Call with Sam Lee | Microsoft Teams", 500_000)]), 1)
    }

    func testTwoNonMatchingWindowsCaptureNothing() {
        XCTAssertNil(choose(zoom, [("Zoom Workplace", 2_000_000), ("Settings", 100_000)]))
        XCTAssertNil(choose("com.microsoft.teams2", [("Chat | Jane Doe | Microsoft Teams", 2_000_000),
                                                    ("Calendar | Microsoft Teams", 1_000_000)]))
        XCTAssertNil(choose(zoom, []))
    }

    /// R15: the one-window rule is only for apps with no known meeting title.
    func testSingleWindowIsUsedOnlyForAppsWithoutTitlePatterns() {
        XCTAssertEqual(choose("com.apple.FaceTime", [("FaceTime", 1_000_000)]), 0)
        XCTAssertNil(choose(zoom, [(nil, 1_000_000)]))
        XCTAssertNil(choose(zoom, [("Zoom Workplace", 1_000_000)]), "a lone Zoom hub (user sharing) is not the meeting")
        XCTAssertNil(choose("com.microsoft.teams2", [("Chat | Jane Doe | Microsoft Teams", 2_000_000)]))
    }

    func testZoomLinkPicksZoomEvenWithSlackRunning() {
        XCTAssertEqual(target(.zoom, frontmost: "com.tinyspeck.slackmacgap",
                              running: ["com.tinyspeck.slackmacgap", "us.zoom.xos"]), .app("us.zoom.xos"))
    }

    func testMeetLinkPicksBrowserWindow() {
        XCTAssertEqual(target(.googleMeet, frontmost: "com.google.Chrome", running: ["com.google.Chrome"], meet: true), .meetBrowser)
        XCTAssertEqual(target(.googleMeet, frontmost: "us.zoom.xos", running: ["us.zoom.xos"]), Target.none)
    }

    func testNoLinkWithSlackFrontmostCapturesNothing() {
        XCTAssertEqual(target(nil, frontmost: "com.tinyspeck.slackmacgap", running: ["com.tinyspeck.slackmacgap"]), Target.none)
    }

    func testNoLinkWithZoomFrontmostPicksZoom() {
        XCTAssertEqual(target(nil, frontmost: "us.zoom.xos", running: ["us.zoom.xos"]), .app("us.zoom.xos"))
    }

    func testNoLinkAndNothingFrontmostCapturesNothing() {
        XCTAssertEqual(target(nil, frontmost: nil), Target.none)
        XCTAssertEqual(target(nil, frontmost: "com.apple.finder", running: ["us.zoom.xos"]), Target.none)
    }

    /// Vision boxes are normalized with a bottom-left origin; the filter wants top-left pixels.
    func testVisionFaceBoxConvertsToTopLeftPixels() {
        XCTAssertEqual(VisionFaces.pixelRect(CGRect(x: 0.1, y: 0.2, width: 0.3, height: 0.4), width: 1000, height: 500),
                       CGRect(x: 100, y: 200, width: 300, height: 200))
        // A face at the very bottom of the frame in Vision terms sits at the bottom in pixels too.
        XCTAssertEqual(VisionFaces.pixelRect(CGRect(x: 0, y: 0, width: 0.5, height: 0.25), width: 800, height: 400),
                       CGRect(x: 0, y: 300, width: 400, height: 100))
    }
}
