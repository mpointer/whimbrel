import XCTest
@testable import Whimbrel

final class AskAvailabilityTests: XCTestCase {
    func testAskIsAvailableForALocalModelWithoutAKey() {
        XCTAssertTrue(AskAvailability.askAvailable(provider: .ollama, localOnly: false, hasKey: false))
        XCTAssertTrue(AskAvailability.askAvailable(provider: .ollama, localOnly: true, hasKey: false))
    }

    func testAHostedProviderNeedsItsKeyAndLocalOnlyOff() {
        for provider in [RoutingSettings.Provider.anthropic, .openai, .google] {
            XCTAssertTrue(AskAvailability.askAvailable(provider: provider, localOnly: false, hasKey: true))
            XCTAssertFalse(AskAvailability.askAvailable(provider: provider, localOnly: false, hasKey: false))
            XCTAssertFalse(AskAvailability.askAvailable(provider: provider, localOnly: true, hasKey: true))
        }
    }
}
