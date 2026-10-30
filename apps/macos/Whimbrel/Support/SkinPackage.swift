import Foundation
import OSLog
import UniformTypeIdentifiers

/// A `.whimbrelskin` file: a zip of one of your skins' folders (`<id>/skin.json`, `palettes/*.json`,
/// `images/*`), never its chat or history. Import goes through the same validation as any skin and
/// adds a copy, never overwriting. See docs/superpowers/specs/2026-10-21-skins-r2-conversation-design.md.
enum SkinPackage {
    static let utType = UTType(exportedAs: "com.mpointer.kestrel.skin", conformingTo: .zip)
    /// Zip-bomb caps: the package file, the listed files, and every byte read out of it.
    static let maxFiles = 200
    static let maxBytes = 64 * 1024 * 1024

    enum PackageError: Error, Equatable { case unreadable, unsafeEntry(String), tooLarge, tooLargeToExport, invalidSkin(String), encrypted, timedOut }

    private static let fm = FileManager.default
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")

    /// Writes the skin as a package at `url`: its manifest, palettes and the images it places. Throws
    /// `.tooLargeToExport` when the files would exceed import's caps (`maxFiles`, `maxBytes`).
    static func export(skinId: String, from store: UserSkinStore, to url: URL, maxFiles: Int = maxFiles, maxBytes: Int = maxBytes) throws {
        guard let manifest = store.manifest(for: skinId) else { throw PackageError.invalidSkin(skinId) }
        let staging = fm.temporaryDirectory.appendingPathComponent("whimbrelskin-\(UUID().uuidString)")
        defer { try? fm.removeItem(at: staging) }
        let top = staging.appendingPathComponent(skinId, isDirectory: true)
        try fm.createDirectory(at: top.appendingPathComponent("palettes"), withIntermediateDirectories: true)
        try manifest.write(to: top.appendingPathComponent("skin.json"))
        for palette in store.palettes(for: skinId) {
            try palette.data.write(to: top.appendingPathComponent("palettes/\(palette.id).json"))
        }
        // Only the images this version places (undo, refines and later approvals leave others behind).
        let images = try store.folder(skinId).appendingPathComponent("images", isDirectory: true)
        let imagery = ((try? JSONSerialization.jsonObject(with: manifest)) as? [String: Any])
            .flatMap { ($0["decoration"] as? [String: Any])?["imagery"] as? [Any] } ?? []
        let names = Set(imagery.compactMap { ($0 as? [String: Any])?["asset"] as? String }).filter { !$0.contains("/") && $0 != "." && $0 != ".." }.sorted()
        if !names.isEmpty { try fm.createDirectory(at: top.appendingPathComponent("images"), withIntermediateDirectories: true) }
        for name in names where (try? fm.attributesOfItem(atPath: images.appendingPathComponent(name).path))?[.type] as? FileAttributeType == .typeRegular {
            try fm.copyItem(at: images.appendingPathComponent(name), to: top.appendingPathComponent("images/\(name)"))
        }
        // What import would refuse (`readPackage`'s caps) is refused here, so your own export always imports.
        let files = fm.enumerator(at: top, includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey])?.compactMap { $0 as? URL }
            .filter { (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true } ?? []
        let bytes = files.reduce(0) { $0 + ((try? $1.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) }
        guard files.count <= maxFiles, bytes <= maxBytes else { throw PackageError.tooLargeToExport }
        // Zipped beside the destination first, so a failure leaves the user's existing file alone.
        let replacement = try fm.url(for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: url, create: true)
        defer { try? fm.removeItem(at: replacement) }
        let zipped = replacement.appendingPathComponent(url.lastPathComponent)
        _ = try output("/usr/bin/ditto", ["-c", "-k", "--keepParent", "--norsrc", "--noextattr", "--noqtn", "--noacl", top.path, zipped.path], limit: 64 * 1024, timeout: 120)
        if fm.fileExists(atPath: url.path) {
            _ = try fm.replaceItemAt(url, withItemAt: zipped)
        } else {
            try fm.moveItem(at: zipped, to: url)
        }
    }

    /// Adds the package as a new skin of yours and returns its id: `user-` added when missing, and
    /// on an id or name clash with one of yours, id `-2`, name " 2" (and so on). Its palettes become
    /// `<id>-p<N>` and its history starts at version 1.
    ///
    /// Nothing is extracted: `zipinfo` lists the central directory, each listed entry is read with
    /// `unzip -p` (the same parser and directory) into memory under a byte budget, and only the
    /// validated bytes reach the store, through `addImage`/`save`. Entries the directory doesn't list
    /// are never read. The package is rejected unless every entry is `<top>/skin.json`,
    /// `<top>/palettes/<name>.json` or `<top>/images/<name>` (or those folders), the files are regular
    /// files by the directory's mode, and the skin, palettes and images pass the usual checks.
    /// `timeLimit` bounds the whole import (every tool it runs shares it); past it, `.timedOut`.
    static func importPackage(at url: URL, into store: UserSkinStore, timeLimit: TimeInterval = 30) throws -> String {
        let (manifestData, paletteFiles, imageFiles) = try readPackage(url, deadline: Date() + timeLimit)

        guard let archetype = SkinLoader.parse(manifestData, base: SkinDefaults.whimbrel)?.0.archetype,
              var manifest = (try? JSONSerialization.jsonObject(with: manifestData)) as? [String: Any],
              let oldId = manifest["id"] as? String, let rawName = manifest["name"] as? String else {
            throw PackageError.invalidSkin("skin.json")
        }
        for (name, data) in paletteFiles where ThemeLoader.parse(data) == nil { throw PackageError.invalidSkin(name) }
        for (name, data) in imageFiles where !SkinChatSession.isUsableImage(data) { throw PackageError.invalidSkin(name) }

        // New id and name, suffixed together until neither clashes with one of yours.
        let baseId = oldId.hasPrefix("user-") ? oldId : "user-\(oldId)"
        let ids = Set(store.skinIds())
        let names = Set(ids.compactMap { (store.manifest(for: $0).flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any])?["name"] as? String })
        var n = 1
        // Capped first, so a name of only spaces counts as missing.
        let name = UserSkinStore.skinName(rawName).isEmpty ? "My skin" : rawName
        var newId = baseId, newName = UserSkinStore.skinName(name)
        while ids.contains(newId) || names.contains(newName) {
            n += 1
            newId = "\(baseId)-\(n)"
            newName = UserSkinStore.skinName(name, suffix: " \(n)")
        }

        // Palettes the manifest lists and the package carries become `<newId>-p<N>`. A listed id the
        // package doesn't carry stays only when it is a built-in palette (user palettes are all
        // `user-…`), so the skin never borrows another of your skins' palettes. Unlisted files are dropped.
        var palettes: [(id: String, data: Data)] = []
        var newPaletteIds: [String: String] = [:]
        manifest["palettes"] = ((manifest["palettes"] as? [Any]) ?? []).compactMap { ref -> String? in
            guard let ref = ref as? String else { return nil }
            if let id = newPaletteIds[ref] { return id }
            guard let data = paletteFiles["\(ref).json"],
                  var palette = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
                return ref.hasPrefix("user-") ? nil : ref
            }
            let id = "\(newId)-p\(palettes.count + 1)"
            palette["id"] = id
            guard var rewritten = try? JSONSerialization.data(withJSONObject: palette, options: [.prettyPrinted, .sortedKeys]) else { return nil }
            // A palette that doesn't suit the skin's archetype is repaired, as one the chat made would be.
            if let theme = ThemeLoader.parse(rewritten), let fixed = SkinChatSession.archetypeRepair(theme, id: id, archetype: archetype) {
                rewritten = fixed.data
            }
            palettes.append((id: id, data: rewritten))
            newPaletteIds[ref] = id
            return id
        }
        manifest["id"] = newId
        manifest["name"] = newName

