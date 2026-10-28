import Foundation

/// Proves the Swift -> Rust FFI boundary works end to end: opens a core-backed store,
/// ingests a small fixture (so this is meaningful even before real capture data exists),
/// also ingests any new lines appended to screen-text.jsonl since the last launch, then
/// runs both search paths. Writes a plain-text report to disk rather than relying on
/// console output, which isn't reliably visible in every environment this has been
/// tested from.
///
/// `ingestNewScreenTextLines` is the only logic that gets captured screen text into the
/// searchable core store — `ScreenTextCaptureService` only appends to the JSONL log, it
/// doesn't ingest. It's driven from three places: this manual self-test, the Settings
/// "Run Core Self-Test" button, and (the real production path) `AppState.
/// scheduleScreenTextIngestStep`'s periodic background loop -- added after three weeks
/// of a real user's capture data went completely unindexed because neither of the first
/// two manual/incidental triggers ran during normal daily use. Re-reading the whole file
/// on every launch was also a real bug once real usage data accumulated: at 87MB/347MB
/// (core-memory.sqlite) it took minutes and blocked the main thread long enough that the
/// window never got a chance to draw (macOS showed the Dock icon with no visible window).
/// Fixed by tracking how much of the file was already ingested (byte offset, `Self.
/// ingestedOffsetKey`) and only re-reading the tail each pass — first run after this fix
/// skips the existing backlog rather than doing one more multi-minute catch-up (that data
/// is already mostly in the store from ingestion during earlier sessions, before this got
/// slow).
enum CoreSelfTest {
    private static let ingestedOffsetKey = "com.mpointer.kestrel.screenTextIngestedOffset"

    static func run(store: CoreMemoryStore?, deviceId: String) {
        var lines: [String] = ["Core version: \(CoreMemoryStore.coreVersion)"]

        guard let store else {
            lines.append("FAIL: shared store failed to open (see AppState.coreStore)")
            writeReport(lines)
            return
        }
        lines.append("Using shared store")
        lines.append("Embedding model loaded: \(store.embeddingModelLoaded())")

        let fixtureURL = SpikePaths.root.appendingPathComponent("core-selftest-fixture.jsonl")
        let fixture = """
        {"timestamp":"2026-07-23T09:00:00Z","bundle_id":"com.apple.Safari","window_title":"Quarterly Report","url":"https://example.com","text":"quarterly revenue finance budget forecast","source":"ax"}
        {"timestamp":"2026-07-23T09:05:00Z","bundle_id":"com.apple.Terminal","window_title":"Terminal","url":null,"text":"cargo build succeeded with zero errors","source":"ax"}

        """
        try? fixture.write(to: fixtureURL, atomically: true, encoding: .utf8)

        if let stats = store.ingestJSONL(path: fixtureURL, deviceId: "self-test-device") {
            lines.append("Ingest fixture: read=\(stats.recordsRead) inserted=\(stats.itemsInserted) deduped=\(stats.itemsDeduped) errors=\(stats.parseErrors)")
        } else {
            lines.append("FAIL: fixture ingest returned nil")
        }

        lines.append(ingestNewScreenTextLines(into: store, deviceId: deviceId).message)

        let keywordResults = store.searchKeyword("cargo build")
        lines.append("Keyword search 'cargo build': \(keywordResults.count) result(s)")
        for item in keywordResults.prefix(3) {
            lines.append("  - [\(item.appBundleId)] \(item.text)")
        }

        let semanticResults = store.searchSemantic("financial numbers forecast")
        lines.append("Semantic search 'financial numbers forecast': \(semanticResults.count) result(s)")
        for item in semanticResults.prefix(3) {
            lines.append("  - [\(item.appBundleId)] \(item.text)")
        }

        let keywordPass = keywordResults.contains { $0.text.contains("cargo build") }
        let semanticPass = semanticResults.contains { $0.appBundleId == "com.apple.Safari" }
        lines.append(keywordPass && semanticPass ? "RESULT: PASS" : "RESULT: FAIL")

        writeReport(lines)
    }

