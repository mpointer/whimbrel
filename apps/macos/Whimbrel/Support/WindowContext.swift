import Foundation

/// The focused window's text, as the hotkey panel hands it to core's
/// `whimbrel_llm_prepare_window_ask`. Encodes as the snake_case JSON core expects; any
/// property added later that is Swift-side only must stay out of `CodingKeys`.
struct WindowContext: Encodable, Equatable {
    let appName: String
    let title: String
    let text: String
    /// True when `text` is what the user had selected, false when it is the visible text.
    let isSelection: Bool
    /// Where `text` came from. Swift-side only: not in `CodingKeys`, so never encoded.
    var source: Source = .accessibility

    enum Source: String {
        case selection, accessibility, ocr
    }

    enum CodingKeys: String, CodingKey {
        case appName = "app_name"
        case title
        case text
        case isSelection = "is_selection"
    }
}
