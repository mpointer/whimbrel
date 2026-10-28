import Foundation

/// FR-CAP-3: captured text with metadata. No screenshots persisted.
struct ScreenTextRecord: Codable {
    let timestamp: Date
    let bundleId: String
    let windowTitle: String
    let url: String?
    let text: String
    /// "ax" (Accessibility API) or "ocr" (Vision fallback, FR-CAP-2).
    let source: String

    enum CodingKeys: String, CodingKey {
        case timestamp, text, source, url
        case bundleId = "bundle_id"
        case windowTitle = "window_title"
    }
}