    /// Bounded per pass (not "read to end of file") so a large backlog — or an offset
    /// reset from a file rotation — can never block the shared `CoreMemoryStore.
    /// accessQueue` on a many-hundred-MB read in one shot. Any remainder is picked up
    /// incrementally on the next launch/Timeline-window-open.
    ///
    /// Lowered from 5,000,000 on 2026-08-22: the byte cap alone wasn't enough. Each line
    /// in this pass gets a real embedding computed (`EmbeddingModel::embed`, a genuine
    /// BERT forward pass, not a cheap hash) before its `insert_capture_item` write can
    /// complete — cost scales with LINE COUNT, not raw bytes, and 5MB of short lines is
    /// thousands of embeddings computed serially inside one `CoreMemoryStore.accessQueue`
    /// closure (now called at most once per launch, not once per Library-tab visit — see
    /// `TimelineView.hasAutoRunSelfTestThisLaunch` — but that only bounds *frequency*, not
    /// how long any single guarded run can still block everything else on that queue). At
    /// 500,000 a single pass finishes quickly regardless of line density; a large backlog
    /// now catches up over several launches instead of stalling the app on one of them.
    private static let maxBytesPerIngestPass = 500_000

    /// Hard ceiling for growing one pass's read past `maxBytesPerIngestPass` when the
    /// first read finds no newline at all. Bug found live 2026-09-10: a single captured
    /// record serialized to a ~526KB JSONL line, just over the 500KB cap. The old code
    /// treated "no newline in this read" as always meaning "genuinely at the end of the
    /// file with a partial trailing line, wait for more" — but that early return never
    /// advanced `ingestedOffsetKey`, so once a real line exceeded the cap, every later
    /// pass re-read the exact same bytes, found no newline again, and never made
    /// progress again. Ingestion was silently stalled for about two weeks before this
    /// was caught. `scanForCompleteLine` now tells these two cases apart.
    private static let maxLineSearchBytes = 20_000_000

    /// Ingests only the bytes appended to `screen-text.jsonl` since the last launch
    /// (tracked via `ingestedOffsetKey`), instead of the whole file every time. Writes
    /// the tail to a small scratch file since `ingestJSONL` takes a path, not raw bytes.
    ///
    /// Bug fixed 2026-08-06: the "no key yet" branch below used to set `previousOffset
    /// = currentSize` and immediately hit the `guard startOffset < currentSize` failure
    /// on the very same call — returning before ever reaching the `defaults.set(...)`
    /// that would have recorded it. That meant `ingestedOffsetKey` could never actually
    /// get written on this path, so `defaults.object(forKey:) == nil` stayed true on
    /// every subsequent launch forever, permanently repeating the same no-op "skip
    /// backlog" branch — real screen captures kept accumulating in the JSONL log (which
    /// `ScreenTextCaptureService` never stopped writing to) but never once reached the
    /// database again after the incremental-ingestion optimization shipped. Fixed by
    /// persisting the offset immediately in that branch, not only after a successful read.
    /// `hasMoreWork` tells a caller driving this on a schedule (`AppState.
    /// scheduleScreenTextIngestStep`) whether to run again immediately (more of the
    /// file queued past this pass's byte cap) or back off (genuinely caught up, or a
    /// transient failure not worth retrying in a tight loop).
    struct ScreenTextIngestOutcome {
        let message: String
        let hasMoreWork: Bool
    }

    /// What to do with a pass's read once no newline turns up in it. Pulled out as a
    /// pure function (over the bytes actually read and whether that read reached the
    /// true end of the file) so the 2026-09-10 stalled-offset bug has a real regression
    /// test without needing to fake the filesystem or `UserDefaults`.
    enum LineScanOutcome: Equatable {
        /// No newline, and the read reached the end of the file: a genuinely partial
        /// trailing line (the capture service is still writing it). Don't advance the
        /// offset — retry from the same spot next pass.
        case waitForMoreData
        /// A complete line ends at this byte index (exclusive) into the read data.
        case foundLine(endIndex: Int)
        /// No newline even after growing the read to `maxLineSearchBytes`, and we're
        /// not at the end of the file: one line is pathologically large. Skip past the
        /// bytes read rather than re-reading the same stuck spot forever.
        case oversizedLine(bytesToSkip: Int)
    }

    static func scanForCompleteLine(in data: Data, reachedEndOfFile: Bool) -> LineScanOutcome {
        if let lastNewline = data.lastIndex(of: 0x0A) {
            return .foundLine(endIndex: lastNewline + 1)
        }
        return reachedEndOfFile ? .waitForMoreData : .oversizedLine(bytesToSkip: data.count)
    }

    static func ingestNewScreenTextLines(into store: CoreMemoryStore, deviceId: String) -> ScreenTextIngestOutcome {
        let logPath = SpikePaths.screenTextLog.path
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: logPath),
              let currentSize = attrs[.size] as? UInt64 else {
            return ScreenTextIngestOutcome(message: "(no real screen-text.jsonl data yet to ingest)", hasMoreWork: false)
        }

