import XCTest
@testable import Whimbrel

final class ActiveMeetingViewTests: XCTestCase {

    @MainActor
    func testFlushSavesPendingTextImmediately() {
        var saved: [String] = []
        let controller = ActiveNoteAutosaveController(debounceSeconds: 60) { text, _ in saved.append(text); return true }
        controller.noteText = "a"
        controller.noteText = "ab"
        controller.flush()
        XCTAssertEqual(saved, ["ab"])
        controller.flush()
        XCTAssertEqual(saved, ["ab"], "nothing pending, nothing saved")
    }

    @MainActor
    func testSeedingDoesNotSave() {
        var saved: [String] = []
        let controller = ActiveNoteAutosaveController(debounceSeconds: 0) { text, _ in saved.append(text); return true }
        controller.seed("draft")
        controller.flush()
        XCTAssertEqual(controller.noteText, "draft")
        XCTAssertEqual(controller.saveState, .idle)
        XCTAssertTrue(saved.isEmpty)
    }
    @MainActor
    func testAutosaveControllerReportsSavingThenSaved() {
        let expectation = expectation(description: "save called")
        var savedText: String?
        let controller = ActiveNoteAutosaveController(debounceSeconds: 0) { text, _ in
            savedText = text
            expectation.fulfill()
            return true
        }

        controller.noteText = "Discussed roadmap"

        wait(for: [expectation], timeout: 2)
        XCTAssertEqual(savedText, "Discussed roadmap")
        XCTAssertEqual(controller.saveState, .saved)
    }

    @MainActor
    func testAutosaveControllerReportsErrorOnFailedWrite() {
        let expectation = expectation(description: "save called")
        let controller = ActiveNoteAutosaveController(debounceSeconds: 0) { _, _ in
            expectation.fulfill()
            return false
        }

        controller.noteText = "Some note"

        wait(for: [expectation], timeout: 2)
        XCTAssertEqual(controller.saveState, .error)
    }

    @MainActor
    func testActiveMeetingViewInstantiates() {
        let meeting = CoreMeeting(
            id: 1, deviceId: "d1", startUnixMs: 1_700_000_000_000, endUnixMs: nil, title: "Standup",
            attendees: [], trigger: .manual, audioPath: nil, status: .recording, transcriptPath: nil,
            notes: nil, summaryTldr: nil, summaryDecisions: [], summaryActionItems: [],
            summaryOpenQuestions: [], summaryProvider: nil, summaryModel: nil, estimatedSpeakerCount: nil
        )
        let model = ActiveMeetingModel(meeting: meeting, elapsedSeconds: 30, isProcessing: false, processingStage: nil)
        let view = ActiveMeetingView(model: model, initialNotes: "", onSaveNotes: { _, _ in true }, onStop: {})
        XCTAssertNotNil(view.body)
    }
}
