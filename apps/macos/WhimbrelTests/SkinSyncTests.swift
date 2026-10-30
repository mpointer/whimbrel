import AppKit
import SQLite3
import XCTest
@testable import Whimbrel

/// Skins Task 16: the Mac sends and receives user skins over sync. A pulled skin is put in the store's
/// `synced_skin` table directly (what a content sync round does), then the real reconcile writes it.
@MainActor
final class SkinSyncTests: XCTestCase {
    private var root: URL!
    private var dbPath: URL!
    private var users: UserSkinStore!
    private var skins: SkinStore!
    private var defaults: UserDefaults!
    private var core: CoreMemoryStore!

    override func setUp() async throws {
        let suite = "skin-sync-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("skin-sync-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock {
            d.removePersistentDomain(forName: suite)
            try? FileManager.default.removeItem(at: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/\(suite).plist"))
            try? FileManager.default.removeItem(at: dir)
        }
        defaults = d
        root = dir.appendingPathComponent("skins")
        dbPath = dir.appendingPathComponent("store.sqlite")
        users = UserSkinStore(root: root)
        skins = SkinStore(defaults: d, themes: ThemeStore(defaults: d), userSkins: users)
        skins.installBundledAndUserSkins()
        core = try XCTUnwrap(CoreMemoryStore(path: dbPath, modelsDir: nil))
        let store = core!
        CoreMemoryStore.accessQueue.sync { _ = store.setDeviceId("mac") }
    }

    // MARK: helpers

    private func manifest(_ id: String, name: String, palettes: [String] = []) -> [String: Any] {
        ["id": id, "name": name, "palettes": palettes]
    }

    private func palette(_ id: String) -> [String: Any] {
        ["id": id, "name": "Sand", "light": ["backgroundPage": "#fdf6e3", "textPrimary": "#222222", "textSecondary": "#444444"]]
    }

    private func save(_ id: String, name: String, palettes: [String] = []) throws {
        let data = try JSONSerialization.data(withJSONObject: manifest(id, name: name, palettes: palettes), options: [.sortedKeys])
        let files = try palettes.map { (id: $0, data: try JSONSerialization.data(withJSONObject: palette($0))) }
        _ = try users.save(manifest: data, palettes: files, for: id)
    }

    private func sqlite<T>(_ body: (OpaquePointer) throws -> T) throws -> T {
        var db: OpaquePointer?
        guard sqlite3_open(dbPath.path, &db) == SQLITE_OK, let db else { throw NSError(domain: "sqlite", code: 1) }
        defer { sqlite3_close(db) }
        return try body(db)
    }

    /// The `skin` rows in the outbox.
    private func skinRowCount() throws -> Int {
        CoreMemoryStore.accessQueue.sync {}  // the enqueue runs there
        return try sqlite { db in
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'skin'", -1, &stmt, nil) == SQLITE_OK else {
                throw NSError(domain: "sqlite", code: 2)
            }
            defer { sqlite3_finalize(stmt) }
            sqlite3_step(stmt)
            return Int(sqlite3_column_int64(stmt, 0))
        }
    }

    /// What a content sync round leaves when a peer sent this skin: a row in `synced_skin`.
    private func pulled(_ id: String, manifest: [String: Any]?, palettes: [[String: Any]] = [], at: Int64 = 4_000_000_000_000, deleted: Bool = false) throws {
        let payload: [String: Any] = [
            "skin_id": id, "name": manifest?["name"] ?? "", "manifest": manifest ?? NSNull(),
            "palettes": palettes.map { [$0["id"]!, $0] }, "images": [] as [Any],
            "updated_at_unix_ms": at, "device_id": "pc", "deleted": deleted,
        ]
        let json = String(decoding: try JSONSerialization.data(withJSONObject: payload), as: UTF8.self)
        try sqlite { db in
            var stmt: OpaquePointer?
            let sql = "INSERT INTO synced_skin (skin_id, payload_json, updated_at_unix_ms, device_id, deleted) VALUES (?1, ?2, ?3, 'pc', ?4) "
                + "ON CONFLICT (skin_id) DO UPDATE SET payload_json = ?2, updated_at_unix_ms = ?3, device_id = 'pc', deleted = ?4"
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { throw NSError(domain: "sqlite", code: 3) }
            defer { sqlite3_finalize(stmt) }
            let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
            sqlite3_bind_text(stmt, 1, id, -1, transient)
            sqlite3_bind_text(stmt, 2, json, -1, transient)
            sqlite3_bind_int64(stmt, 3, at)
            sqlite3_bind_int(stmt, 4, deleted ? 1 : 0)
            guard sqlite3_step(stmt) == SQLITE_DONE else { throw NSError(domain: "sqlite", code: 4) }
        }
    }

