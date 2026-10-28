import XCTest
@testable import Whimbrel

final class ThemeBundleTests: XCTestCase {
    func testTheAppBundleShipsTheFourThemesAndTheyLoad() throws {
        let directory = try XCTUnwrap(ThemeStore.bundledThemesDirectory, "themes/ must be in the app bundle (project.yml)")
        let themes = ThemeLoader.loadDirectory(directory)
        XCTAssertEqual(Set(themes.map(\.id)), ["default", "slate", "sepia", "high-contrast"])
    }
}
