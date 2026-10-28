import XCTest
@testable import Whimbrel

final class ZoomJoinURLTests: XCTestCase {
    func test_cases() {
        let cases: [(link: String, expected: String?)] = [
            ("https://us02web.zoom.us/j/81234567890?pwd=AbC.1",
             "zoommtg://zoom.us/join?action=join&confno=81234567890&pwd=AbC.1"),
            ("https://zoom.us/j/81234567890",
             "zoommtg://zoom.us/join?action=join&confno=81234567890"),
            ("https://zoom.us/my/someone", nil),
            ("https://teams.microsoft.com/l/meetup-join/x", nil),
            ("https://evilzoom.us/j/1", nil),
        ]
        for (link, expected) in cases {
            let url = URL(string: link).flatMap { ZoomJoinURL.make(from: $0) }
            XCTAssertEqual(url?.absoluteString, expected, "link: \(link)")
        }
    }
}
