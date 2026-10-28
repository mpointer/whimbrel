import ApplicationServices
import Foundation

/// FR-CAP-1: walks the Accessibility tree of a window to collect its visible text.
enum AXTextWalker {
    struct Extraction {
        var text: String
        var url: String?
    }

    static func extract(from window: AXUIElement, maxNodes: Int = 400, maxDepth: Int = 40) -> Extraction {
        var collected: [String] = []
        var foundURL: String?
        var visited = 0
        walk(window, depth: 0, maxDepth: maxDepth, visited: &visited, maxNodes: maxNodes, collected: &collected, url: &foundURL)
        return Extraction(text: collected.joined(separator: "\n"), url: foundURL)
    }

    private static func walk(
        _ element: AXUIElement,
        depth: Int,
        maxDepth: Int,
        visited: inout Int,
        maxNodes: Int,
        collected: inout [String],
        url: inout String?
    ) {
        guard depth < maxDepth, visited < maxNodes else { return }
        visited += 1

        if url == nil, let role = stringAttribute(element, kAXRoleAttribute), role == "AXWebArea" {
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
            walk(child, depth: depth + 1, maxDepth: maxDepth, visited: &visited, maxNodes: maxNodes, collected: &collected, url: &url)
        }
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
