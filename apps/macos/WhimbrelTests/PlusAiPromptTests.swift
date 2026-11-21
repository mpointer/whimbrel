import XCTest
@testable import Whimbrel

@MainActor
final class PlusAiPromptTests: XCTestCase {
    private var suite: UserDefaults!
    private let suiteName = "PlusAiPromptTests.\(UUID().uuidString)"
    /// RoutingSettings writes these to the host's standard defaults; saved and restored.
    private static let touchedKeys = [
        "com.mpointer.kestrel.routingPreferredProvider",
        "com.mpointer.kestrel.syncTimestamp.routing.preferredProvider",
        "com.mpointer.kestrel.routingLocalOnlyMode",
        "com.mpointer.kestrel.syncTimestamp.routing.localOnlyMode",
    ]
    private var saved: [String: Any] = [:]

    override func setUp() {
        super.setUp()
        suite = UserDefaults(suiteName: suiteName)
        for key in Self.touchedKeys {
            if let v = UserDefaults.standard.object(forKey: key) { saved[key] = v }
            UserDefaults.standard.removeObject(forKey: key)
        }
    }

    override func tearDown() {
        suite.removePersistentDomain(forName: suiteName)
        for key in Self.touchedKeys {
            if let v = saved[key] { UserDefaults.standard.set(v, forKey: key) } else { UserDefaults.standard.removeObject(forKey: key) }
        }
        saved = [:]
        super.tearDown()
    }

    private func show(
        tier: String? = "plus", signedIn: Bool = true, credentials: Bool = true,
        provider: RoutingSettings.Provider = .ollama, localOnly: Bool = false,
        answered: Bool = false, recording: Bool = false, note: Bool = true
    ) -> Bool {
        PlusAiPrompt.shouldShow(
            planTier: tier, isSignedIn: signedIn, hasWhimbrelCredentials: credentials,
            preferredProvider: provider, localOnlyMode: localOnly, answered: answered, isRecording: recording, hasDataNote: note)
    }

    func testShowsInTheHappyCase() {
        XCTAssertTrue(show())
        XCTAssertTrue(show(provider: .anthropic), "an existing provider choice is still asked once")
    }

    func testEachBlockingConditionAloneHidesIt() {
        XCTAssertFalse(show(tier: "cloud"))
        XCTAssertFalse(show(tier: nil))
        XCTAssertFalse(show(signedIn: false))
        XCTAssertFalse(show(credentials: false))
        XCTAssertFalse(show(provider: .whimbrel))
        XCTAssertFalse(show(localOnly: true))
        XCTAssertFalse(show(answered: true))
        XCTAssertFalse(show(recording: true))
        XCTAssertFalse(show(note: false), "no disclosure, no sheet")
    }

    func testFlagKeyIsInTheWhimbrelNamespaceAndDefaultsOff() {
        XCTAssertEqual(PlusAiPrompt.Answered.key, "com.mpointer.whimbrel.plusAiPromptAnswered")
        XCTAssertFalse(PlusAiPrompt.Answered(defaults: suite).isSet)
    }

    func testAcceptSetsProviderAndFlag() {
        let routing = RoutingSettings()
        routing.preferredProvider = .ollama
        let answered = PlusAiPrompt.Answered(defaults: suite)
        PlusAiPrompt.accept(routing: routing, answered: answered)
        XCTAssertEqual(routing.preferredProvider, .whimbrel)
        XCTAssertTrue(answered.isSet)
    }

    func testDeclineSetsOnlyTheFlag() {
        let routing = RoutingSettings()
        routing.preferredProvider = .anthropic
        let answered = PlusAiPrompt.Answered(defaults: suite)
        PlusAiPrompt.decline(answered: answered)
        XCTAssertEqual(routing.preferredProvider, .anthropic)
        XCTAssertTrue(answered.isSet)
    }

    func testNeverAskedTwiceOnceAnswered() {
        let answered = PlusAiPrompt.Answered(defaults: suite)
        XCTAssertTrue(show(answered: answered.isSet))
        PlusAiPrompt.decline(answered: answered)
        XCTAssertFalse(show(answered: PlusAiPrompt.Answered(defaults: suite).isSet), "the answer persists")
        // Even if the user later switches away from Whimbrel AI.
        PlusAiPrompt.accept(routing: RoutingSettings(), answered: answered)
        XCTAssertFalse(show(provider: .ollama, answered: answered.isSet))
    }

