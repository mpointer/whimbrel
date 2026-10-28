import XCTest
@testable import Whimbrel

final class ConversationSidebarSortTests: XCTestCase {
    func testStarredSortsBeforeRecencyRegardlessOfAge() {
        let old = CoreMemoryStore.ConversationSummary(id: 1, title: "old", starred: false, updatedAtUnixMs: 1_000)
        let newer = CoreMemoryStore.ConversationSummary(id: 2, title: "newer", starred: false, updatedAtUnixMs: 2_000)
        let starredButOldest = CoreMemoryStore.ConversationSummary(id: 3, title: "starred", starred: true, updatedAtUnixMs: 500)

        let sorted = sortedForSidebar([old, newer, starredButOldest])
        XCTAssertEqual(sorted.map(\.id), [3, 2, 1])
    }
}
