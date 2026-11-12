import Foundation
import UserNotifications

/// Free space on a volume, as macOS counts it for files the user cares about (purgeable
/// space included, as Finder shows it).
enum StorageVolume {
    /// Space that is free now, without counting purgeable space macOS could clear. What a
    /// write that can't wait (compacting the database) can count on.
    static func writableFreeBytes(at url: URL) -> Int64? {
        let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityKey])
        return values?.volumeAvailableCapacity.map(Int64.init)
    }

    static func freeBytes(at url: URL) -> Int64? {
        let values = try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        return values?.volumeAvailableCapacityForImportantUsage
    }
}

/// The Storage pane's lines about compacting.
enum CompactCopy {
    static func message(_ result: CoreCompactResult?) -> String {
        func size(_ bytes: Int64) -> String { ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file) }
        guard let result else { return String(localized: "Couldn't compact the database.") }
        switch result.reason {
        case "ran" where result.spacePending == true:
            return String(localized: "Compacted. The freed space returns to the disk shortly.")
        case "ran":
            return String(localized: "Compacted: \(size(result.fileBytesBefore)) → \(size(result.fileBytesAfter)).")
        case "not_enough_disk":
            return String(localized: "Not enough free disk to compact. Free \(size(result.needBytes ?? 0)) and try again.")
        default:
            return String(localized: "Nothing to compact yet.")
        }
    }
}

/// What Whimbrel keeps on this Mac, by part (Settings → Storage).
struct StorageFootprint: Equatable {
    /// `core-memory.sqlite` with its `-wal` and `-shm`.
    var database: Int64 = 0
    /// The media folder: meeting audio and slide snapshots (every Mac's, when shared).
    var media: Int64 = 0
    /// Speech-to-text models.
    var models: Int64 = 0
    /// `screen-text.jsonl`: captures waiting to be saved to the database.
    var captureLog: Int64 = 0
    /// Everything else in the data folder.
    var other: Int64 = 0

    var total: Int64 { database + media + models + captureLog + other }

    /// Sizes on disk. Walks the folders, so call it off the main thread. A media folder inside
    /// the data folder is counted once, as media.
    static func measure(dataRoot: URL, mediaFolder: URL) -> StorageFootprint {
        let root = dataRoot.standardizedFileURL, media = mediaFolder.standardizedFileURL
        var footprint = StorageFootprint(media: size(of: media))
        let databaseNames: Set<String> = ["core-memory.sqlite", "core-memory.sqlite-wal", "core-memory.sqlite-shm"]
        let children = (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        for child in children {
            let child = child.standardizedFileURL
            if child == media { continue }  // already counted as media
            if media.path.hasPrefix(child.path + "/") {
                // A folder holding the media folder: what's beside it is other.
                footprint.other += max(0, size(of: child) - footprint.media)
                continue
            }
            let bytes = size(of: child)
            switch child.lastPathComponent {
            case let name where databaseNames.contains(name): footprint.database += bytes
            case "models": footprint.models += bytes
            case "screen-text.jsonl": footprint.captureLog += bytes
            default: footprint.other += bytes
            }
        }
        return footprint
    }

    /// The allocated size of a file, or of everything under a folder. 0 when missing.
    static func size(of url: URL) -> Int64 {
        let keys: Set<URLResourceKey> = [.totalFileAllocatedSizeKey, .isDirectoryKey]
        guard let values = try? url.resourceValues(forKeys: keys) else { return 0 }
        guard values.isDirectory == true else { return Int64(values.totalFileAllocatedSize ?? 0) }
        var total: Int64 = 0
        let walker = FileManager.default.enumerator(at: url, includingPropertiesForKeys: Array(keys), options: [], errorHandler: nil)
        while let item = walker?.nextObject() as? URL {
            total += Int64((try? item.resourceValues(forKeys: [.totalFileAllocatedSizeKey]))?.totalFileAllocatedSize ?? 0)
        }
        return total
    }

    /// A volume with less than `threshold` free: where, how much is left, and its name.
    struct LowVolume: Equatable {
        let url: URL
        let freeBytes: Int64
        let name: String
    }

    static let lowDiskThreshold: Int64 = 10_000_000_000

    /// Each distinct volume holding the data folder or the media folder whose free space is under
    /// `threshold`. `volumeID` tells two paths on one volume apart from two volumes.
    static func lowDiskVolumes(
        dataRoot: URL, mediaFolder: URL,
        free: (URL) -> Int64? = StorageVolume.freeBytes(at:),
        volumeID: (URL) -> String? = StorageVolume.identifier(of:),
        name: (URL) -> String = StorageVolume.name(of:),
        threshold: Int64 = lowDiskThreshold
    ) -> [LowVolume] {
        var seen = Set<String>(), low: [LowVolume] = []
        for url in [dataRoot, mediaFolder] {
            let id = volumeID(url) ?? url.path
            guard seen.insert(id).inserted, let bytes = free(url), bytes < threshold else { continue }
            low.append(LowVolume(url: url, freeBytes: bytes, name: name(url)))
        }
        return low
    }
}

extension StorageVolume {
    static func identifier(of url: URL) -> String? {
        guard let id = (try? url.resourceValues(forKeys: [.volumeIdentifierKey]))?.volumeIdentifier else { return nil }
        return String(describing: id)
    }

