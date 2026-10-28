import XCTest
@testable import Whimbrel

final class TodayComposerTests: XCTestCase {
    func testAskIsAvailableForALocalModelWithoutAKey() {
        XCTAssertTrue(TodayComposer.askAvailable(provider: .ollama, localOnly: false, hasKey: false))
        XCTAssertTrue(TodayComposer.askAvailable(provider: .ollama, localOnly: true, hasKey: false))
    }

    func testAHostedProviderNeedsItsKeyAndLocalOnlyOff() {
        for provider in [RoutingSettings.Provider.anthropic, .openai, .google] {
            XCTAssertTrue(TodayComposer.askAvailable(provider: provider, localOnly: false, hasKey: true))
            XCTAssertFalse(TodayComposer.askAvailable(provider: provider, localOnly: false, hasKey: false))
            XCTAssertFalse(TodayComposer.askAvailable(provider: provider, localOnly: true, hasKey: true))
        }
    }
}
