import CoreGraphics

/// FR-CAP-2: Vision OCR fallback for windows with no usable Accessibility text
/// (canvas-rendered Electron apps, images, remote desktops, video). The rendered
/// image is held in memory only for recognition and is never written to disk.
enum OCRFallback {
    static func recognizeText(windowID: CGWindowID, bounds: CGRect) -> String? {
        guard let image = CGWindowListCreateImage(bounds, .optionIncludingWindow, windowID, [.boundsIgnoreFraming]) else {
            return nil
        }

        return TextRecognizer.recognize(image)
    }

    /// Locates the on-screen window ID/bounds matching an Accessibility window element,
    /// via CGWindowListCopyWindowInfo (avoids relying on private AX APIs).
    static func windowID(forPID pid: pid_t, matchingBounds bounds: CGRect?) -> CGWindowID? {
        guard let infoList = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: AnyObject]] else {
            return nil
        }
        let candidates = infoList.filter { ($0[kCGWindowOwnerPID as String] as? pid_t) == pid }

        if let bounds, let match = candidates.first(where: { info in
            guard let boundsDict = info[kCGWindowBounds as String] as? [String: CGFloat] else { return false }
            let x = boundsDict["X"] ?? 0
            let y = boundsDict["Y"] ?? 0
            return abs(x - bounds.origin.x) < 4 && abs(y - bounds.origin.y) < 4
        }) {
            return match[kCGWindowNumber as String] as? CGWindowID
        }
        return candidates.first?[kCGWindowNumber as String] as? CGWindowID
    }
}
