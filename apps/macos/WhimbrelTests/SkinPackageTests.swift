import AppKit
import XCTest
@testable import Whimbrel

final class SkinPackageTests: XCTestCase {
    private let fm = FileManager.default

    /// A fresh temp folder, removed after the test.
    private func tempDir(_ label: String) throws -> URL {
        let url = fm.temporaryDirectory.appendingPathComponent("skin-package-\(label)-\(UUID().uuidString)")
        try fm.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func store() throws -> UserSkinStore { UserSkinStore(root: try tempDir("store")) }

    private static let png: Data = {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        return rep.representation(using: .png, properties: [:])!
    }()

    private func json(_ object: Any) -> Data { try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) }

    private func manifest(_ id: String, name: String, palettes: [String], asset: String? = nil) -> Data {
        var root: [String: Any] = ["id": id, "name": name, "palettes": palettes]
        if let asset { root["decoration"] = ["imagery": [["asset": asset, "placement": "header"]]] }
        return json(root)
    }

    private func palette(_ id: String, _ name: String = "Mine") -> Data {
        json(["id": id, "name": name, "light": ["accentPrimary": "#123456"]])
    }

    private func object(_ data: Data?) -> [String: Any] {
        (data.flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]) ?? [:]
    }

    /// A skin with one palette, one image placed in it, a chat and a second version.
    private func makeSkin(in s: UserSkinStore, id: String = "user-sea-1", name: String = "Sea") throws {
        let asset = try s.addImage(Self.png, preferredName: "wave.png", to: id)
        _ = try s.save(manifest: manifest(id, name: name, palettes: ["\(id)-p1"], asset: asset), palettes: [(id: "\(id)-p1", data: palette("\(id)-p1"))], for: id)
        _ = try s.save(manifest: manifest(id, name: name, palettes: ["\(id)-p1"], asset: asset), palettes: [(id: "\(id)-p1", data: palette("\(id)-p1"))], for: id)
        try s.appendChat(SkinChatTurn(role: "user", text: "make it blue", date: Date()), to: id)
    }

    private func zipEntries(_ url: URL) throws -> [String] {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/zipinfo")
        p.arguments = ["-1", url.path]
        let out = Pipe()
        p.standardOutput = out
        try p.run()
        let data = out.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        return String(decoding: data, as: UTF8.self).split(separator: "\n").map(String.init)
    }

    /// Zips `folder` (kept as the archive's top folder) the way export does.
    private func zip(_ folder: URL) throws -> URL {
        let out = try tempDir("zip").appendingPathComponent("hand.whimbrelskin")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        p.arguments = ["-c", "-k", "--keepParent", "--norsrc", folder.path, out.path]
        try p.run()
        p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0)
        return out
    }

    /// A hand-made package folder `<top>/skin.json` + `palettes/<id>.json`.
    private func packageFolder(id: String, name: String = "Hand") throws -> URL {
        let top = try tempDir("hand").appendingPathComponent("skin")
        try fm.createDirectory(at: top.appendingPathComponent("palettes"), withIntermediateDirectories: true)
        try manifest(id, name: name, palettes: ["\(id)-dark"]).write(to: top.appendingPathComponent("skin.json"))
        try palette("\(id)-dark").write(to: top.appendingPathComponent("palettes/\(id)-dark.json"))
        return top
    }

    /// Import throws (`expected`, when given) and adds nothing.
    private func assertRejected(_ package: URL, into s: UserSkinStore, _ expected: SkinPackage.PackageError? = nil, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertThrowsError(try SkinPackage.importPackage(at: package, into: s), file: file, line: line) { error in
            if let expected { XCTAssertEqual(error as? SkinPackage.PackageError, expected, file: file, line: line) }
        }
        XCTAssertEqual(s.skinIds(), [], "nothing imported", file: file, line: line)
    }

    func testRoundTripPreservesSkinPalettesAndImages() throws {
        let source = try store()
        try makeSkin(in: source)
        let file = try tempDir("out").appendingPathComponent("Sea.whimbrelskin")
        try SkinPackage.export(skinId: "user-sea-1", from: source, to: file)

        let target = try store()
        let id = try SkinPackage.importPackage(at: file, into: target)
        XCTAssertEqual(id, "user-sea-1")
        let skin = object(target.manifest(for: id))
        XCTAssertEqual(skin["name"] as? String, "Sea")
        XCTAssertEqual(skin["palettes"] as? [String], ["user-sea-1-p1"])
        XCTAssertEqual(target.palettes(for: id).map(\.id), ["user-sea-1-p1"])
        XCTAssertEqual(ThemeLoader.parse(target.palettes(for: id)[0].data)?.light[.accentPrimary], 0x123456)
        XCTAssertEqual(fm.contents(atPath: target.imageURL(asset: "wave.png", skinId: id).path), Self.png)
        let loaded = target.loadSkinsAndPalettes(base: SkinDefaults.whimbrel)
        XCTAssertEqual(loaded.skins.map(\.id), [id])
        XCTAssertEqual(loaded.palettes.map(\.id), ["user-sea-1-p1"])
        XCTAssertEqual(loaded.skins[0].decoration.imagery.map(\.asset), ["wave.png"])
    }

    func testExportOmitsChatAndHistory() throws {
        let s = try store()
        try makeSkin(in: s)
        let file = try tempDir("out").appendingPathComponent("Sea.whimbrelskin")
        try SkinPackage.export(skinId: "user-sea-1", from: s, to: file)
        XCTAssertEqual(try zipEntries(file).filter { !$0.hasSuffix("/") }.sorted(),
                       ["user-sea-1/images/wave.png", "user-sea-1/palettes/user-sea-1-p1.json", "user-sea-1/skin.json"])

        // The import starts its own history at version 1, with no chat.
        let target = try store()
        let id = try SkinPackage.importPackage(at: file, into: target)
        XCTAssertEqual(target.chat(for: id), [])
        XCTAssertEqual(try target.undo(skinId: id), false, "only one version")
    }

    func testImportAddsPrefix() throws {
        let s = try store()
        let id = try SkinPackage.importPackage(at: try zip(try packageFolder(id: "harbour")), into: s)
        XCTAssertEqual(id, "user-harbour")
        XCTAssertEqual(object(s.manifest(for: id))["id"] as? String, "user-harbour")
        // The palette is tied to the new id, so the loader keeps it.
        XCTAssertEqual(object(s.manifest(for: id))["palettes"] as? [String], ["user-harbour-p1"])
        XCTAssertEqual(s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel).palettes.map(\.id), ["user-harbour-p1"])
    }

    func testImportClashMakesRenamedCopy() throws {
        let s = try store()
        try makeSkin(in: s)
        let before = s.manifest(for: "user-sea-1")
        let file = try tempDir("out").appendingPathComponent("Sea.whimbrelskin")
        try SkinPackage.export(skinId: "user-sea-1", from: s, to: file)

        let id = try SkinPackage.importPackage(at: file, into: s)
        XCTAssertEqual(id, "user-sea-1-2")
        XCTAssertEqual(object(s.manifest(for: id))["name"] as? String, "Sea 2")
        XCTAssertEqual(object(s.manifest(for: id))["palettes"] as? [String], ["user-sea-1-2-p1"])
        XCTAssertEqual(s.manifest(for: "user-sea-1"), before, "existing skin untouched")
        XCTAssertEqual(s.chat(for: "user-sea-1").count, 1)

        // A different id with a clashing name is renamed too ("Sea 2" is taken, so id and name both go to 3).
        let other = try SkinPackage.importPackage(at: try zip(try packageFolder(id: "user-other", name: "Sea")), into: s)
        XCTAssertEqual(other, "user-other-3")
        XCTAssertEqual(object(s.manifest(for: other))["name"] as? String, "Sea 3")
    }

    func testImportRejectsCorruptZip() throws {
        let s = try store()
        let file = try tempDir("bad").appendingPathComponent("bad.whimbrelskin")
        try Data("not a zip".utf8).write(to: file)
        assertRejected(file, into: s, .unreadable)

        // A valid zip whose skin.json is not a skin.
        let top = try packageFolder(id: "user-x")
        try Data("{".utf8).write(to: top.appendingPathComponent("skin.json"))
        assertRejected(try zip(top), into: s, .invalidSkin("skin.json"))

        // A palette that is not a palette.
        let top2 = try packageFolder(id: "user-y")
        try Data(#"{"id":"user-y-dark"}"#.utf8).write(to: top2.appendingPathComponent("palettes/user-y-dark.json"))
        assertRejected(try zip(top2), into: s, .invalidSkin("user-y-dark.json"))
    }

    func testImportRejectsSymlinkAndUnknownFiles() throws {
        let s = try store()
        let linked = try packageFolder(id: "user-l")
        try fm.createDirectory(at: linked.appendingPathComponent("images"), withIntermediateDirectories: true)
        try fm.createSymbolicLink(atPath: linked.appendingPathComponent("images/x.png").path, withDestinationPath: "/etc/hosts")
        assertRejected(try zip(linked), into: s, .unsafeEntry("skin/images/x.png"))

        let linkedDir = try packageFolder(id: "user-d")
        try fm.createSymbolicLink(atPath: linkedDir.appendingPathComponent("images").path, withDestinationPath: "/tmp")
        assertRejected(try zip(linkedDir), into: s, .unsafeEntry("skin/images"))

        let chat = try packageFolder(id: "user-c")
        try Data("[]".utf8).write(to: chat.appendingPathComponent("chat.json"))
        assertRejected(try zip(chat), into: s, .unsafeEntry("skin/chat.json"))

        let notImage = try packageFolder(id: "user-n")
        try fm.createDirectory(at: notImage.appendingPathComponent("images"), withIntermediateDirectories: true)
        try Data("<html>".utf8).write(to: notImage.appendingPathComponent("images/a.png"))
        assertRejected(try zip(notImage), into: s, .invalidSkin("a.png"))

        let nested = try packageFolder(id: "user-m")
        try fm.createDirectory(at: nested.appendingPathComponent("images/sub"), withIntermediateDirectories: true)
        try Self.png.write(to: nested.appendingPathComponent("images/sub/a.png"))
        assertRejected(try zip(nested), into: s, .unsafeEntry("skin/images/sub/"))
    }

    func testImportRejectsZipSlip() throws {
        let s = try store()
        let top = try packageFolder(id: "user-z")
        // "aaaaaaaaaaa/" is as long as "../../../../", so the archive's offsets stay valid after the swap.
        try fm.createDirectory(at: top.appendingPathComponent("palettes/aaaaaaaaaaa"), withIntermediateDirectories: true)
        try palette("e").write(to: top.appendingPathComponent("palettes/aaaaaaaaaaa/e.json"))
        let good = try zip(top)
        let evil = good.deletingLastPathComponent().appendingPathComponent("evil.whimbrelskin")
        try swap("palettes/aaaaaaaaaaa/", for: "palettes/../../../../", in: good, to: evil)
        XCTAssertTrue(try zipEntries(evil).contains("skin/palettes/../../../../e.json"), "fixture really escapes")
        assertRejected(evil, into: s, .unsafeEntry("skin/palettes/../../../../"))

        let plain = try zip(try packageFolder(id: "user-abs"))
        let absolute = plain.deletingLastPathComponent().appendingPathComponent("abs.whimbrelskin")
        try swap("skin/skin.json", for: "/tmp/skin.json", in: plain, to: absolute)
        XCTAssertTrue(try zipEntries(absolute).contains("/tmp/skin.json"), "fixture really absolute")
        assertRejected(absolute, into: s, .unsafeEntry("/tmp/skin.json"))
    }

    /// Copies a zip with every `from` (local headers and central directory) replaced by the same-length `to`.
    private func swap(_ from: String, for to: String, in source: URL, to target: URL) throws {
        XCTAssertEqual(from.utf8.count, to.utf8.count)
        var bytes = try Data(contentsOf: source)
        while let r = bytes.range(of: Data(from.utf8)) { bytes.replaceSubrange(r, with: Data(to.utf8)) }
        try bytes.write(to: target)
    }

    func testImportRejectsTooManyFiles() throws {
        let s = try store()
        let top = try packageFolder(id: "user-many")
        try fm.createDirectory(at: top.appendingPathComponent("images"), withIntermediateDirectories: true)
        for i in 0..<SkinPackage.maxFiles { try Self.png.write(to: top.appendingPathComponent("images/i\(i).png")) }
        assertRejected(try zip(top), into: s, .tooLarge)
    }

    func testImportRejectsTooManyBytes() throws {
        let s = try store()
        let top = try packageFolder(id: "user-big")
        // Zeros compress to almost nothing; the cap is on what extraction would write.
        try Data(count: SkinPackage.maxBytes + 1).write(to: top.appendingPathComponent("skin.json"))
        assertRejected(try zip(top), into: s, .tooLarge)
    }

    func testImportedImagesNeedNoApproval() throws {
        let s = try store()
        let top = try packageFolder(id: "user-pic")
        try manifest("user-pic", name: "Pic", palettes: ["user-pic-dark"], asset: "Photo One.png").write(to: top.appendingPathComponent("skin.json"))
        try fm.createDirectory(at: top.appendingPathComponent("images"), withIntermediateDirectories: true)
        try Self.png.write(to: top.appendingPathComponent("images/Photo One.png"))

        let id = try SkinPackage.importPackage(at: try zip(top), into: s)
        let skin = try XCTUnwrap(s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel).skins.first)
        // The store's flat name, and the placement points at it.
        XCTAssertEqual(skin.decoration.imagery.map(\.asset), ["photo-one.png"])
        XCTAssertEqual(fm.contents(atPath: s.imageURL(asset: "photo-one.png", skinId: id).path), Self.png)
    }

    // MARK: archives that lie (fix round 1: one parser lists and reads)

    private func le32(_ b: [UInt8], _ i: Int) -> Int { Int(b[i]) | Int(b[i + 1]) << 8 | Int(b[i + 2]) << 16 | Int(b[i + 3]) << 24 }
    private func setLE32(_ b: inout [UInt8], _ i: Int, _ v: Int) { for k in 0..<4 { b[i + k] = UInt8((v >> (8 * k)) & 0xFF) } }

    /// A comment-free zip split into its local entries, central directory and end record.
    private func parts(_ url: URL) throws -> (locals: [UInt8], cd: [UInt8], eocd: [UInt8]) {
        let b = [UInt8](try Data(contentsOf: url))
        let at = b.count - 22
        XCTAssertEqual(le32(b, at), 0x06054b50, "no comment, so the end record is the last 22 bytes")
        let cd = le32(b, at + 16)
        return (Array(b[..<cd]), Array(b[cd..<at]), Array(b[at...]))
    }

    private func write(_ bytes: [UInt8], named name: String) throws -> URL {
        let url = try tempDir("crafted").appendingPathComponent(name)
        try Data(bytes).write(to: url)
        return url
    }

    func testImportRejectsLyingSizes() throws {
        let s = try store()
        let top = try packageFolder(id: "user-liar")
        let real = SkinPackage.maxBytes + 1
        try Data(count: real).write(to: top.appendingPathComponent("skin.json"))
        // Every declared size of skin.json (local header, central directory, any descriptor) says 10 bytes.
        var b = [UInt8](try Data(contentsOf: try zip(top)))
        var patched = 0
        for i in 0..<(b.count - 3) where le32(b, i) == real { setLE32(&b, i, 10); patched += 1 }
        XCTAssertGreaterThanOrEqual(patched, 2)
        let liar = try write(b, named: "liar.whimbrelskin")
        XCTAssertFalse(try zipEntries(liar).isEmpty)
        assertRejected(liar, into: s, .tooLarge)
        // The read stops at its budget, whatever the archive declares.
        XCTAssertThrowsError(try SkinPackage.readEntry("skin/skin.json", of: liar, limit: 1024 * 1024)) {
            XCTAssertEqual($0 as? SkinPackage.PackageError, .tooLarge)
        }
    }

    func testHiddenLocalEntryIsNeverImported() throws {
        let s = try store()
        let package = try parts(try zip(try packageFolder(id: "user-hide")))
        let extra = try tempDir("hidden").appendingPathComponent("skin")
        try fm.createDirectory(at: extra, withIntermediateDirectories: true)
        try json([["role": "user", "text": "planted", "date": 0]]).write(to: extra.appendingPathComponent("chat.json"))
        let hidden = try parts(try zip(extra)).locals
        // The hidden entry sits after the listed ones; only the end record's directory offset moves.
        var eocd = package.eocd
        setLE32(&eocd, 16, package.locals.count + hidden.count)
        let crafted = try write(package.locals + hidden + package.cd + eocd, named: "hidden.whimbrelskin")
        XCTAssertFalse(try zipEntries(crafted).contains("skin/chat.json"), "the listing doesn't show it")

        let id = try SkinPackage.importPackage(at: crafted, into: s)
        XCTAssertEqual(s.chat(for: id), [])
        XCTAssertFalse(fm.fileExists(atPath: try s.folder(id).appendingPathComponent("chat.json").path))
    }

    func testDualCentralDirectorySymlinkImportsNothing() throws {
        let s = try store()
        let top = try packageFolder(id: "user-dual")
        try fm.createDirectory(at: top.appendingPathComponent("images"), withIntermediateDirectories: true)
        try fm.createSymbolicLink(atPath: top.appendingPathComponent("images/x.png").path, withDestinationPath: "/etc/hosts")
        let p = try parts(try zip(top))
        // A second directory, the one the end record points at, calls the link a regular file (0120755 → 0100644).
        var regular = p.cd
        let hits = (0..<(regular.count - 1)).filter { regular[$0] == 0xED && regular[$0 + 1] == 0xA1 }
        XCTAssertEqual(hits.count, 1)
        regular[hits[0]] = 0xA4
        regular[hits[0] + 1] = 0x81
        var eocd = p.eocd
        setLE32(&eocd, 16, p.locals.count + p.cd.count)
        let crafted = try write(p.locals + p.cd + regular + eocd, named: "dual.whimbrelskin")
        XCTAssertTrue(try zipEntries(crafted).contains("skin/images/x.png"))

        // Read as a file, the "image" is the link's text, which is no image.
        assertRejected(crafted, into: s, .invalidSkin("x.png"))
    }

    func testImportRejectsOversizedPackageFile() throws {
        let s = try store()
        let file = try tempDir("huge").appendingPathComponent("huge.whimbrelskin")
        try Data(count: SkinPackage.maxBytes + 1).write(to: file)
        assertRejected(file, into: s, .tooLarge)
    }

    func testImportRejectsWildcardNames() throws {
        let s = try store()
        let top = try packageFolder(id: "user-w")
        try fm.createDirectory(at: top.appendingPathComponent("images"), withIntermediateDirectories: true)
        try Self.png.write(to: top.appendingPathComponent("images/[a].png"))
        assertRejected(try zip(top), into: s, .unsafeEntry("skin/images/[a].png"))
    }

    func testImportKeepsOnlyBuiltInPaletteRefsItDoesNotCarry() throws {
        let s = try store()
        let top = try packageFolder(id: "user-refs")
        try manifest("user-refs", name: "Refs", palettes: ["user-victim-p1", "default", "user-refs-dark"]).write(to: top.appendingPathComponent("skin.json"))
        let id = try SkinPackage.importPackage(at: try zip(top), into: s)
        XCTAssertEqual(object(s.manifest(for: id))["palettes"] as? [String], ["default", "user-refs-p1"])
    }

    func testImportAcceptsArchiveComment() throws {
        let s = try store()
        let file = try zip(try packageFolder(id: "user-note"))
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/zip")
        p.arguments = ["-q", "-z", file.path]
        let input = Pipe()
        p.standardInput = input
        try p.run()
        input.fileHandleForWriting.write(Data("made by hand\nsecond line\n".utf8))
        input.fileHandleForWriting.closeFile()
        p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0)
        XCTAssertEqual(try SkinPackage.importPackage(at: file, into: s), "user-note")
    }

    // MARK: fix round 2: encrypted entries, entry count, package path

    func testImportRejectsEncryptedEntryFast() throws {
        let s = try store()
        let top = try packageFolder(id: "user-enc")
        let file = try tempDir("enc").appendingPathComponent("enc.whimbrelskin")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/zip")
        p.arguments = ["-q", "-r", "-P", "secret", file.path, top.lastPathComponent]
        p.currentDirectoryURL = top.deletingLastPathComponent()
        try p.run()
        p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0)

        let start = Date()
        assertRejected(file, into: s, .encrypted)
        // Even read directly, unzip is given an empty password and fails instead of prompting.
        XCTAssertThrowsError(try SkinPackage.readEntry("skin/skin.json", of: file, limit: 1024)) {
            XCTAssertEqual($0 as? SkinPackage.PackageError, .unreadable)
        }
        XCTAssertLessThan(Date().timeIntervalSince(start), 5)
    }

    func testEntryCountCappedBeforeNamesAreChecked() throws {
        let s = try store()
        let top = try packageFolder(id: "user-count")
        // Not allowed paths either, so `.tooLarge` shows the count was checked first.
        try fm.createDirectory(at: top.appendingPathComponent("zzz"), withIntermediateDirectories: true)
        for i in 0..<(SkinPackage.maxFiles + 10) { try Data().write(to: top.appendingPathComponent("zzz/\(i)")) }
        assertRejected(try zip(top), into: s, .tooLarge)
    }

    func testPackagePathIsResolvedAndMustBeReadable() throws {
        let s = try store()
        let dir = try tempDir("paths")
        let huge = dir.appendingPathComponent("huge.bin")
        try Data(count: SkinPackage.maxBytes + 1).write(to: huge)
        let link = dir.appendingPathComponent("link.whimbrelskin")
        try fm.createSymbolicLink(at: link, withDestinationURL: huge)
        assertRejected(link, into: s, .tooLarge)
        assertRejected(dir.appendingPathComponent("missing.whimbrelskin"), into: s, .unreadable)
    }

    func testImportFailsPastOverallDeadline() throws {
        let s = try store()
        let file = try zip(try packageFolder(id: "user-slow"))
        XCTAssertThrowsError(try SkinPackage.importPackage(at: file, into: s, timeLimit: 0)) {
            XCTAssertEqual($0 as? SkinPackage.PackageError, .timedOut)
        }
        XCTAssertEqual(s.skinIds(), [], "nothing written")
        XCTAssertEqual(try SkinPackage.importPackage(at: file, into: s), "user-slow", "the default limit is ample")
    }

    /// I3: an imported name is capped before the clash suffix is added, so the copy's name is capped too.
    func testImportCapsLongName() throws {
        let s = try store()
        let long = String(repeating: "Harbour ", count: 10)
        let first = try SkinPackage.importPackage(at: try zip(try packageFolder(id: "harbour", name: long)), into: s)
        let name = try XCTUnwrap(object(s.manifest(for: first))["name"] as? String)
        XCTAssertTrue((UserSkinStore.nameLimit - 1...UserSkinStore.nameLimit).contains(name.count), "cut at 40 (a trailing space trimmed): \(name.count)")
        let second = try SkinPackage.importPackage(at: try zip(try packageFolder(id: "harbour", name: long)), into: s)
        let copy = try XCTUnwrap(object(s.manifest(for: second))["name"] as? String)
        XCTAssertTrue((UserSkinStore.nameLimit - 1...UserSkinStore.nameLimit).contains(copy.count), "cut at 40 (a trailing space trimmed): \(copy.count)")
        XCTAssertTrue(copy.hasSuffix(" 2"))
        XCTAssertNotEqual(copy, name)
    }

    /// M5: images no version still uses (left behind by undo, refines or a later approval) are not exported.
    func testExportShipsOnlyReferencedImages() throws {
        let s = try store()
        try makeSkin(in: s)
        _ = try s.addImage(Self.png, preferredName: "old-header.png", to: "user-sea-1")
        let file = try tempDir("out").appendingPathComponent("Sea.whimbrelskin")
        try SkinPackage.export(skinId: "user-sea-1", from: s, to: file)
        XCTAssertEqual(try zipEntries(file).filter { $0.contains("/images/") && !$0.hasSuffix("/") }, ["user-sea-1/images/wave.png"])
    }

    /// M5: an export that import would refuse is refused up front, with its own error.
    func testExportRefusesWhatImportWouldReject() throws {
        let s = try store()
        try makeSkin(in: s)
        let file = try tempDir("out").appendingPathComponent("Sea.whimbrelskin")
        XCTAssertThrowsError(try SkinPackage.export(skinId: "user-sea-1", from: s, to: file, maxBytes: 64)) { error in
            XCTAssertEqual(error as? SkinPackage.PackageError, .tooLargeToExport)
        }
        XCTAssertThrowsError(try SkinPackage.export(skinId: "user-sea-1", from: s, to: file, maxFiles: 2)) { error in
            XCTAssertEqual(error as? SkinPackage.PackageError, .tooLargeToExport)
        }
        XCTAssertFalse(fm.fileExists(atPath: file.path))
    }

    /// N1: an imported palette that fails the imported skin's archetype (here Default's secondary text on Glass's
    /// glow, 3.4:1) is repaired on the way in.
    func testImportRepairsPaletteForTheSkinsArchetype() throws {
        let top = try packageFolder(id: "glassy")
        var root = object(try Data(contentsOf: top.appendingPathComponent("skin.json")))
        root["today"] = ["archetype": "glass"]
        try json(root).write(to: top.appendingPathComponent("skin.json"))
        let before = try XCTUnwrap(ThemeLoader.parse(palette("glassy-dark")))
        XCTAssertFalse(ArchetypeContrast.passes(before.light, ArchetypeContrast.pairs(for: .glass)), "the fixture must fail Glass")

        let s = try store()
        let id = try SkinPackage.importPackage(at: try zip(top), into: s)
        let loaded = s.loadSkinsAndPalettes(base: SkinDefaults.whimbrel)
        XCTAssertEqual(loaded.skins.first?.archetype, .glass)
        let theme = try XCTUnwrap(loaded.palettes.first { $0.id == "\(id)-p1" })
        XCTAssertEqual(ArchetypeContrast.failures(theme.light, ArchetypeContrast.pairs(for: .glass)), [])
        XCTAssertEqual(ArchetypeContrast.failures(theme.dark, ArchetypeContrast.pairs(for: .glass)), [])
    }
}