    static func name(of url: URL) -> String {
        (try? url.resourceValues(forKeys: [.volumeLocalizedNameKey]))?.volumeLocalizedName ?? url.path
    }
}

/// The once-a-day "disk almost full" notification. Tapping it opens Settings → Storage with
/// Free up space showing.
enum LowDiskNotice {
    static let category = "low-disk"
    private static let lastPostedKey = "com.mpointer.kestrel.lowDiskNoticeDay"

    /// Posts at most once per calendar day, for the lowest volume.
    static func postIfDue(_ low: [StorageFootprint.LowVolume], defaults: UserDefaults = .standard, now: Date = Date()) {
        guard let lowest = low.min(by: { $0.freeBytes < $1.freeBytes }) else { return }
        let day = ISO8601DateFormatter.string(from: now, timeZone: .current, formatOptions: [.withFullDate])
        guard defaults.string(forKey: lastPostedKey) != day else { return }
        defaults.set(day, forKey: lastPostedKey)
        let content = UNMutableNotificationContent()
        content.title = String(localized: "Your disk is almost full")
        content.body = String(localized: "Only \(ByteCountFormatter.string(fromByteCount: lowest.freeBytes, countStyle: .file)) left. Whimbrel may stop recording.")
        content.sound = .default
        content.categoryIdentifier = category
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "low-disk-\(day)", content: content, trigger: nil))
    }
}

/// Settings → Storage's lines.
enum StorageCopy {
    private static func size(_ b: Int64) -> String { ByteCountFormatter.string(fromByteCount: b, countStyle: .file) }

    static func total(_ f: StorageFootprint) -> String {
        String(localized: "Whimbrel uses \(size(f.total)) on this Mac.")
    }

    static func parts(_ f: StorageFootprint) -> String {
        String(localized: "Database \(size(f.database)) · Recordings and images \(size(f.media)) · Speech models \(size(f.models)) · Capture queue \(size(f.captureLog)) · Other \(size(f.other))")
    }

    static func lowDisk(_ low: StorageFootprint.LowVolume) -> String {
        String(localized: "Only \(size(low.freeBytes)) free on \(low.name). Whimbrel needs room to record. Free some space or move the media folder.")
    }
}

/// Runs a database compaction. It holds both store lanes for the whole rewrite, so the slow
/// lane's connection waits instead of failing with "database is locked" partway through.
/// Lock order is slow lane, then access queue; nothing takes them the other way round.
/// Never call it from the main thread or from either queue.
enum StorageCompaction {
    static func run(_ store: CoreMemoryStore, forced: Bool) -> CoreCompactResult? {
        CoreMemoryStore.slowLaneQueue.sync {
            CoreMemoryStore.accessQueue.sync {
                store.compact(volumeFreeBytes: StorageVolume.writableFreeBytes(at: SpikePaths.root) ?? 0, forced: forced)
            }
        }
    }
}
