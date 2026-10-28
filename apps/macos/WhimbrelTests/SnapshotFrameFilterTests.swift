import CoreGraphics
import XCTest
@testable import Whimbrel

final class SnapshotFrameFilterTests: XCTestCase {
    private let roadmap = ["Roadmap", "Ship meeting snapshots", "Calendar sync", "Faster search"]

    // MARK: - isJunk

    func testSolidFrameIsJunk() {
        XCTAssertTrue(SnapshotFrameFilter.isJunk(SnapshotFixtures.solid()))
    }

    func testSpinnerFrameIsJunk() {
        XCTAssertTrue(SnapshotFrameFilter.isJunk(SnapshotFixtures.spinner()))
    }

    func testTextSlideIsNotJunk() {
        XCTAssertFalse(SnapshotFrameFilter.isJunk(SnapshotFixtures.slide(roadmap)))
    }

    func testDarkThemeSlideIsNotJunk() {
        XCTAssertFalse(SnapshotFrameFilter.isJunk(SnapshotFixtures.slide(roadmap, background: 0.35, ink: 0.95)))
    }

    func testCrossFadeIsJunk() {
        let dark = SnapshotFixtures.slide(["Agenda", "Intros", "Demo", "Questions"], background: 0, ink: 1)
        XCTAssertTrue(SnapshotFrameFilter.isJunk(SnapshotFixtures.blend(dark, SnapshotFixtures.slide(roadmap))))
    }

    // MARK: - contentCrop

    func testFaceGridHasNoCrop() {
        let grid = SnapshotFixtures.faceGrid(columns: 2, rows: 2)
        XCTAssertEqual(grid.faces.count, 4)
        XCTAssertNil(SnapshotFrameFilter.contentCrop(grid.image, faces: grid.faces))
    }

    func testSlideWithFilmstripCropsToSlide() throws {
        let frame = SnapshotFixtures.slideWithFilmstrip()
        let crop = try XCTUnwrap(SnapshotFrameFilter.contentCrop(frame.image, faces: frame.faces))
        XCTAssertTrue(frame.slide.contains(crop), "crop \(crop) leaves slide \(frame.slide)")
        let covered = crop.intersection(frame.slide)
        XCTAssertGreaterThanOrEqual(covered.width * covered.height, 0.9 * frame.slide.width * frame.slide.height)
    }

    func testSlideWithTopFilmstripCropsToSlide() throws {
        let frame = SnapshotFixtures.slideWithTopFilmstrip()
        let crop = try XCTUnwrap(SnapshotFrameFilter.contentCrop(frame.image, faces: frame.faces))
        XCTAssertTrue(frame.slide.contains(crop), "crop \(crop) leaves slide \(frame.slide)")
        let covered = crop.intersection(frame.slide)
        XCTAssertGreaterThanOrEqual(covered.width * covered.height, 0.9 * frame.slide.width * frame.slide.height)
    }

    func testSingleLargeFaceSpeakerViewHasNoCrop() {
        let speaker = SnapshotFixtures.webcam(faceSide: 420)  // 420² of 1280x720 ≈ 19% of the frame
        XCTAssertNil(SnapshotFrameFilter.contentCrop(speaker.image, faces: speaker.faces))
    }

    func testSingleSmallFaceOnWebcamBackgroundHasNoCrop() {
        // 120² ≈ 1.6% of the frame; its tile leaves a 460 px wide strip (36%) beside it.
        let webcam = SnapshotFixtures.webcam(faceSide: 120)
        XCTAssertNil(SnapshotFrameFilter.contentCrop(webcam.image, faces: webcam.faces))
    }

    func testFaceLeavingUnderAQuarterOfTheFrameHasNoCrop() {
        // A mid-size face on a text slide: its 900 px tile leaves 190 px strips (15%) each side.
        let image = SnapshotFixtures.slide(roadmap)
        let face = CGRect(x: 490, y: 210, width: 300, height: 300)
        XCTAssertNil(SnapshotFrameFilter.contentCrop(image, faces: [face]))
    }

    func testSlideWithoutFacesKeepsWholeFrame() throws {
        let image = SnapshotFixtures.slide(roadmap)
        let crop = try XCTUnwrap(SnapshotFrameFilter.contentCrop(image, faces: []))
        XCTAssertEqual(crop, CGRect(x: 0, y: 0, width: image.width, height: image.height))
    }

    // MARK: - decide

    private func features(_ hash: UInt64, _ text: String = "") -> FrameFeatures {
        FrameFeatures(hash: hash, text: text, crop: CGRect(x: 0, y: 0, width: 100, height: 100))
    }

    func testFirstSampleWaitsForStable() {
        XCTAssertEqual(SnapshotFrameFilter.decide(current: features(1), previousSample: nil, lastKept: nil), .waitForStable)
    }

