import Foundation

/// Appends one JSON object per line. Used for the FR-CAP-1..3 screen-text spike log.
final class JSONLLogger {
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
}
