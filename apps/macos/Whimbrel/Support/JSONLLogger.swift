import Foundation

/// Appends one JSON object per line. `screenText` is the capture queue: `ScreenTextCaptureService`
/// appends to it and `CoreSelfTest.ingestNewScreenTextLines` reads it into the store, then empties
/// it (`trimIfFullyRead`) so it never grows past what is waiting to be saved.
final class JSONLLogger {
    /// The one logger for `screen-text.jsonl`. Trimming must go through the handle that writes,
    /// so there is only ever one.
    static let screenText = JSONLLogger(url: SpikePaths.screenTextLog)

    private let queue = DispatchQueue(label: "com.mpointer.kestrel.jsonl-logger")
    private var handle: FileHandle?
    private let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }()

    init(url: URL) {
        if !FileManager.default.fileExists(atPath: url.path) {
            FileManager.default.createFile(atPath: url.path, contents: nil)
        }
        handle = try? FileHandle(forWritingTo: url)
        try? handle?.seekToEnd()
    }

    func append<T: Encodable>(_ record: T) {
        queue.async { [weak self] in
            guard let self, let handle = self.handle,
                  let data = try? self.encoder.encode(record) else { return }
            handle.write(data)
            handle.write("\n".data(using: .utf8)!)
        }
    }

    /// Empties the file when every byte written so far has been read (`offset` is where the
    /// reader got to). Runs on the writing queue, so no append lands between the check and the
    /// truncate, and the next append starts at byte 0 rather than past a shrunken end.
    /// False, changing nothing, while unread bytes remain.
    /// `beforeTruncate` runs inside that same block, first: the reader resets its offset there, so
    /// a crash can only leave it at 0 over a file still full (re-read and deduplicated), never
    /// past the end of an emptied one.
    func trimIfFullyRead(through offset: UInt64, beforeTruncate: () -> Void = {}) -> Bool {
        queue.sync {
            guard let handle, let end = try? handle.offset(), end == offset, end > 0 else { return false }
            beforeTruncate()
            do {
                try handle.truncate(atOffset: 0)
                try handle.seek(toOffset: 0)
                return true
            } catch {
                return false
            }
        }
    }

    /// Returns once every append queued so far is written. For tests.
    func waitUntilWritten() {
        queue.sync {}
    }
}
