import AppKit

/// The bitmap for a skin's imagery entry: `user-` skins read their own `images/` folder, built-ins
/// the bundle's `skins/images/`. nil when the asset is not a plain file name, or the file is missing
/// or not an image, or a symlink is on its way inside the skin. Decoded images and their scrims are
/// cached until the skins are installed again.
enum SkinImageResolver {
    /// Where `user-` skins live; tests point it at a temp folder.
    static var userSkins: UserSkinStore = .shared
    /// Main thread only (views call it). Not an NSCache: that may evict at any time, and the images are the
    /// installed skins' few files, dropped on every reinstall.
    // ponytail: unbounded between reloads; switch to an LRU if skins ever carry many large images.
    private static var cache: [URL: NSImage] = [:]

    static func image(for image: SkinImage, skin: Skin) -> NSImage? {
        // A plain file name only: no "..", "." or path, so nothing outside `images/` is read.
        let asset = image.asset
        guard !asset.isEmpty, asset != ".", asset != "..", !asset.contains("/") else { return nil }
        let url = skin.id.hasPrefix("user-")
            ? userSkins.imageURL(asset: asset, skinId: skin.id)
            : SkinStore.bundledSkinsDirectory?.appendingPathComponent("images").appendingPathComponent(asset)
        guard let url else { return nil }
        if let cached = cache[url] { return cached }
        guard staysInside(url, depth: skin.id.hasPrefix("user-") ? 3 : 2), let loaded = NSImage(contentsOf: url), loaded.isValid else { return nil }
        cache[url] = loaded
        return loaded
    }

    /// True when no symlink sits on the last `depth` components of `url` (`<id>/images/<file>` or
    /// `images/<file>`): its real path is its base's real path plus those same components.
    private static func staysInside(_ url: URL, depth: Int) -> Bool {
        var base = url
        for _ in 0..<depth { base.deleteLastPathComponent() }
        let expected = url.pathComponents.suffix(depth).reduce(base.resolvingSymlinksInPath()) { $0.appendingPathComponent($1) }
        return url.resolvingSymlinksInPath().path == expected.path
    }

    /// Scrim opacities per image (weakly held) and text/scrim colours, worked out once.
    private static let scrims = NSMapTable<NSImage, NSMutableDictionary>(keyOptions: [.weakMemory, .objectPointerPersonality], valueOptions: .strongMemory)

    /// `SkinImageryView.scrimOpacity` for `image`, sampled once per image and colours.
    static func scrimOpacity(for image: NSImage, text: [UInt32], scrim: UInt32, glow: (accent: UInt32, text: [UInt32])? = nil) -> Double {
        let key = "\(text)-\(scrim)-\(glow.map { "\($0.accent)-\($0.text)" } ?? "none")" as NSString
        let known = scrims.object(forKey: image) ?? NSMutableDictionary()
        if let cached = known[key] as? Double { return cached }
        let (lightest, darkest) = SkinImageryView.extremes(of: image, page: scrim)
        let opacity = SkinImageryView.scrimOpacity(text: text, scrim: scrim, lightest: lightest, darkest: darkest, glow: glow)
        known[key] = opacity
        scrims.setObject(known, forKey: image)
        return opacity
    }

    /// Drops every decoded image (`SkinStore.install`, which every reload of the skins goes through).
    static func invalidate() {
        cache.removeAll()
        scrims.removeAllObjects()
    }
}
