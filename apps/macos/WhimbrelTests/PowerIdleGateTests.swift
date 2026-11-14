import XCTest
@testable import Whimbrel

final class PowerIdleGateTests: XCTestCase {
    func test_eligible_when_on_ac_power_and_idle_past_threshold() {
        XCTAssertTrue(PowerIdleGate.isEligible(isOnACPower: true, idleSeconds: 60, thresholdSeconds: 60))
        XCTAssertTrue(PowerIdleGate.isEligible(isOnACPower: true, idleSeconds: 120, thresholdSeconds: 60))
    }

    func test_ineligible_when_on_battery_even_if_idle() {
        XCTAssertFalse(PowerIdleGate.isEligible(isOnACPower: false, idleSeconds: 3600, thresholdSeconds: 60))
    }

    func test_ineligible_when_on_ac_but_not_idle_long_enough() {
        XCTAssertFalse(PowerIdleGate.isEligible(isOnACPower: true, idleSeconds: 5, thresholdSeconds: 60))
    }

    func test_boundary_exactly_at_threshold_is_eligible() {
        XCTAssertTrue(PowerIdleGate.isEligible(isOnACPower: true, idleSeconds: 60, thresholdSeconds: 60))
    }

    func test_recording_blocks_work_that_would_otherwise_run() {
        XCTAssertFalse(PowerIdleGate.isEligible(isOnACPower: true, idleSeconds: 3600, recordingActive: true))
        XCTAssertTrue(PowerIdleGate.isEligible(isOnACPower: true, idleSeconds: 3600, recordingActive: false))
    }

    func test_meeting_prep_is_exempt_from_recording_power_and_idle() {
        XCTAssertTrue(PowerIdleGate.isEligible(isOnACPower: false, idleSeconds: 0, recordingActive: true, exempt: true))
    }

    func test_a_meeting_is_recording_from_start_claim_until_the_id_clears() {
        XCTAssertFalse(PowerIdleGate.isRecording(activeMeetingId: nil, isStartingMeeting: false))
        XCTAssertTrue(PowerIdleGate.isRecording(activeMeetingId: nil, isStartingMeeting: true))
        XCTAssertTrue(PowerIdleGate.isRecording(activeMeetingId: 7, isStartingMeeting: false))
        XCTAssertTrue(PowerIdleGate.isRecording(activeMeetingId: 7, isStartingMeeting: true))
    }

    func test_the_lease_is_renewed_well_inside_core_expiry() {
        XCTAssertLessThan(PowerIdleGate.recordingRenewSeconds * 3, 10 * 60)
    }
}