    private func round() async {
        await SkinSync.round(store: core, skins: users, skinStore: skins, defaults: defaults)
    }

    private func name(_ id: String) -> String? {
        users.manifest(for: id).flatMap { (try? JSONSerialization.jsonObject(with: $0)) as? [String: Any] }?["name"] as? String
    }

    /// A PNG `width` x `height`, its left half black and its right half white.
    private func png(_ width: Int, _ height: Int) -> Data {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8, samplesPerPixel: 4,
                                   hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        NSColor.black.setFill()
        NSRect(x: 0, y: 0, width: width / 2, height: height).fill()
        NSColor.white.setFill()
        NSRect(x: width / 2, y: 0, width: width - width / 2, height: height).fill()
        NSGraphicsContext.restoreGraphicsState()
        return rep.representation(using: .png, properties: [:])!
    }

    /// "WxH" of an encoded image.
    private func pixelSize(_ data: Data) -> String? {
        NSBitmapImageRep(data: data).map { "\($0.pixelsWide)x\($0.pixelsHigh)" }
    }

    // MARK: tests

    func test_save_enqueues() throws {
        SkinSync.connect(store: core, skins: users)
        try save("user-a-1", name: "A")
        XCTAssertEqual(try skinRowCount(), 1, "a save sends the skin")
        try users.rename(skinId: "user-a-1", to: "B")
        XCTAssertEqual(try skinRowCount(), 2, "a rename sends it once")
        XCTAssertTrue(try users.undo(skinId: "user-a-1"))
        XCTAssertEqual(try skinRowCount(), 3, "an undo sends it")
        try users.delete(skinId: "user-a-1")
        XCTAssertEqual(try skinRowCount(), 4, "a delete sends the deletion")
        _ = try users.addImage(png(4, 4), preferredName: "x.png", to: "user-b-1")
        XCTAssertEqual(try skinRowCount(), 4, "an image alone is not a change; the save that places it is")
    }

