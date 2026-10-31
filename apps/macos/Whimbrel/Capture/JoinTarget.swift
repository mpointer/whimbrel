import Foundation

/// Where the meeting-start pop-up's Join button sends the user: the provider's app
/// when there is a known app URL, otherwise the https link in the default browser.
/// Pure, so it's testable without NSWorkspace.
struct JoinTarget: Equatable {
    let provider: MeetingProvider
    let appURL: URL?
    let webURL: URL

    /// `link` is the event's already-chosen conference link (`CalendarService` /
    /// core pick conference data first, then the first recognized link). Returns nil
    /// unless the link is https on a host that really belongs to its provider — the
    /// detector's regexes alone would accept e.g. `evilzoom.us` or `evilwebex.com`.
    static func make(from link: DetectedConferenceLink?) -> JoinTarget? {
        guard let link,
              link.url.scheme?.lowercased() == "https",
              link.url.user == nil, link.url.password == nil,
              let host = link.url.host?.lowercased(),
              hostBelongs(host, to: link.provider)
        else { return nil }

        let appURL: URL? = switch link.provider {
        case .zoom: ZoomJoinURL.make(from: link.url)
        // Microsoft documents msteams: only for teams.microsoft.com links; consumer
        // teams.live.com links go through the browser, whose page offers the app.
        // Only meeting-join links get the app scheme; any other Teams page stays in the browser.
        case .teams where host == "teams.microsoft.com" && link.url.path.hasPrefix("/l/meetup-join/"): msteamsURL(from: link.url)
        // Webex documents no meeting-join URL scheme (only webexteams: for spaces),
        // so its https join page, which launches the app itself, is the target.
        case .teams, .googleMeet, .webex: nil
        }
        return JoinTarget(provider: link.provider, appURL: appURL, webURL: link.url)
    }

    /// The app URL when its app is installed, else the web link.
    func urlToOpen(appInstalled: (URL) -> Bool) -> URL {
        if let appURL, appInstalled(appURL) { return appURL }
        return webURL
    }

    private static func hostBelongs(_ host: String, to provider: MeetingProvider) -> Bool {
        switch provider {
        case .zoom: host == "zoom.us" || host.hasSuffix(".zoom.us")
        case .googleMeet: host == "meet.google.com"
        case .teams: host == "teams.microsoft.com" || host == "teams.live.com"
        case .webex: host == "webex.com" || host.hasSuffix(".webex.com")
        }
    }

    private static func msteamsURL(from link: URL) -> URL? {
        var components = URLComponents(url: link, resolvingAgainstBaseURL: false)
        components?.scheme = "msteams"
        return components?.url
    }
}
