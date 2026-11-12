import XCTest
@testable import Whimbrel

/// Fake provider. `forbidTextReads` makes any text method fail the test, for the
/// "excluded app is never read" guarantee.
// Test-only: mutated before `read` and only read back after it returns.
private final class FakeProvider: FocusedElementProvider, @unchecked Sendable {
    var trusted = true
    var selection: String?
    var window: (title: String, text: String, url: String?)?
    var ocr: String?
    var forbidTextReads = false
    var screenAllowed = true
    private(set) var sawMainThread = false
    private(set) var selectionCalls = 0
    private(set) var windowCalls = 0
    private(set) var ocrCalls = 0

    var accessibilityTrusted: Bool { trusted }
    var screenCaptureAllowed: Bool { screenAllowed }
    func frontmostApp() -> (bundleId: String, name: String, pid: pid_t)? { nil }

    func selectedText(pid: pid_t) -> String? {
        selectionCalls += 1
        if Thread.isMainThread { sawMainThread = true }
        if forbidTextReads { XCTFail("selectedText must not be called") }
        return selection
    }

    func windowText(pid: pid_t) -> (title: String, text: String, url: String?)? {
        windowCalls += 1
        if Thread.isMainThread { sawMainThread = true }
        if forbidTextReads { XCTFail("windowText must not be called") }
        return window
    }

    func ocrText(pid: pid_t) async -> String? {
        ocrCalls += 1
        if Thread.isMainThread { sawMainThread = true }
        if forbidTextReads { XCTFail("ocrText must not be called") }
        return ocr
    }
}

@MainActor
final class FocusedWindowReaderTests: XCTestCase {
    private let app = (bundleId: "com.example.Notes", name: "Notes", pid: pid_t(1234))

    private func reader(_ p: FakeProvider) -> FocusedWindowReader {
        FocusedWindowReader(provider: p)
    }

    func test_selection_is_preferred_over_window_text() async {
        let p = FakeProvider()
        p.selection = "the chosen sentence"
        p.window = (title: "Plan", text: "a long window of visible text here", url: nil)
        let result = await reader(p).read(frontmost: app)
        guard case .read(let ctx) = result else { return XCTFail("expected read, got \(result)") }
        XCTAssertEqual(ctx.text, "the chosen sentence")
        XCTAssertTrue(ctx.isSelection)
        XCTAssertEqual(ctx.source, .selection)
        XCTAssertEqual(ctx.appName, "Notes")
        XCTAssertEqual(ctx.title, "Plan")
    }

    func test_window_text_used_when_no_selection() async {
        let p = FakeProvider()
        p.window = (title: "Plan", text: "visible window text", url: nil)
        let result = await reader(p).read(frontmost: app)
        guard case .read(let ctx) = result else { return XCTFail("expected read, got \(result)") }
        XCTAssertEqual(ctx.text, "visible window text")
        XCTAssertFalse(ctx.isSelection)
        XCTAssertEqual(ctx.source, .accessibility)
        XCTAssertEqual(p.ocrCalls, 0)
    }

    func test_short_window_text_falls_back_to_ocr() async {
        let p = FakeProvider()
        p.window = (title: "Canvas", text: "  ab  ", url: nil)
        p.ocr = "text recognised from pixels"
        let result = await reader(p).read(frontmost: app)
        guard case .read(let ctx) = result else { return XCTFail("expected read, got \(result)") }
        XCTAssertEqual(ctx.text, "text recognised from pixels")
        XCTAssertEqual(ctx.source, .ocr)
        XCTAssertEqual(p.ocrCalls, 1)
    }

    func test_excluded_bundle_id_never_touches_provider_text() async {
        let p = FakeProvider()
        p.forbidTextReads = true
        p.selection = "secret"
        p.window = (title: "t", text: "secret window text", url: nil)
        let excluded = (bundleId: "com.apple.loginwindow", name: "loginwindow", pid: pid_t(1))
        let result = await reader(p).read(frontmost: excluded)
        XCTAssertEqual(result, .excluded(appName: "loginwindow"))
        XCTAssertEqual(p.selectionCalls + p.windowCalls + p.ocrCalls, 0)
    }

    func test_excluded_domain_returns_excluded_and_drops_text() async {
        let exclusions = CaptureExclusions.shared
        let had = exclusions.excludedDomains
        exclusions.addExcludedDomain("bank.example.com")
        defer { exclusions.setExcludedDomains(Array(had)) }
        let p = FakeProvider()
        p.selection = "account number"
        p.window = (title: "Bank", text: "balance details here", url: "https://www.bank.example.com/acct")
        let result = await reader(p).read(frontmost: app)
        XCTAssertEqual(result, .excluded(appName: "Notes"))
        XCTAssertEqual(p.selectionCalls, 0, "selection must not be consulted for an excluded domain")
        XCTAssertEqual(p.ocrCalls, 0)
    }

    func test_not_trusted_returns_not_permitted() async {
        let p = FakeProvider()
        p.trusted = false
        p.forbidTextReads = true
        let result = await reader(p).read(frontmost: app)
        XCTAssertEqual(result, .notPermitted)
    }

    func test_secrets_are_redacted() async {
        let p = FakeProvider()
        let key = "sk-ant-api03-" + String(repeating: "A1b2C3d4", count: 6)
        p.window = (title: "Notes", text: "my key is \(key) please keep it", url: nil)
        let result = await reader(p).read(frontmost: app)
        guard case .read(let ctx) = result else { return XCTFail("expected read, got \(result)") }
        XCTAssertFalse(ctx.text.contains(key))
        XCTAssertEqual(ctx.text, CoreMemoryStore.redactSecrets(ctx.text))
    }

    func test_nothing_anywhere_is_unreadable() async {
        let p = FakeProvider()
        p.window = (title: "Empty", text: "   ", url: nil)
        p.ocr = "  "
        let result = await reader(p).read(frontmost: app)
        XCTAssertEqual(result, .unreadable(appName: "Notes"))
    }

    func test_no_window_and_no_selection_is_unreadable() async {
        let result = await reader(FakeProvider()).read(frontmost: app)
        XCTAssertEqual(result, .unreadable(appName: "Notes"))
    }

    func test_provider_calls_run_off_the_main_thread() async {
        let p = FakeProvider()
        p.window = (title: "t", text: "ab", url: nil)
        p.ocr = "recognised text from pixels"
        _ = await reader(p).read(frontmost: app)
        XCTAssertGreaterThan(p.windowCalls, 0)
        XCTAssertGreaterThan(p.selectionCalls, 0)
        XCTAssertGreaterThan(p.ocrCalls, 0)
        XCTAssertFalse(p.sawMainThread)
    }

    func test_ocr_skipped_without_screen_recording_permission() async {
        let p = FakeProvider()
        p.screenAllowed = false
        p.window = (title: "t", text: "ab", url: nil)
        p.ocr = "recognised text from pixels"
        let result = await reader(p).read(frontmost: app)
        XCTAssertEqual(p.ocrCalls, 0)
        XCTAssertEqual(result, .read(WindowContext(appName: "Notes", title: "t", text: "ab", isSelection: false)))
    }
}
