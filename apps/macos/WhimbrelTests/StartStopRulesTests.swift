import XCTest
@testable import Whimbrel

/// A Stop that arrives while a recording is starting (audio on, meeting row not yet written).
/// Pure decisions only: no audio, store, clock or locale is touched.
final class StartStopRulesTests: XCTestCase {
    private func action(hasRow: Bool = false, isStarting: Bool = false, ahead: Bool = false,
                        requested: Bool = false, live: Bool = false) -> StartStopRules.StopAction {
        StartStopRules.stopAction(hasRow: hasRow, isStarting: isStarting, audioStartedAhead: ahead, stopRequested: requested, audioLive: live)
    }

    func testAMeetingWithARowEndsAsUsual() {
        XCTAssertEqual(action(hasRow: true), .endMeeting)
        XCTAssertEqual(action(hasRow: true, live: true), .endMeeting)
        XCTAssertEqual(action(hasRow: true, isStarting: true, ahead: true), .endMeeting)
    }

    func testAStopDuringAStartWithAudioAheadOfItsRowStopsTheAudioAndWaitsForTheRow() {
        XCTAssertEqual(action(isStarting: true, ahead: true), .stopAudioAndEndWhenRowLands)
        XCTAssertEqual(action(isStarting: true, ahead: true, live: true), .stopAudioAndEndWhenRowLands)
    }

    func testASecondStopWhileOneIsWaitingDoesNothing() {
        XCTAssertEqual(action(isStarting: true, ahead: true, requested: true), .nothing)
        XCTAssertEqual(action(isStarting: true, ahead: true, requested: true, live: true), .nothing)
    }

    func testAStartWithNoAudioAheadOfItsRowHasNothingToStop() {
        // A manual start: its audio begins only after the row lands.
        XCTAssertEqual(action(isStarting: true, ahead: false), .nothing)
        XCTAssertEqual(action(isStarting: true, ahead: false, live: true), .nothing)
    }

    func testAnOrphanRecordingWithNoStartInFlightIsStopped() {
        XCTAssertEqual(action(live: true), .stopRowlessRecording)
        XCTAssertEqual(action(live: false), .nothing)
    }

    func testTheMenuOffersStopOnlyForALiveStartWithNoStopWaiting() {
        XCTAssertTrue(StartStopRules.canOfferStop(isStarting: true, audioStartedAhead: true, stopRequested: false))
        XCTAssertFalse(StartStopRules.canOfferStop(isStarting: true, audioStartedAhead: true, stopRequested: true))
        XCTAssertFalse(StartStopRules.canOfferStop(isStarting: true, audioStartedAhead: false, stopRequested: false))
        XCTAssertFalse(StartStopRules.canOfferStop(isStarting: false, audioStartedAhead: true, stopRequested: false))
        XCTAssertFalse(StartStopRules.canOfferStop(isStarting: false, audioStartedAhead: false, stopRequested: false))
    }

    func testARowThatLandsAfterAStopEndsThroughTheNormalEnd() {
        XCTAssertEqual(StartStopRules.finishAction(generationCurrent: true, stopRequested: true, insertLanded: true), .endThroughNormalEnd)
    }

    func testARowThatLandsWithNoStopGoesLiveAsBefore() {
        XCTAssertEqual(StartStopRules.finishAction(generationCurrent: true, stopRequested: false, insertLanded: true), .proceed)
    }

    func testAFailedInsertRefusesWhetherOrNotAStopWasWaiting() {
        XCTAssertEqual(StartStopRules.finishAction(generationCurrent: true, stopRequested: true, insertLanded: false), .refuse)
        XCTAssertEqual(StartStopRules.finishAction(generationCurrent: true, stopRequested: false, insertLanded: false), .refuse)
    }

    func testAReplacedStartIsAbandonedAsBefore() {
        for requested in [true, false] {
            for landed in [true, false] {
                XCTAssertEqual(StartStopRules.finishAction(generationCurrent: false, stopRequested: requested, insertLanded: landed), .abandon)
            }
        }
    }

    func testTheCompletionSettlesTheAudioStartFirstOnlyWhenAStopIsWaiting() {
        XCTAssertTrue(StartStopRules.mustSettleAudioFirst(stopRequested: true, alreadySettled: false))
        XCTAssertFalse(StartStopRules.mustSettleAudioFirst(stopRequested: true, alreadySettled: true))
        XCTAssertFalse(StartStopRules.mustSettleAudioFirst(stopRequested: false, alreadySettled: false))
        XCTAssertFalse(StartStopRules.mustSettleAudioFirst(stopRequested: false, alreadySettled: true))
    }

    // MARK: Which audio start owns the service

    func testAStartThatIsStoppedWhileOpeningIsStaleWhenItResumes() {
        var epochs = AudioStartEpochs()
        let start1 = epochs.begin()
        XCTAssertTrue(epochs.owns(start1), "awaiting, nothing has happened")
        epochs.stop()
        XCTAssertFalse(epochs.owns(start1), "stop, then start1 resumes: must publish nothing")
    }

    func testAStoppedStartIsStaleEvenAfterANewerStartBegins() {
        var epochs = AudioStartEpochs()
        let start1 = epochs.begin()
        epochs.stop()
        let start2 = epochs.begin()
        XCTAssertFalse(epochs.owns(start1), "start1 resumes first: stale, leaves start2's streams alone")
        XCTAssertTrue(epochs.owns(start2), "start2 resumes: still current, publishes")
    }

    func testASupersededStartIsStaleWithNoStopAtAll() {
        var epochs = AudioStartEpochs()
        let start1 = epochs.begin()
        let start2 = epochs.begin()
        XCTAssertFalse(epochs.owns(start1))
        XCTAssertTrue(epochs.owns(start2))
    }

    func testALoneStartThatNothingTouchesKeepsTheService() {
        var epochs = AudioStartEpochs()
        let start1 = epochs.begin()
        XCTAssertTrue(epochs.owns(start1))
        XCTAssertTrue(epochs.owns(start1), "asking does not move it")
    }

    func testAStopAfterTheStartPublishedMakesItStaleToo() {
        var epochs = AudioStartEpochs()
        let start1 = epochs.begin()
        epochs.stop()
        epochs.stop()
        XCTAssertFalse(epochs.owns(start1))
    }

    // MARK: What a stopped start does with its row

    func testAStoppedStartWithAudioEndsNormallyWhetherNewOrContinued() {
        XCTAssertEqual(StartStopRules.stoppedStartOutcome(continued: false, audioFileExists: true), .endNormally)
        XCTAssertEqual(StartStopRules.stoppedStartOutcome(continued: true, audioFileExists: true), .endNormally)
    }

    func testAStoppedNewMeetingWithNoAudioFileLeavesNoRow() {
        XCTAssertEqual(StartStopRules.stoppedStartOutcome(continued: false, audioFileExists: false), .deleteEmptyNewMeeting)
    }

    func testAStoppedContinuedMeetingWithNoNewAudioUndoesItsReopen() {
        XCTAssertEqual(StartStopRules.stoppedStartOutcome(continued: true, audioFileExists: false), .undoReopenOfContinued)
    }

    func testTheSettleWaitIsBoundedAndShort() {
        XCTAssertGreaterThan(StartStopRules.settleTimeoutNanoseconds, 0)
        XCTAssertLessThanOrEqual(StartStopRules.settleTimeoutNanoseconds, 10_000_000_000)
    }
}
