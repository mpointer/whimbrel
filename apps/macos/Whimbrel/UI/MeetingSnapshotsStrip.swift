import SwiftUI
import AppKit
import ImageIO

/// Backs the snapshots strip. Store calls are synchronous, so they run on
/// `CoreMemoryStore.accessQueue` (the #353 pattern), never on the main thread.
@MainActor
final class MeetingSnapshotsModel: ObservableObject {
    @Published var snapshots: [CoreMeetingSnapshot] = []
    @Published var keep = false
    /// By snapshot id, with the file it was decoded from: a build replaces a slide's file.
    @Published private(set) var thumbnails: [Int64: (path: String, image: NSImage)] = [:]
    var folderURL: URL?

    private let meetingId: Int64
    /// Settable so the owning view can attach the app's store on appear.
    var store: CoreMemoryStore?

    init(meetingId: Int64, store: CoreMemoryStore?) {
        self.meetingId = meetingId
        self.store = store
    }

    private nonisolated static func onAccessQueue<T>(_ work: @escaping () -> T) async -> T {
        await withCheckedContinuation { continuation in
            CoreMemoryStore.accessQueue.async { continuation.resume(returning: work()) }
        }
    }

    static func folderURL(fromInfoDir dir: String?) -> URL? {
        guard let dir, !dir.isEmpty else { return nil }
        return URL(fileURLWithPath: dir, isDirectory: true)
    }

    func reload() async {
        guard let store else { return }
        let id = meetingId
        let (list, info) = await Self.onAccessQueue {
            (store.listMeetingSnapshots(meetingId: id), store.meetingSnapshotsInfo(meetingId: id))
        }
        snapshots = list
        keep = info?.keep ?? false
        folderURL = Self.folderURL(fromInfoDir: info?.dir)
        let missing = list.filter { thumbnails[$0.id]?.path != $0.filePath }
        for snap in missing {
            if let image = await SnapshotThumbnails.image(for: snap.filePath) {
                thumbnails[snap.id] = (snap.filePath, image)
            }
        }
    }

    func setKeep(_ value: Bool) async {
        guard let store else { return }
        let id = meetingId
        keep = value
        await Self.onAccessQueue { store.setKeepSnapshots(meetingId: id, keep: value) }
    }

    func delete(_ snapshot: CoreMeetingSnapshot) async {
        guard let store else { return }
        let snapId = snapshot.id
        // Behind `MediaMover.queue`: the meeting's snapshots may be mid-move or mid-shrink.
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            MediaMover.queue.async {
                if let path = CoreMemoryStore.accessQueue.sync(execute: { store.deleteMeetingSnapshot(id: snapId) }) {
                    try? FileManager.default.removeItem(atPath: path)
                }
                c.resume()
            }
        }
        snapshots.removeAll { $0.id == snapId }
        thumbnails[snapId] = nil
    }
}

/// Downsampled snapshot thumbnails, decoded off the main thread and cached by file path.
/// Shared by the snapshots strip and Timeline's snapshot rows.
@MainActor
enum SnapshotThumbnails {
    private static let cache = NSCache<NSString, NSImage>()

    /// nil when the file is missing or unreadable (not cached, so a restored file shows up).
    /// Keyed by path and modification time: a replaced build can reuse an earlier file name.
    static func image(for path: String) async -> NSImage? {
        let modified = (try? FileManager.default.attributesOfItem(atPath: path)[.modificationDate] as? Date)?
            .timeIntervalSince1970 ?? 0
        let key = "\(path)#\(modified)" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        let url = URL(fileURLWithPath: path)
        guard let image = await Task.detached(priority: .utility, operation: { decode(url) }).value else { return nil }
        cache.setObject(image, forKey: key)
        return image
    }

    private nonisolated static func decode(_ url: URL) -> NSImage? {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 480,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
    }
}

struct MeetingSnapshotsStrip: View {
    @ObservedObject var model: MeetingSnapshotsModel

    var body: some View {
        if !model.snapshots.isEmpty {
            VStack(alignment: .leading, spacing: Spacing.sm) {
                HStack(spacing: Spacing.md) {
                    Text("Slides (\(model.snapshots.count))")
                        .font(.whimbrelBody(.callout).bold())
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    Spacer()
                    Toggle("Keep snapshots", isOn: Binding(
                        get: { model.keep },
                        set: { v in Task { await model.setKeep(v) } }
                    ))
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .tint(Color.whimbrelAccentPrimary)
                    Button("Show in Finder") { showInFinder(nil) }
                        .buttonStyle(.whimbrelSecondary)
                        .controlSize(.small)
                }
                ScrollView(.horizontal, showsIndicators: true) {
                    HStack(spacing: Spacing.sm) {
                        ForEach(model.snapshots) { snap in thumbnail(snap) }
                    }
                }
            }
        }
    }

    private func thumbnail(_ snap: CoreMeetingSnapshot) -> some View {
        let url = URL(fileURLWithPath: snap.filePath)
        return Group {
            if let image = model.thumbnails[snap.id]?.image {
                Image(nsImage: image).resizable().scaledToFit()
            } else {
                Rectangle().fill(Color.whimbrelBorder).aspectRatio(16.0 / 9.0, contentMode: .fit)
            }
        }
        .frame(height: 120)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder))
        .onTapGesture { NSWorkspace.shared.open(url) }
        .onDrag { NSItemProvider(contentsOf: url) ?? NSItemProvider() }
        .contextMenu {
            Button("Copy Image") {
                if let image = NSImage(contentsOf: url) {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.writeObjects([image])
                }
            }
            Button("Show in Finder") { showInFinder(url) }
            Button("Delete", role: .destructive) { Task { await model.delete(snap) } }
        }
        .accessibilityLabel("Slide \(snap.id)")
        .accessibilityAddTraits(.isButton)
    }

    private func showInFinder(_ file: URL?) {
        if let file {
            NSWorkspace.shared.activateFileViewerSelecting([file])
        } else if let folder = model.folderURL {
            NSWorkspace.shared.open(folder)
        } else if let first = model.snapshots.first {
            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: first.filePath)])
        }
    }
}
