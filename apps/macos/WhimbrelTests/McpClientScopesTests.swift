import XCTest
@testable import Whimbrel

final class McpClientScopesTests: XCTestCase {
    func testOffersTheFourScopesInFormOrderWithGoogleDataLast() {
        XCTAssertEqual(McpClientScopes.all, ["memory", "meetings", "transcripts", "google_data"])
    }

    func testGoogleDataAloneOpensNothing() {
        XCTAssertFalse(McpClientScopes.canCreate(name: "Claude", scopes: ["google_data"]))
        XCTAssertFalse(McpClientScopes.canCreate(name: "Claude", scopes: []))
        XCTAssertFalse(McpClientScopes.canCreate(name: "   ", scopes: ["memory"]))
        XCTAssertTrue(McpClientScopes.canCreate(name: "Claude", scopes: ["memory"]))
        XCTAssertTrue(McpClientScopes.canCreate(name: "Claude", scopes: ["google_data", "meetings"]))
    }

    func testOrderedKeepsFormOrderAndDropsUnknownIds() {
        XCTAssertEqual(McpClientScopes.ordered(["google_data", "transcripts", "admin", "memory"]),
                       ["memory", "transcripts", "google_data"])
        XCTAssertEqual(McpClientScopes.ordered([]), [])
    }

    func testAClientWithoutGoogleDataStaysOff() {
        // Tokens made before this switch existed carry no google_data id, so they read as off.
        let existing = ["memory", "meetings"]
        XCTAssertFalse(existing.contains(McpClientScopes.googleData))
        XCTAssertFalse(McpClientScopes.line(existing).contains("Gmail"))
    }

    func testLineShowsNamesNotIds() {
        XCTAssertEqual(McpClientScopes.line(["meetings", "google_data"]), "Meetings, Gmail and Google Calendar")
        XCTAssertEqual(McpClientScopes.line(["future_scope"]), "future_scope")
    }
}
