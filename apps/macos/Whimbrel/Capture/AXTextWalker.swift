import ApplicationServices
import Foundation

/// FR-CAP-1: walks the Accessibility tree of a window to collect its visible text.
enum AXTextWalker {
    struct Extraction {
        var text: String
        var url: String?
        /// The page address of a browser whose page content is skipped. Used only to check
        /// site exclusions; never stored.
        var exclusionURL: String? = nil
    }

    static func extract(from window: AXUIElement, maxNodes: Int = 400, maxDepth: Int = 40, skipWebAreas: Bool = false) -> Extraction {
        var collected: [String] = []
        var foundURL: String?
        var checkURL: String?
        var visited = 0
        walk(window, depth: 0, maxDepth: maxDepth, visited: &visited, maxNodes: maxNodes, skipWebAreas: skipWebAreas, collected: &collected, url: &foundURL, checkURL: &checkURL)
        return Extraction(text: collected.joined(separator: "\n"), url: foundURL, exclusionURL: checkURL)
    }

    /// Password fields never, and a Chromium browser's page content and URL: its tree is on
    /// only for meeting detection, and must not widen what screen memory reads.
    static func skips(role: String?, subrole: String?, skipWebAreas: Bool) -> Bool {
        subrole == kAXSecureTextFieldSubrole || (skipWebAreas && role == "AXWebArea")
    }

    private static func walk(
        _ element: AXUIElement,
        depth: Int,
        maxDepth: Int,
        visited: inout Int,
        maxNodes: Int,
        skipWebAreas: Bool,
        collected: inout [String],
        url: inout String?,
        checkURL: inout String?
    ) {
        guard depth < maxDepth, visited < maxNodes else { return }
        visited += 1

        let role = stringAttribute(element, kAXRoleAttribute)
        if skipWebAreas, checkURL == nil {
            if role == "AXWebArea" {
                checkURL = urlAttribute(element)
            } else if role == "AXTextField" {
                checkURL = addressBarURL(description: stringAttribute(element, kAXDescriptionAttribute),
                                         value: stringAttribute(element, kAXValueAttribute))
            }
        }
        if skips(role: role, subrole: stringAttribute(element, kAXSubroleAttribute), skipWebAreas: skipWebAreas) { return }
        if url == nil, role == "AXWebArea" {
            url = urlAttribute(element)
        }

        if let value = stringAttribute(element, kAXValueAttribute), !value.isEmpty {
            collected.append(value)
        } else if let description = stringAttribute(element, kAXDescriptionAttribute), !description.isEmpty {
            collected.append(description)
        }

        var childrenRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
              let children = childrenRef as? [AXUIElement] else { return }

        for child in children {
            if visited >= maxNodes { return }
            walk(child, depth: depth + 1, maxDepth: maxDepth, visited: &visited, maxNodes: maxNodes, skipWebAreas: skipWebAreas, collected: &collected, url: &url, checkURL: &checkURL)
        }
    }

    /// A Chromium address bar's text as a URL for the exclusion check. The bar drops the
    /// scheme ("mail.google.com/mail"), so one is added.
    // ponytail: matches the English "Address and search bar" label; other UI languages fall
    // back to the page's URL, which is only readable while the browser's tree is on.
    static func addressBarURL(description: String?, value: String?) -> String? {
        guard description?.lowercased().contains("address") == true,
              let value = value?.trimmingCharacters(in: .whitespaces), !value.isEmpty, !value.contains(" ")
        else { return nil }
        return value.contains("://") ? value : "https://" + value
    }

    /// Not `private` — `BrowserMeetingCheck` reuses this exact attribute-reading
    /// pattern for a narrower, differently-scoped AX check (frontmost browser's Meet
    /// tab, not full text extraction).
    static func stringAttribute(_ element: AXUIElement, _ attribute: String) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else { return nil }
        return value as? String
    }

    static func urlAttribute(_ element: AXUIElement) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, "AXURL" as CFString, &value) == .success, let value else { return nil }
        if let url = value as? URL { return url.absoluteString }
        if CFGetTypeID(value) == CFURLGetTypeID() {
            return (value as! CFURL as URL).absoluteString
        }
        return nil
    }

    static func frame(of element: AXUIElement) -> CGRect? {
        var positionRef: CFTypeRef?
        var sizeRef: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &positionRef) == .success,
              AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &sizeRef) == .success else {
            return nil
        }
        var position = CGPoint.zero
        var size = CGSize.zero
        if let positionValue = positionRef, CFGetTypeID(positionValue) == AXValueGetTypeID() {
            AXValueGetValue((positionValue as! AXValue), .cgPoint, &position)
        }
        if let sizeValue = sizeRef, CFGetTypeID(sizeValue) == AXValueGetTypeID() {
            AXValueGetValue((sizeValue as! AXValue), .cgSize, &size)
        }
        return CGRect(origin: position, size: size)
    }
}
