import XCTest
@testable import Whimbrel

@MainActor
final class ProfileSuggestionsModelTests: XCTestCase {
    private func profile() -> ProfileSettings {
        let suite = "profile-suggest-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return ProfileSettings(defaults: d)
    }

    private func suggestion(_ id: Int64, _ text: String, meeting: Int64? = nil) -> CoreProfileSuggestion {
        CoreProfileSuggestion(id: id, text: text, sourceMeetingId: meeting)
    }

    private func model(_ profile: ProfileSettings, pending: [CoreProfileSuggestion], log: Log) -> ProfileSuggestionsModel {
        let rows = Rows(pending)
        return ProfileSuggestionsModel(
            profile: profile,
            list: { log.listedOnMain.append(Thread.isMainThread); return rows.value },
            dismiss: { id in log.dismissed.append(id); rows.value.removeAll { $0.id == id }; return true },
            delete: { id in log.deleted.append(id); rows.value.removeAll { $0.id == id }; return true }
        )
    }

    final class Rows: @unchecked Sendable { var value: [CoreProfileSuggestion]; init(_ v: [CoreProfileSuggestion]) { value = v } }
    final class Log: @unchecked Sendable { var dismissed: [Int64] = []; var deleted: [Int64] = []; var listedOnMain: [Bool] = [] }

    /// Settings rebuilds its panes on a skin change, and the Profile pane reloads on appear. The
    /// store queue can be busy for minutes, so the read must never wait on the main thread.
    func testReloadReadsTheStoreOffTheMainThread() async {
        let log = Log()
        let m = model(profile(), pending: [suggestion(1, "a")], log: log)
        await m.reload()
        XCTAssertEqual(log.listedOnMain, [false])
        XCTAssertEqual(m.pending.map(\.id), [1])
    }

    func testReloadShowsThePendingSuggestions() async {
        let m = model(profile(), pending: [suggestion(1, "a"), suggestion(2, "b")], log: Log())
        await m.reload()
        XCTAssertEqual(m.pending.map(\.id), [1, 2])
    }

    func testAddTurnsASuggestionIntoAFactWithItsSourceAndRemovesTheRow() async {
        let p = profile()
        let log = Log()
        let m = model(p, pending: [suggestion(1, "Falcon is the Q4 launch", meeting: 7)], log: log)
        await m.reload()
        let added = await m.add(m.pending[0])
        XCTAssertTrue(added)
        XCTAssertEqual(p.facts.map(\.text), ["Falcon is the Q4 launch"])
        XCTAssertEqual(p.facts.first?.sourceMeetingId, 7)
        XCTAssertEqual(log.deleted, [1])
        XCTAssertTrue(m.pending.isEmpty)
    }

    func testAddingAnAlreadyKnownFactStillClearsTheSuggestionWithoutDuplicating() async {
        let p = profile()
        XCTAssertTrue(p.addFact("Falcon is the Q4 launch", sourceMeetingId: nil))
        let log = Log()
        let m = model(p, pending: [suggestion(1, "falcon is the q4 launch.")], log: log)
        await m.reload()
        let added = await m.add(m.pending[0])
        XCTAssertFalse(added)
        XCTAssertEqual(p.facts.count, 1)
        XCTAssertEqual(log.deleted, [1], "a suggestion that cannot be added is cleared, not left pending forever")
    }

    func testDismissRecordsItAndRemovesTheRowWithoutTouchingTheProfile() async {
        let p = profile()
        let log = Log()
        let m = model(p, pending: [suggestion(1, "a")], log: log)
        await m.reload()
        await m.dismiss(m.pending[0])
        XCTAssertEqual(log.dismissed, [1])
        XCTAssertTrue(m.pending.isEmpty)
        XCTAssertTrue(p.facts.isEmpty)
    }

    actor Gate {
        private var waiters: [CheckedContinuation<Void, Never>] = []
        private var open = false
        func wait() async { if !open { await withCheckedContinuation { waiters.append($0) } } }
        func release() { open = true; waiters.forEach { $0.resume() }; waiters = [] }
    }

    /// The store queue can be busy for minutes; the row must go when the button is pressed.
    func testDismissAndAddHideTheRowBeforeTheStoreAnswers() async {
        let gate = Gate()
        let rows = Rows([suggestion(1, "a"), suggestion(2, "b")])
        let m = ProfileSuggestionsModel(
            profile: profile(),
            list: { rows.value },
            dismiss: { id in await gate.wait(); rows.value.removeAll { $0.id == id }; return true },
            delete: { id in await gate.wait(); rows.value.removeAll { $0.id == id }; return true }
        )
        await m.reload()
        let first = m.pending[0], second = m.pending[1]
        let dismissing = Task { await m.dismiss(first) }
        let adding = Task { await m.add(second) }
        for _ in 0..<5 { await Task.yield() }
        XCTAssertTrue(m.pending.isEmpty, "rows are hidden while the store is still busy")
        await gate.release()
        await dismissing.value
        _ = await adding.value
        XCTAssertTrue(m.pending.isEmpty)
    }
}
