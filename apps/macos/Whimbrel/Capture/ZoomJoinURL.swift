import Foundation

/// Turns a calendar Zoom link (`https://<host>.zoom.us/j/<digits>?pwd=...`) into a
/// `zoommtg://` URL that opens the meeting straight in the Zoom app.
enum ZoomJoinURL {
    static func make(from link: URL) -> URL? {
        guard let host = link.host?.lowercased(),
              host == "zoom.us" || host.hasSuffix(".zoom.us")
        else { return nil }

        let parts = link.path.split(separator: "/")
        guard parts.count == 2, parts[0] == "j",
              !parts[1].isEmpty, parts[1].allSatisfy({ $0.isASCII && $0.isNumber })
        else { return nil }

        // URLQueryItem leaves a literal "+" in place, which a server reads as a space,
        // so the password is encoded strictly here.
        var query = "action=join&confno=\(parts[1])"
        let pwd = URLComponents(url: link, resolvingAgainstBaseURL: false)?
            .queryItems?.first { $0.name == "pwd" }?.value
        let unreserved = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        if let pwd, let encoded = pwd.addingPercentEncoding(withAllowedCharacters: unreserved) {
            query += "&pwd=" + encoded
        }

        var components = URLComponents()
        components.scheme = "zoommtg"
        components.host = "zoom.us"
        components.path = "/join"
        components.percentEncodedQuery = query
        return components.url
    }
}
