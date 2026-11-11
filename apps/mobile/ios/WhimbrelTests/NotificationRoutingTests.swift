import XCTest
@testable import Whimbrel

final class NotificationRoutingTests: XCTestCase {
    func testRouteRoundTripsThroughUserInfo() {
        for route in [ReminderRoute.today, .topics, .topic(key: "acme")] {
            XCTAssertEqual(ReminderRoute(userInfo: route.userInfo), route)
        }
        XCTAssertEqual(ReminderRoute.topic(key: "acme").userInfo["key"] as? String, "acme")
    }

    func testMalformedUserInfoGivesNoRoute() {
        XCTAssertNil(ReminderRoute(userInfo: [:]))
        XCTAssertNil(ReminderRoute(userInfo: ["route": "nowhere"]))
        XCTAssertNil(ReminderRoute(userInfo: ["route": "topic"]), "a topic needs its key")
        XCTAssertNil(ReminderRoute(userInfo: ["route": "topic", "key": ""]))
        XCTAssertNil(ReminderRoute(userInfo: ["route": 3]))
    }

    func testRootDestinationPerRoute() {
        XCTAssertEqual(RootView.destination(for: .today).tab, 0)
        XCTAssertEqual(RootView.destination(for: .topics).tab, 4)
        XCTAssertNil(RootView.destination(for: .topics).topicKey)
        XCTAssertEqual(RootView.destination(for: .topic(key: "acme")).tab, 4)
        XCTAssertEqual(RootView.destination(for: .topic(key: "acme")).topicKey, "acme")
    }
}
