import Foundation
import OSLog

/// Skins Task 16: images approved before approvals were downscaled (up to 10 MB each) are downscaled in
/// place once, to at most 1600 px on the long side, before the first push, so the hash sent always matches
/// the file on disk. Every file in each skin's `images/` is done, not only the placed ones, since undo can
/// place a kept one again. Scrims are worked out from the decoded image (`SkinImageResolver`), so the
/// reload that follows a migration recomputes them. Guarded by a `UserDefaults` flag.
enum SkinImageMigration {
    static let doneKey = "com.mpointer.kestrel.skinImagesDownscaled"
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")

    /// Runs until one pass rewrites every file it needs to (then never again for `defaults`); true when a file changed (the caller reloads the skins). Call with
    /// `UserSkinStore.writing` held, off the main thread.
    static func runIfNeeded(skins: UserSkinStore, defaults: UserDefaults) -> Bool {
        guard !defaults.bool(forKey: doneKey) else { return false }
        let fm = FileManager.default
        var changed = false
        var failed = false
        for id in skins.skinIds() {
            guard let images = try? skins.folder(id).appendingPathComponent("images", isDirectory: true),
                  let names = try? fm.contentsOfDirectory(atPath: images.path) else { continue }
            for name in names {
                let url = images.appendingPathComponent(name)
                // Regular files only: a link could point outside the skin.
                let kind = try? url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
                guard kind?.isRegularFile == true, kind?.isSymbolicLink != true,
                      let data = fm.contents(atPath: url.path),
                      let small = CoreSkinMaker.downscale(data), small != data else { continue }
                do {
                    try small.write(to: url, options: .atomic)
                    changed = true
                } catch {
                    failed = true
                    log.error("skin image downscale failed for \(id, privacy: .public)/\(name, privacy: .public): \(String(describing: error), privacy: .public)")
                }
            }
        }
        // A file that couldn't be rewritten is tried again next round.
        if !failed { defaults.set(true, forKey: doneKey) }
        return changed
    }
}