        let defaults = UserDefaults.standard
        let previousOffset: UInt64
        if defaults.object(forKey: ingestedOffsetKey) == nil {
            // First launch ever under this scheme: skip the existing backlog (already
            // mostly in the store from earlier sessions) rather than doing a giant
            // catch-up read — but persist that decision now, or this branch repeats
            // forever (see this function's doc comment).
            previousOffset = currentSize
            defaults.set(Int(currentSize), forKey: ingestedOffsetKey)
        } else {
            previousOffset = UInt64(defaults.integer(forKey: ingestedOffsetKey))
        }

        // File shrank/rotated since we last looked — start over from the beginning.
        let startOffset = previousOffset > currentSize ? 0 : previousOffset
        guard startOffset < currentSize else {
            return ScreenTextIngestOutcome(message: "(no new screen-text data since last launch)", hasMoreWork: false)
        }

        guard let handle = FileHandle(forReadingAtPath: logPath) else {
            return ScreenTextIngestOutcome(message: "(couldn't open screen-text.jsonl for incremental read)", hasMoreWork: false)
        }
        defer { try? handle.close() }
        try? handle.seek(toOffset: startOffset)
        let firstStep = min(currentSize - startOffset, UInt64(maxBytesPerIngestPass))
        var newData = handle.readData(ofLength: Int(firstStep))
        var reachedEndOfFile = startOffset + UInt64(newData.count) >= currentSize

        // A single record's line can exceed maxBytesPerIngestPass (see that constant's
        // doc comment) — grow the read once, up to maxLineSearchBytes, before treating
        // "no newline yet" as "genuinely partial, wait for more."
        if newData.lastIndex(of: 0x0A) == nil, !reachedEndOfFile {
            let remainingInFile = currentSize - (startOffset + UInt64(newData.count))
            let extra = min(remainingInFile, UInt64(maxLineSearchBytes) - UInt64(newData.count))
            newData.append(handle.readData(ofLength: Int(extra)))
            reachedEndOfFile = startOffset + UInt64(newData.count) >= currentSize
        }

        switch Self.scanForCompleteLine(in: newData, reachedEndOfFile: reachedEndOfFile) {
        case .waitForMoreData:
            return ScreenTextIngestOutcome(
                message: "(no complete screen-text line within this pass yet — will retry next launch)",
                hasMoreWork: false
            )

        case .oversizedLine(let bytesToSkip):
            let endOffset = startOffset + UInt64(bytesToSkip)
            defaults.set(Int(endOffset), forKey: ingestedOffsetKey)
            return ScreenTextIngestOutcome(
                message: "FAIL: a single screen-text line exceeded \(maxLineSearchBytes) bytes with no newline — skipped \(bytesToSkip) bytes rather than stall forever",
                hasMoreWork: endOffset < currentSize
            )

        case .foundLine(let endIndex):
            newData = newData.prefix(endIndex)
            let endOffset = startOffset + UInt64(endIndex)
            defaults.set(Int(endOffset), forKey: ingestedOffsetKey)

            let deltaURL = SpikePaths.root.appendingPathComponent("screen-text-delta.jsonl")
            do {
                try newData.write(to: deltaURL)
            } catch {
                return ScreenTextIngestOutcome(message: "FAIL: couldn't write screen-text delta scratch file: \(error)", hasMoreWork: false)
            }

            guard let stats = store.ingestJSONL(path: deltaURL, deviceId: deviceId) else {
                return ScreenTextIngestOutcome(message: "FAIL: incremental screen-text ingest returned nil", hasMoreWork: false)
            }
            let remaining = currentSize - endOffset
            let remainingNote = remaining > 0 ? " (\(remaining) bytes still queued for a later pass)" : ""
            return ScreenTextIngestOutcome(
                message: "Ingest new screen-text.jsonl lines (\(newData.count) bytes): read=\(stats.recordsRead) inserted=\(stats.itemsInserted) deduped=\(stats.itemsDeduped) errors=\(stats.parseErrors)\(remainingNote)",
                hasMoreWork: remaining > 0
            )
        }
    }

    private static func writeReport(_ lines: [String]) {
        let reportPath = SpikePaths.root.appendingPathComponent("core-selftest.txt")
        try? lines.joined(separator: "\n").write(to: reportPath, atomically: true, encoding: .utf8)
    }
}