    func testAlreadyOnWhimbrelIsMarkedAnsweredSilently() {
        let answered = PlusAiPrompt.Answered(defaults: suite)
        PlusAiPrompt.markAnsweredIfAlreadyOnWhimbrel(planTier: "plus", preferredProvider: .whimbrel, answered: answered)
        XCTAssertTrue(answered.isSet)
        XCTAssertFalse(show(provider: .ollama, answered: answered.isSet))
    }

    func testSilentMarkOnlyForPlusOnWhimbrel() {
        let answered = PlusAiPrompt.Answered(defaults: suite)
        PlusAiPrompt.markAnsweredIfAlreadyOnWhimbrel(planTier: "plus", preferredProvider: .ollama, answered: answered)
        PlusAiPrompt.markAnsweredIfAlreadyOnWhimbrel(planTier: "cloud", preferredProvider: .whimbrel, answered: answered)
        XCTAssertFalse(answered.isSet)
    }

    // MARK: Host mapping and button handling

    private func plan(_ plan: String, tier: String?) -> AccountPlan {
        AccountPlan(plan: plan, until: 1_790_000_000_000, interval: "month", planTier: tier, billing: true)
    }

    func testPlanTierUsesIsPlusIncludingTheCloudCheck() {
        XCTAssertEqual(PlusAiPrompt.planTier(of: plan("cloud", tier: "plus")), "plus")
        XCTAssertNil(PlusAiPrompt.planTier(of: plan("free", tier: "plus")), "tier plus without Cloud is not Plus")
        XCTAssertNil(PlusAiPrompt.planTier(of: plan("cloud", tier: "cloud")))
        XCTAssertNil(PlusAiPrompt.planTier(of: nil))
    }

    func testWantedReadsPlanProviderAndLocalOnlyFromTheRealSettings() {
        let routing = RoutingSettings()
        routing.preferredProvider = .ollama
        routing.localOnlyMode = false
        let answered = PlusAiPrompt.Answered(defaults: suite)
        let plus = plan("cloud", tier: "plus")
        // Whimbrel credentials are nil in a test, so Whimbrel AI is not usable: never wanted.
        XCTAssertFalse(PlusAiPrompt.wanted(plan: plus, isSignedIn: true, routing: routing, answered: answered, isRecording: false, dataNote: "note"))
        routing.whimbrelCredentialsProvider = { ("id", "key") }
        XCTAssertTrue(PlusAiPrompt.wanted(plan: plus, isSignedIn: true, routing: routing, answered: answered, isRecording: false, dataNote: "note"))
        XCTAssertFalse(PlusAiPrompt.wanted(plan: plan("free", tier: "plus"), isSignedIn: true, routing: routing, answered: answered, isRecording: false, dataNote: "note"))
        XCTAssertFalse(PlusAiPrompt.wanted(plan: plus, isSignedIn: true, routing: routing, answered: answered, isRecording: false, dataNote: nil))
        routing.localOnlyMode = true
        XCTAssertFalse(PlusAiPrompt.wanted(plan: plus, isSignedIn: true, routing: routing, answered: answered, isRecording: false, dataNote: "note"), "Local-only mode is passed through")
    }

    func testKeepAndNotNowNeverAccept() {
        for choice in [PlusAiPrompt.Choice.keep, .notNow] {
            let routing = RoutingSettings()
            routing.preferredProvider = .anthropic
            let answered = PlusAiPrompt.Answered(defaults: suite)
            answered.isSet = false
            PlusAiPrompt.handle(choice, routing: routing, answered: answered)
            XCTAssertEqual(routing.preferredProvider, .anthropic)
            XCTAssertEqual(answered.isSet, choice == .keep, "Keep answers, Not now does not")
        }
    }

    func testUseAccepts() {
        let routing = RoutingSettings()
        routing.preferredProvider = .ollama
        let answered = PlusAiPrompt.Answered(defaults: suite)
        PlusAiPrompt.handle(.use, routing: routing, answered: answered)
        XCTAssertEqual(routing.preferredProvider, .whimbrel)
        XCTAssertTrue(answered.isSet)
    }

    func testNotNowLeavesItToBeAskedAgain() {
        let answered = PlusAiPrompt.Answered(defaults: suite)
        PlusAiPrompt.handle(.notNow, routing: RoutingSettings(), answered: answered)
        XCTAssertTrue(show(answered: PlusAiPrompt.Answered(defaults: suite).isSet))
    }
}
