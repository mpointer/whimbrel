import AppKit
import SwiftUI

/// Renders a SwiftUI view through an unshown window. `ImageRenderer` draws nothing inside a
/// `ScrollView` (Today's content sits in one), so this hosts the view in an `NSHostingView`
/// instead. Synchronous and main thread only: the skin and palette overrides are global.
@MainActor
enum OffscreenRender {
    static func bitmap(of view: some View, size: CGSize, colorScheme: ColorScheme, scale: Int, settle: Bool = true) -> NSBitmapImageRep? {
        let host = NSHostingView(rootView: view.environment(\.colorScheme, colorScheme))
        let window = NSWindow(contentRect: CGRect(origin: .zero, size: size), styleMask: [], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)
        window.contentView = host
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        // Spinning the run loop lets the rest of the app run, so previews (which hold a global override) pass `settle: false`.
        if settle { RunLoop.main.run(until: Date().addingTimeInterval(0.3)) }
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(size.width) * scale, pixelsHigh: Int(size.height) * scale, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        ) else { return nil }
        rep.size = size
        host.cacheDisplay(in: host.bounds, to: rep)
        return rep
    }
}