    func test_delete_moves_to_deleted_folder() throws {
        try save("user-a-1", name: "A")
        let before = Date()
        try users.delete(skinId: "user-a-1")
        let moved = root.appendingPathComponent(".deleted/user-a-1")
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("user-a-1").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: moved.appendingPathComponent("skin.json").path))
        XCTAssertEqual(users.skinIds(), [])
        // Core's marker: milliseconds since 1970, as text.
        let stamp = try XCTUnwrap(Int64(String(decoding: try Data(contentsOf: moved.appendingPathComponent(".deleted-at")), as: UTF8.self)))
        XCTAssertGreaterThanOrEqual(stamp, Int64(before.timeIntervalSince1970 * 1000) - 1000)
        // Kept for 30 days, then purged at launch.
        XCTAssertEqual(users.purgeDeleted(olderThan: SkinSync.deletedKept), 0)
        let old = Int64(Date().addingTimeInterval(-31 * 24 * 3600).timeIntervalSince1970 * 1000)
        try Data("\(old)".utf8).write(to: moved.appendingPathComponent(".deleted-at"))
        XCTAssertEqual(users.purgeDeleted(olderThan: SkinSync.deletedKept), 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: moved.path))
    }

    func test_reconcile_reloads_and_selects_fallback() async throws {
        try save("user-a-1", name: "Mine", palettes: ["user-a-1-p1", "user-a-1-p2"])
        skins.reloadUserSkins()
        skins.select(skinId: "user-a-1", paletteId: "user-a-1-p2")
        XCTAssertEqual(skins.selectedPaletteId, "user-a-1-p2")
        // The PC renamed it and dropped the palette in use, and made a skin of its own.
        try pulled("user-a-1", manifest: manifest("user-a-1", name: "Renamed on the PC", palettes: ["user-a-1-p1"]), palettes: [palette("user-a-1-p1")])
        try pulled("user-pc-1", manifest: manifest("user-pc-1", name: "From the PC"))
        await round()
        XCTAssertEqual(skins.selected.id, "user-a-1")
        XCTAssertEqual(skins.selected.name, "Renamed on the PC", "the skins were reloaded")
        XCTAssertEqual(skins.selectedPaletteId, "user-a-1-p1", "the palette in use went, so the skin's first is selected")
        XCTAssertTrue(skins.skins.contains { $0.id == "user-pc-1" && $0.name == "From the PC" })
    }

    func test_remote_delete_of_selected_falls_back() async throws {
        try save("user-a-1", name: "Mine")
        skins.reloadUserSkins()
        skins.select(skinId: "user-a-1", paletteId: nil)
        try pulled("user-a-1", manifest: nil, deleted: true)
        await round()
        XCTAssertEqual(skins.selected.id, SkinDefaults.whimbrel.id)
        XCTAssertEqual(skins.selectedPaletteId, ThemeDefaults.theme.id)
        XCTAssertFalse(skins.skins.contains { $0.id == "user-a-1" })
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent(".deleted/user-a-1/skin.json").path), "kept for 30 days")
    }

    func test_rust_written_history_undoes() async throws {
        try save("user-a-1", name: "Mine")
        try pulled("user-a-1", manifest: manifest("user-a-1", name: "Theirs"))
        await round()
        XCTAssertEqual(name("user-a-1"), "Theirs")
        try users.rename(skinId: "user-a-1", to: "Later")
        // Undo reads the version core wrote, then the one written here before it.
        XCTAssertTrue(try users.undo(skinId: "user-a-1"))
        XCTAssertEqual(name("user-a-1"), "Theirs")
        XCTAssertTrue(try users.undo(skinId: "user-a-1"))
        XCTAssertEqual(name("user-a-1"), "Mine", "the version that lost to the PC's is in Undo")
    }

    func test_migration_downscales_once_and_updates_scrim() async throws {
        try save("user-a-1", name: "Pictured")
        let asset = try users.addImage(png(2400, 1200), preferredName: "big.png", to: "user-a-1")
        var object = manifest("user-a-1", name: "Pictured")
        object["decoration"] = ["imagery": [["asset": asset, "placement": "header", "textFree": false]]]
        _ = try users.save(manifest: try JSONSerialization.data(withJSONObject: object), palettes: [], for: "user-a-1")
        skins.reloadUserSkins()
        let previous = SkinImageResolver.userSkins
        SkinImageResolver.userSkins = users
        defer { SkinImageResolver.userSkins = previous }
        let skin = try XCTUnwrap(skins.skins.first { $0.id == "user-a-1" })
        let image = try XCTUnwrap(skin.decoration.imagery.first)
        let before = try XCTUnwrap(SkinImageResolver.image(for: image, skin: skin))
        XCTAssertEqual(before.representations.first?.pixelsWide, 2400)
        _ = SkinImageResolver.scrimOpacity(for: before, text: [0x222222], scrim: 0xFFFFFF)

        await round()
        let url = users.imageURL(asset: asset, skinId: "user-a-1")
        XCTAssertEqual(pixelSize(try Data(contentsOf: url)), "1600x800")
        // The drawn image (and so its scrim) is the downscaled file, not the one decoded before.
        let after = try XCTUnwrap(SkinImageResolver.image(for: image, skin: skin))
        XCTAssertFalse(after === before)
        XCTAssertEqual(after.representations.first?.pixelsWide, 1600)
        XCTAssertEqual(SkinImageResolver.scrimOpacity(for: after, text: [0x222222], scrim: 0xFFFFFF),
                       SkinImageryView.scrimOpacity(text: [0x222222], scrim: 0xFFFFFF,
                                                    lightest: SkinImageryView.extremes(of: after, page: 0xFFFFFF).0,
                                                    darkest: SkinImageryView.extremes(of: after, page: 0xFFFFFF).1, glow: nil))

        // Once only: a big file written after the migration is left alone by later rounds.
        try png(2400, 1200).write(to: url)
        await round()
        XCTAssertEqual(pixelSize(try Data(contentsOf: url)), "2400x1200")
    }

    /// A file the migration couldn't rewrite leaves it to run again, and the next pass finishes it.
    func test_migration_retries_after_a_failed_write() throws {
        try save("user-a-1", name: "Pictured")
        let asset = try users.addImage(png(2400, 1200), preferredName: "big.png", to: "user-a-1")
        let images = root.appendingPathComponent("user-a-1/images")
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: images.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: images.path) }
        XCTAssertFalse(SkinImageMigration.runIfNeeded(skins: users, defaults: defaults))
        XCTAssertFalse(defaults.bool(forKey: SkinImageMigration.doneKey), "a failed write leaves the migration to run again")

        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: images.path)
        XCTAssertTrue(SkinImageMigration.runIfNeeded(skins: users, defaults: defaults))
        XCTAssertTrue(defaults.bool(forKey: SkinImageMigration.doneKey))
        XCTAssertEqual(pixelSize(try Data(contentsOf: users.imageURL(asset: asset, skinId: "user-a-1"))), "1600x800")
    }
}