    func testChangingFrameWaitsForStable() {
        XCTAssertEqual(SnapshotFrameFilter.decide(current: features(0), previousSample: features(0xFF), lastKept: nil),
                       .waitForStable)
    }

    func testFirstStableFrameKeepsNew() {
        XCTAssertEqual(SnapshotFrameFilter.decide(current: features(0), previousSample: features(0), lastKept: nil), .keepNew)
    }

    func testUnchangedSlideIsSkipped() {
        let slide = features(0xF0F0, "Roadmap Q1")
        XCTAssertEqual(SnapshotFrameFilter.decide(current: slide, previousSample: slide, lastKept: slide), .skip)
    }

    func testBuildReplacesLast() {
        let current = features(0xF, "Roadmap Q1 Q2")  // 4 bits from the kept slide: a build looks alike
        XCTAssertEqual(SnapshotFrameFilter.decide(current: current, previousSample: current,
                                                  lastKept: features(0, "Roadmap Q1")), .replaceLast)
    }

    func testNewSlideKeepsNew() {
        let current = features(0xFFFF, "Hiring plan")
        XCTAssertEqual(SnapshotFrameFilter.decide(current: current, previousSample: current,
                                                  lastKept: features(0, "Roadmap Q1")), .keepNew)
    }

    func testSameWordsIsNotABuild() {
        let current = features(0xFFFF, "roadmap, Q1!")
        XCTAssertEqual(SnapshotFrameFilter.decide(current: current, previousSample: current,
                                                  lastKept: features(0, "Roadmap Q1")), .keepNew)
    }

    func testBuildBeatsSameSlideWhenHashesMatch() {
        let current = features(0, "Roadmap Q1 Q2")
        XCTAssertEqual(SnapshotFrameFilter.decide(current: current, previousSample: current,
                                                  lastKept: features(0, "Roadmap Q1")), .replaceLast)
    }

    func testTextDecidesStabilityWhenBothHaveEnoughWords() {
        let current = features(0, "Hiring plan for next quarter")
        XCTAssertEqual(SnapshotFrameFilter.decide(current: current, previousSample: features(0, "Roadmap Q1 Q2 Q3"),
                                                  lastKept: nil), .waitForStable)
    }

    // MARK: - decide on rendered frames

    /// Features as the snapshot service computes them: dHash plus Vision OCR of the whole frame.
    private func rendered(_ image: CGImage) -> FrameFeatures {
        FrameFeatures(hash: SnapshotFrameFilter.dHash(image), text: TextRecognizer.recognize(image) ?? "",
                      crop: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    }

    /// Decision for `next` sampled twice in a row after `kept` was saved.
    private func decideRecaptured(_ next: CGImage, after kept: CGImage) -> SnapshotDecision {
        let current = rendered(next)
        return SnapshotFrameFilter.decide(current: current, previousSample: rendered(next), lastKept: rendered(kept))
    }

    func testTwoDifferentTemplateTextSlidesAreBothKept() {
        let hiring = SnapshotFixtures.slide(["Hiring plan", "Two engineers", "One designer", "Support lead"])
        XCTAssertEqual(decideRecaptured(hiring, after: SnapshotFixtures.slide(roadmap)), .keepNew)
    }

    func testBuildStepReplacesLastWithRealFrames() {
        XCTAssertEqual(decideRecaptured(SnapshotFixtures.slideBuild(roadmap, shown: 3),
                                        after: SnapshotFixtures.slideBuild(roadmap, shown: 2)), .replaceLast)
    }

    func testTitleSlideFollowedByContentSlideWithSameTitleKeepsNew() {
        let content = SnapshotFixtures.slide(["Q3 Review", "Revenue up 12%", "Churn down to 3%", "Two launches shipped",
                                              "Hiring on plan", "Next: Q4 goals"])
        XCTAssertEqual(decideRecaptured(content, after: SnapshotFixtures.slide(["Q3 Review"])), .keepNew)
    }

    func testSameSlideRecapturedIsSkippedWithRealFrames() {
        XCTAssertEqual(decideRecaptured(SnapshotFixtures.slide(roadmap), after: SnapshotFixtures.slide(roadmap)), .skip)
    }

    func testLowTextDifferentDiagramsAreBothKept() {
        XCTAssertEqual(decideRecaptured(SnapshotFixtures.barChart(), after: SnapshotFixtures.flowchart()), .keepNew)
    }

    // MARK: - dHash

    func testIdenticalHashesDistanceZero() {
        let a = SnapshotFrameFilter.dHash(SnapshotFixtures.slide(roadmap))
        let b = SnapshotFrameFilter.dHash(SnapshotFixtures.slide(roadmap))
        XCTAssertEqual(SnapshotFrameFilter.hamming(a, b), 0)
    }

    func testHammingCountsDifferingBits() {
        XCTAssertEqual(SnapshotFrameFilter.hamming(0b1011, 0b0110), 3)
    }
}
