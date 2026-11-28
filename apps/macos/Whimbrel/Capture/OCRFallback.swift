import CoreGraphics

/// FR-CAP-2: Vision OCR fallback for windows with no usable Accessibility text
/// (canvas-rendered Electron apps, images, remote desktops, video). The rendered
/// image is held in memory only for recognition and is never written to disk.
enum OCRFallback {
    /// Accessibility text shorter than this (trimmed) is treated as "no usable text" and
    /// OCR is tried instead. Shared by screen capture and the on-demand window reader.
    static let minimumAccessibilityTextLength = 8

    static func recognizeText(windowID: CGWindowID, bounds: CGRect) -> String? {
        guard let image = windowImage(windowID: windowID, bounds: bounds) else { return nil }
        return TextRecognizer.recognize(image)
    }

    /// The window's current image, in memory only.
    static func windowImage(windowID: CGWindowID, bounds: CGRect) -> CGImage? {
        CGWindowListCreateImage(bounds, .optionIncludingWindow, windowID, [.boundsIgnoreFraming])
    }

    /// Side of the grayscale thumbnail `fingerprint` hashes.
    static let fingerprintSide = 64

    /// A cheap fingerprint of `image`: the image drawn into a 64x64 grayscale thumbnail, hashed
    /// (FNV-1a). Two images with the same fingerprint look the same at thumbnail size, so OCR of
    /// the second can reuse the first's text. Drawing the thumbnail costs well under a
    /// millisecond, against tens to hundreds for Vision's accurate OCR. Nil if the thumbnail
    /// cannot be drawn. A change too small to show at 64x64 is missed until the next change that
    /// does show, which is the trade-off that lets an idle window skip OCR.
    static func fingerprint(of image: CGImage) -> UInt64? {
        let side = fingerprintSide
        var pixels = [UInt8](repeating: 0, count: side * side)
        let drawn: Bool = pixels.withUnsafeMutableBytes { buffer in
            guard let context = CGContext(
                data: buffer.baseAddress, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side,
                space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue
            ) else { return false }
            context.interpolationQuality = .low
            context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
            return true
        }
        guard drawn else { return nil }
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in pixels {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        // The size is part of the identity: a resized window is a different image.
        hash ^= UInt64(image.width) << 32 | UInt64(image.height)
        return hash
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
