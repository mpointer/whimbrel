import XCTest
@testable import Whimbrel

final class ParticipantCountEstimatorTests: XCTestCase {
    func testReturnsNilForEmptyObservations() {
        XCTAssertNil(ParticipantCountEstimator.bestEstimate(from: []))
    }

    func testReturnsTheSingleValueWhenOnlyOneObservationExists() {
        XCTAssertEqual(ParticipantCountEstimator.bestEstimate(from: [4]), 4)
    }

    func testReturnsTheMostFrequentlyObservedValue() {
        // 5 appears 3 times, 4 appears once — 5 is the clear mode.
        XCTAssertEqual(ParticipantCountEstimator.bestEstimate(from: [4, 5, 5, 5]), 5)
    }

    func testBreaksATieByPreferringTheMostRecentlyObservedValue() {
        // 3 and 5 each appear twice; 5 was observed last, so it wins the tie.
        XCTAssertEqual(ParticipantCountEstimator.bestEstimate(from: [3, 5, 3, 5]), 5)
    }

    func testBreaksATieTheOtherWayWhenTheMoreRecentObservationIsTheOtherValue() {
        // Same two values, same counts, but 3 was observed last this time.
        XCTAssertEqual(ParticipantCountEstimator.bestEstimate(from: [5, 3, 5, 3]), 3)
    }
}
