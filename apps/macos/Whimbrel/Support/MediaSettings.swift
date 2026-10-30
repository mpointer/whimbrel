import Foundation

/// The media ages handed to core's media planner (`api::media_retention::MediaPolicy`).
/// nil is the "off" choice for that step.
struct CoreMediaPolicy: Codable, Equatable {
    var compressAfterDays: Int?
    var deleteAudioAfterDays: Int?
    var shrinkImagesAfterDays: Int?
    /// The delete age the usage's "would free" figure is measured at while deletion is off.
    var deleteAudioPreviewAfterDays: Int?

    enum CodingKeys: String, CodingKey {
        case compressAfterDays = "compress_after_days"
        case deleteAudioAfterDays = "delete_audio_after_days"
        case shrinkImagesAfterDays = "shrink_images_after_days"
        case deleteAudioPreviewAfterDays = "delete_audio_preview_after_days"
    }
}

/// Storage retention: the one media folder (audio and snapshots) and the media ages.
/// Replaces the old Snapshots folder setting. Local to this Mac: none of these keys are in
/// the settings-sync field list.
@MainActor
final class MediaSettings: ObservableObject {
    private static let prefix = "com.mpointer.kestrel.media."
    private static let folderKey = prefix + "folderPath"
    private static let compressKey = prefix + "compressAfterDays"
    private static let keepFullAudioKey = prefix + "keepFullAudioForever"
    private static let deleteAudioKey = prefix + "deleteAudioAfterDays"
    private static let neverDeleteKey = prefix + "neverDeleteAudio"
    private static let shrinkKey = prefix + "shrinkImagesAfterDays"
    private static let keepFullImagesKey = prefix + "keepFullImagesForever"

    private let defaults: UserDefaults

    /// nil means the default `~/Pictures/Whimbrel`.
    @Published var folderPath: String? { didSet { defaults.set(folderPath, forKey: Self.folderKey) } }
    @Published var compressAfterDays: Int { didSet { defaults.set(compressAfterDays, forKey: Self.compressKey); storePolicy() } }
    @Published var keepFullAudioForever: Bool { didSet { defaults.set(keepFullAudioForever, forKey: Self.keepFullAudioKey); storePolicy() } }
    @Published var deleteAudioAfterDays: Int { didSet { defaults.set(deleteAudioAfterDays, forKey: Self.deleteAudioKey); storePolicy() } }
    @Published var neverDeleteAudio: Bool { didSet { defaults.set(neverDeleteAudio, forKey: Self.neverDeleteKey); storePolicy() } }
    @Published var shrinkImagesAfterDays: Int { didSet { defaults.set(shrinkImagesAfterDays, forKey: Self.shrinkKey); storePolicy() } }
    @Published var keepFullImagesForever: Bool { didSet { defaults.set(keepFullImagesForever, forKey: Self.keepFullImagesKey); storePolicy() } }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        func int(_ key: String, _ fallback: Int) -> Int { defaults.object(forKey: key) == nil ? fallback : defaults.integer(forKey: key) }
        folderPath = defaults.string(forKey: Self.folderKey)
        compressAfterDays = int(Self.compressKey, 30)
        keepFullAudioForever = defaults.bool(forKey: Self.keepFullAudioKey)
        deleteAudioAfterDays = int(Self.deleteAudioKey, 90)
        // Off until the user turns it on in Settings → Retention, new install or upgrade.
        neverDeleteAudio = defaults.object(forKey: Self.neverDeleteKey) == nil ? true : defaults.bool(forKey: Self.neverDeleteKey)
        shrinkImagesAfterDays = int(Self.shrinkKey, 30)
        keepFullImagesForever = defaults.bool(forKey: Self.keepFullImagesKey)
        storePolicy()
    }

    /// `policy` as of the last change, readable from any thread (the media run's steps
    /// re-check it on `MediaMover.queue`).
    nonisolated var currentPolicy: CoreMediaPolicy { policyLock.withLock { storedPolicy } }
    private let policyLock = NSLock()
    nonisolated(unsafe) private var storedPolicy = CoreMediaPolicy()
    private func storePolicy() {
        let p = policy
        policyLock.withLock { storedPolicy = p }
    }

    var folderURL: URL {
        if let path = folderPath, !path.isEmpty { return URL(fileURLWithPath: path, isDirectory: true) }
        return FileManager.default.urls(for: .picturesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Whimbrel", isDirectory: true)
    }
    /// This Mac's own audio folder, `Audio/<deviceId>`: the mover writes only here and the
    /// orphan sweep scans only here, so another Mac sharing the media folder can never
    /// sweep this one's recordings.
    func audioURL(deviceId: String) -> URL {
        var name = deviceId.replacingOccurrences(of: "/", with: "-").replacingOccurrences(of: ":", with: "-")
        if name.isEmpty || name == "." || name == ".." { name = "device" }
        return folderURL.appendingPathComponent("Audio", isDirectory: true).appendingPathComponent(name, isDirectory: true)
    }
    var snapshotsURL: URL { folderURL.appendingPathComponent("Snapshots", isDirectory: true) }

    var policy: CoreMediaPolicy {
        CoreMediaPolicy(
            compressAfterDays: keepFullAudioForever ? nil : compressAfterDays,
            deleteAudioAfterDays: neverDeleteAudio ? nil : deleteAudioAfterDays,
            shrinkImagesAfterDays: keepFullImagesForever ? nil : shrinkImagesAfterDays,
            deleteAudioPreviewAfterDays: deleteAudioAfterDays
        )
    }

    /// Why `folder` can't be the media folder, or nil when it can. Writes and removes a
    /// hidden probe file (the orphan sweep skips dotfiles).
    nonisolated static func refusal(for folder: URL, dataFolder: URL) -> String? {
        // Lowercased: APFS is case-insensitive, so `~/Library/…/DATA` is the data folder too.
        func components(_ url: URL) -> [String] {
            url.standardizedFileURL.resolvingSymlinksInPath().pathComponents.map { $0.lowercased() }
        }
        let f = components(folder)
        let d = components(dataFolder)
        if f.starts(with: d) { return "This folder is inside Whimbrel's data folder. Choose another." }
        let probe = folder.appendingPathComponent(".whimbrel-write-probe-\(UUID().uuidString)")
        guard FileManager.default.createFile(atPath: probe.path, contents: Data()),
              (try? FileManager.default.removeItem(at: probe)) != nil else {
            return "Whimbrel can't write to this folder."
        }
        return nil
    }
}