        do {
            // Images first: the store gives each its flat name, and the placements follow it.
            var renamed: [String: String] = [:]
            for (name, data) in imageFiles.sorted(by: { $0.key < $1.key }) {
                // Kept at most 1600 px on the long side, as an approved image is (skins Task 16).
                renamed[name] = try store.addImage(CoreSkinMaker.downscale(data) ?? data, preferredName: name, to: newId)
            }
            if var decoration = manifest["decoration"] as? [String: Any], let imagery = decoration["imagery"] as? [Any] {
                decoration["imagery"] = imagery.map { entry -> Any in
                    guard var d = entry as? [String: Any], let asset = d["asset"] as? String, let name = renamed[asset] else { return entry }
                    d["asset"] = name
                    return d
                }
                manifest["decoration"] = decoration
            }
            let data = try JSONSerialization.data(withJSONObject: manifest, options: [.prettyPrinted, .sortedKeys])
            _ = try store.save(manifest: data, palettes: palettes, for: newId)
        } catch {
            do { try store.removeFolder(skinId: newId) } catch let cleanup {
                log.error("import cleanup failed for \(newId, privacy: .public): \(String(describing: cleanup), privacy: .public)")
            }
            throw error
        }
        return newId
    }

    /// The package's skin.json, palettes and images by file name, read from the central directory only.
    private static func readPackage(_ link: URL, deadline: Date) throws -> (manifest: Data, palettes: [String: Data], images: [String: Data]) {
        // The real file, which must be a readable regular file within the cap (fails closed).
        let url = link.resolvingSymlinksInPath()
        guard let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey]),
              values.isRegularFile == true, let size = values.fileSize else { throw PackageError.unreadable }
        guard size <= maxBytes else { throw PackageError.tooLarge }

        // The entry count from the header first, so a huge listing is never read (+3 for the folders).
        let header = String(decoding: try output("/usr/bin/zipinfo", ["-h", url.path], limit: 64 * 1024, deadline: deadline), as: UTF8.self)
        guard let count = header.components(separatedBy: "number of entries: ").last.flatMap({ Int($0.trimmingCharacters(in: .whitespacesAndNewlines)) }) else {
            throw PackageError.unreadable
        }
        guard count <= maxFiles + 3 else { throw PackageError.tooLarge }
        let listingLimit = 4 * 1024 * 1024
        let names = String(decoding: try output("/usr/bin/zipinfo", ["-1", url.path], limit: listingLimit, deadline: deadline), as: UTF8.self)
        let long = String(decoding: try output("/usr/bin/zipinfo", ["-l", url.path], limit: listingLimit, deadline: deadline), as: UTF8.self)  // header, `-rw-r--r-- … TX … name` per entry, totals
        let entries = names.split(separator: "\n", omittingEmptySubsequences: false).dropLast().map(String.init)
        let rows = long.split(separator: "\n").dropFirst(2).dropLast()
        guard rows.count == entries.count, entries.count == count, Set(entries).count == entries.count else { throw PackageError.unreadable }

        // Every name is checked before any is used: one top folder and the allowed paths, with plain
        // components only (no `..`, leading `.` or `-`, control characters, or any of `\[]*?^`, which
        // unzip would read as a pattern), so each name selects exactly its own entry.
        func plain(_ part: Substring) -> Bool {
            !part.isEmpty && !part.hasPrefix(".") && !part.hasPrefix("-")
                && !part.unicodeScalars.contains { $0.value < 0x20 || $0.value == 0x7F || "\\[]*?^".unicodeScalars.contains($0) }
        }
        var files: [(name: String, kind: String, file: String)] = []
        var top: Substring?
        for (row, name) in zip(rows, entries) {
            let columns = row.split(separator: " ")
            // The type/flags column (`bx`, `tX`): upper-case first letter means encrypted.
            guard columns.count > 4, columns[4].first?.isUppercase != true else { throw PackageError.encrypted }
            let isDir = name.hasSuffix("/")
            let parts = (isDir ? name.dropLast() : Substring(name)).split(separator: "/", omittingEmptySubsequences: false)
            guard row.first == (isDir ? "d" : "-"), parts.allSatisfy(plain), top == nil || parts[0] == top else { throw PackageError.unsafeEntry(name) }
            top = parts[0]
            switch (parts.count, isDir) {
            case (1, true): break
            case (2, true) where parts[1] == "palettes" || parts[1] == "images": break
            case (2, false) where parts[1] == "skin.json": files.append((name, "skin", "skin.json"))
            case (3, false) where parts[1] == "palettes" && parts[2].hasSuffix(".json"): files.append((name, "palette", String(parts[2])))
            case (3, false) where parts[1] == "images": files.append((name, "image", String(parts[2])))
            default: throw PackageError.unsafeEntry(name)
            }
        }
        guard files.count <= maxFiles else { throw PackageError.tooLarge }

        var budget = maxBytes
        var manifest: Data?, palettes: [String: Data] = [:], images: [String: Data] = [:]
        for f in files {
            let data = try readEntry(f.name, of: url, limit: min(budget, SkinChatSession.imageByteLimit), deadline: deadline)
            budget -= data.count
            switch f.kind {
            case "skin": manifest = data
            case "palette": palettes[f.file] = data
            default: images[f.file] = data
            }
        }
        guard let manifest else { throw PackageError.invalidSkin("no skin.json") }
        return (manifest, palettes, images)
    }

    /// One entry's bytes via `unzip -p` (never written to disk), with an empty password so an
    /// encrypted entry fails instead of prompting. `name` must already be a plain path.
    static func readEntry(_ name: String, of package: URL, limit: Int, deadline: Date = .distantFuture) throws -> Data {
        try output("/usr/bin/unzip", ["-P", "", "-p", package.path, name], limit: limit, deadline: deadline)
    }

    /// A tool's stdout, read into memory. Past `limit` bytes it is terminated (`.tooLarge`); after
    /// `timeout` seconds or at `deadline`, whichever is first, too (`.timedOut`); any other non-zero
    /// exit is `.unreadable`. No stdin, no
    /// stderr, and an empty environment (so `$UNZIP`/`$ZIPINFO` options can't change matching).
    private static func output(_ tool: String, _ arguments: [String], limit: Int, timeout: TimeInterval = 10, deadline: Date = .distantFuture) throws -> Data {
        let seconds = min(timeout, deadline.timeIntervalSinceNow)
        guard seconds > 0 else { throw PackageError.timedOut }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: tool)
        process.arguments = arguments
        process.environment = [:]
        process.standardInput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        let pipe = Pipe()
        process.standardOutput = pipe
        do { try process.run() } catch { throw PackageError.unreadable }
        // Terminating closes the pipe, which ends the read loop below.
        let deadline = DispatchWorkItem { [process] in if process.isRunning { process.terminate() } }
        DispatchQueue.global().asyncAfter(deadline: .now() + seconds, execute: deadline)
        defer { deadline.cancel() }
        var data = Data()
        while true {
            let chunk = pipe.fileHandleForReading.availableData
            if chunk.isEmpty { break }
            data.append(chunk)
            if data.count > limit {
                process.terminate()
                process.waitUntilExit()
                throw PackageError.tooLarge
            }
        }
        process.waitUntilExit()
        guard process.terminationReason == .exit else { throw PackageError.timedOut }  // only the deadline signals it here
        guard process.terminationStatus == 0 else { throw PackageError.unreadable }
        return data
    }
}
