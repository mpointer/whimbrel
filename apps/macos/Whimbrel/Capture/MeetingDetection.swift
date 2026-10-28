import Foundation

/// FR-MTG-1: a video-conferencing provider whose link we can recognize in a calendar
/// event's location, notes, or URL field.
enum MeetingProvider: String, Codable {
    case zoom
    case googleMeet
    case teams
    case webex
}

struct DetectedConferenceLink: Equatable, Codable {
    let provider: MeetingProvider
    let url: URL
}

/// FR-MTG-1: "events containing a conferencing link" — pure string matching, no
/// EventKit dependency, so this is testable without Calendar access or any TCC grant.
enum MeetingLinkDetector {
    /// Checks `location`, then `notes`, then `url` (in that order — organizers most
    /// often put the join link in Location) and returns the first recognized
    /// conferencing link, or nil if none of them contain one.
    static func detectLink(location: String?, notes: String?, url: URL?) -> DetectedConferenceLink? {
        for candidate in [location, notes, url?.absoluteString] {
            guard let candidate, let link = firstLink(in: candidate) else { continue }
            return link
        }
        return nil
    }

    /// The patterns (Zoom, Google Meet, Teams, Webex; trailing prose punctuation
    /// trimmed) live in core/api::connectors::calendar so EventKit and the Google
    /// connector recognize exactly the same links (M3.0 Task 8).
    private static func firstLink(in text: String) -> DetectedConferenceLink? {
        CoreMemoryStore.detectConferenceLink(in: text)
    }
}
