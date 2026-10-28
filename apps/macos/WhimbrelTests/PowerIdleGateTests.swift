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
}
