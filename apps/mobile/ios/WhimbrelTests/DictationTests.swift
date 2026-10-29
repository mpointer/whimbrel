import Speech
import XCTest
@testable import Whimbrel

final class DictationTests: XCTestCase {
    func test_denied_speech_is_denied() {
        XCTAssertEqual(Dictation.state(speech: .denied, mic: true, onDevice: true), .denied)
    }

    func test_denied_mic_is_denied() {
        XCTAssertEqual(Dictation.state(speech: .authorized, mic: false, onDevice: true), .denied)
    }

    func test_no_on_device_is_unavailable() {
        XCTAssertEqual(Dictation.state(speech: .authorized, mic: true, onDevice: false), .unavailable)
    }

    func test_authorized_is_idle() {
        XCTAssertEqual(Dictation.state(speech: .authorized, mic: true, onDevice: true), .idle)
    }

    func test_merge_keeps_typed_text_and_final_words() {
        XCTAssertEqual(Dictation.merge(base: "Who", transcript: "attended?"), "Who attended?")
        XCTAssertEqual(Dictation.merge(base: "", transcript: "hello"), "hello")
    }

    /// A permission closure that waits until the test lets it finish.
    @MainActor private func gated() -> (Dictation, () -> Void, () -> Int) {
        var release: CheckedContinuation<Void, Never>?
        var begins = 0
        let d = Dictation(permissions: { await withCheckedContinuation { release = $0 }; return (.authorized, true) },
                          onDevice: { true }, begin: { _ in begins += 1 })
        return (d, { release?.resume(); release = nil }, { begins })
    }

    @MainActor func test_start_while_starting_is_ignored() async {
        let (d, release, begins) = gated()
        d.start(); d.start()
        XCTAssertEqual(d.state, .starting)
        await Task.yield(); release()
        for _ in 0..<5 { await Task.yield() }
        XCTAssertEqual(begins(), 1)
    }

    @MainActor func test_stop_during_permission_prompt_never_begins() async {
        let (d, release, begins) = gated()
        d.start()
        await Task.yield()
        d.stop()
        XCTAssertEqual(d.state, .idle)
        release()
        for _ in 0..<5 { await Task.yield() }
        XCTAssertEqual(begins(), 0)
        XCTAssertEqual(d.state, .idle)
    }
}
