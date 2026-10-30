import XCTest
@testable import Whimbrel

/// The 32×32 duplicate-frame gate in front of the dHash/OCR filter: a frame is saved only if
/// at least 6 of its 32×32 grey pixels differ from the last saved frame's by more than 16.
@MainActor
final class SnapshotDedupeTests: XCTestCase {
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

    /// A white 32×32 image with the first `black` pixels black: drawn 1:1 into the gate's luma.
    private func cells(black: Int) -> Luma {
        var px = [UInt8](repeating: 255, count: 32 * 32)
        for i in 0..<black { px[i] = 0 }
        let ctx = CGContext(data: &px, width: 32, height: 32, bitsPerComponent: 8, bytesPerRow: 32,
                            space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
        return Luma(ctx.makeImage()!, width: SnapshotFrameFilter.dedupeSide, height: SnapshotFrameFilter.dedupeSide)!
    }

    func testIdenticalFramesDoNotDiffer() {
        XCTAssertFalse(SnapshotFrameFilter.differs(cells(black: 100), cells(black: 100)))
    }

    /// A clock tick or cursor blink: 1–3 cells (5 at the edge) is the same frame.
    func testAFewChangedCellsDoNotDiffer() {
        for n in [1, 2, 3, 5] {
            XCTAssertFalse(SnapshotFrameFilter.differs(cells(black: 0), cells(black: n)), "\(n) cells")
        }
    }

    func testSixChangedCellsDiffer() {
        XCTAssertTrue(SnapshotFrameFilter.differs(cells(black: 0), cells(black: 6)))   // the edge
        XCTAssertTrue(SnapshotFrameFilter.differs(cells(black: 0), cells(black: 52)))
    }

    /// The measured text-slide changes (14–35 of 1024 cells) are all saved.
    func testTextSlideChangesDiffer() {
        let pairs: [([String], [String])] = [
            (["Roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search"],
             ["Hiring plan", "Two engineers in Q1", "One designer", "Interview loop"]),
            (["Roadmap"], ["Hiring plan", "Two engineers in Q1", "One designer", "Interview loop"]),
            (["Roadmap", "Ship it"], ["Q3 goals", "Hire", "Launch beta"]),
        ]
        let side = SnapshotFrameFilter.dedupeSide
        for (a, b) in pairs {
            let la = Luma(SnapshotFixtures.slide(a), width: side, height: side)!
            let lb = Luma(SnapshotFixtures.slide(b), width: side, height: side)!
            XCTAssertTrue(SnapshotFrameFilter.differs(la, lb), "\(a.first!) → \(b.first!)")
        }
    }

    func testSmallGreyStepsDoNotCount() {
        var a = [UInt8](repeating: 100, count: 1024), b = [UInt8](repeating: 116, count: 1024)
        let luma = { (px: inout [UInt8]) -> Luma in
            let ctx = CGContext(data: &px, width: 32, height: 32, bitsPerComponent: 8, bytesPerRow: 32,
                                space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
            return Luma(ctx.makeImage()!, width: 32, height: 32)!
        }
        XCTAssertFalse(SnapshotFrameFilter.differs(luma(&a), luma(&b)), "16 grey levels is not a change")
    }

    // MARK: Service

    private final class Frames: SnapshotFrameSource {
        var frames: [CGImage]
        init(_ frames: [CGImage]) { self.frames = frames }
        func captureFrame() async -> CGImage? { frames.isEmpty ? nil : frames.removeFirst() }
    }

    private func makeService(_ frames: [CGImage], ocr: @escaping (CGImage) -> String?) -> MeetingSnapshotService {
        MeetingSnapshotService(
            meetingId: meetingId, meetingStart: Date(timeIntervalSince1970: 1), meetingTitle: "Standup",
            source: Frames(frames),
            storage: SnapshotStorage(root: tempDir.appendingPathComponent("root", isDirectory: true),
                                     fallbackRoot: tempDir.appendingPathComponent("fallback", isDirectory: true)),
            store: store, faceDetector: { _ in [] }, textRecognizer: ocr
        )
    }

    /// The same slide comes back after a blank frame and OCR reads it differently, so the
    /// dHash/OCR filter says "new slide"; the pixels are identical, so the gate saves nothing.
    func testServiceSavesIdenticalFramesOnce() async {
        let slide = SnapshotFixtures.slide(["Roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search"])
        let blank = SnapshotFixtures.solid()
        let reads = ["alpha beta gamma delta", "one two three four", "red green blue black"]
        var calls = 0
        let service = makeService([slide, slide, blank, slide, slide]) { _ in
            defer { calls += 1 }
            return reads[min(calls, reads.count - 1)]
        }
        for _ in 0..<5 { await service.tick() }

        XCTAssertEqual(calls, 2, "the slide was analysed twice: once per appearance")
        XCTAssertEqual(service.keptCount, 1)
        XCTAssertEqual(store.listMeetingSnapshots(meetingId: meetingId).count, 1)
    }

    func testServiceSavesASlideChange() async {
        let a = SnapshotFixtures.slide(["Roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search"])
        let b = SnapshotFixtures.slide(["Hiring plan", "Two engineers in Q1", "One designer", "Interview loop"])
        let service = makeService([a, a, b, b], ocr: TextRecognizer.recognize)
        for _ in 0..<4 { await service.tick() }

        XCTAssertEqual(service.keptCount, 2)
        let rows = store.listMeetingSnapshots(meetingId: meetingId)
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows.map { URL(fileURLWithPath: $0.filePath).pathExtension }, ["heic", "heic"])
    }
}
