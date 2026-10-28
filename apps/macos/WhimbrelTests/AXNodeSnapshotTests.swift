#if DEBUG
import XCTest
@testable import Whimbrel

final class AXNodeSnapshotTests: XCTestCase {
    func test_round_trip() throws {
        let tree = AXNodeSnapshot(
            role: "AXWindow", subrole: "AXStandardWindow", title: "Zoom Meeting", value: nil, description: nil, identifier: nil,
            children: [
                AXNodeSnapshot(role: "AXButton", subrole: nil, title: "Mute", value: nil, description: "mute my microphone", identifier: "mute", children: []),
                AXNodeSnapshot(role: "AXGroup", subrole: nil, title: nil, value: nil, description: nil, identifier: nil, children: [
                    AXNodeSnapshot(role: "AXStaticText", subrole: nil, title: nil, value: "Person A", description: nil, identifier: nil, children: [])
                ])
            ]
        )
        let decoded = try JSONDecoder().decode(AXNodeSnapshot.self, from: JSONEncoder().encode(tree))
        XCTAssertEqual(decoded, tree)
    }
}
#endif
