import AppKit
import OSLog
import SwiftUI

/// Renders and caches the gallery's preview images: `TodayChrome` with the sample content under a
/// skin and palette override, at 900x1200 scaled to a quarter. An entry is reused only while the
/// installed skin and palette are unchanged, so installing a changed skin or theme re-renders it.
/// The stores only say what is installed; the render itself runs under the shared stores' overrides,
/// since that is what `TodayChrome` and the colors read.
@MainActor
final class SkinPreviewCache {
    static let shared = SkinPreviewCache()

    static let renderSize = CGSize(width: 900, height: 1200)
    static let scale: CGFloat = 0.25

    private struct Key: Hashable {
        let skinId: String
        let paletteId: String
        let dark: Bool
    }

    private struct Entry {
        let skin: Skin
        let theme: Theme
        let image: NSImage
    }

    private let skins: SkinStore
    private let themes: ThemeStore
    private var entries: [Key: Entry] = [:]
    /// How many previews have been drawn (not served from the cache).
    private(set) var renderCount = 0
    /// True while a preview is being drawn; no other render starts then.
    private(set) var isRendering = false
    /// Called at the start of each render (tests use it to probe re-entrancy).
    var onRender: (() -> Void)?

    init(skins: SkinStore = .shared, themes: ThemeStore = .shared) {
        self.skins = skins
        self.themes = themes
    }

    private func resolve(_ skinId: String, _ paletteId: String, _ colorScheme: ColorScheme) -> (key: Key, skin: Skin, theme: Theme) {
        let skin = skins.skins.first { $0.id == skinId } ?? SkinDefaults.whimbrel
        let theme = themes.themes.first { $0.id == paletteId } ?? ThemeDefaults.theme
        return (Key(skinId: skin.id, paletteId: theme.id, dark: colorScheme == .dark), skin, theme)
    }

    /// The cached preview, when it is still current.
    func cachedImage(skinId: String, paletteId: String, colorScheme: ColorScheme) -> NSImage? {
        let r = resolve(skinId, paletteId, colorScheme)
        guard let entry = entries[r.key], entry.skin == r.skin, entry.theme == r.theme else { return nil }
        return entry.image
    }

    /// The preview, drawing it now if needed; nil when another render is in progress (retry later).
    func imageIfIdle(skinId: String, paletteId: String, colorScheme: ColorScheme) -> NSImage? {
        if let cached = cachedImage(skinId: skinId, paletteId: paletteId, colorScheme: colorScheme) { return cached }
        guard !isRendering else { return nil }
        let r = resolve(skinId, paletteId, colorScheme)
        isRendering = true
        defer { isRendering = false }
        onRender?()
        // ponytail: override swaps the global store during a synchronous main-thread render; per-view environment injection if previews ever render off-main.
        let image = SkinStore.shared.withOverride(skin: r.skin, paletteId: r.theme.id) { Self.render(colorScheme: colorScheme) }
        renderCount += 1
        entries[r.key] = Entry(skin: r.skin, theme: r.theme, image: image)
        return image
    }

    /// Like `imageIfIdle`, but a blank placeholder (never cached) while another render is in progress.
    func image(skinId: String, paletteId: String, colorScheme: ColorScheme) -> NSImage {
        imageIfIdle(skinId: skinId, paletteId: paletteId, colorScheme: colorScheme)
            ?? NSImage(size: CGSize(width: Self.renderSize.width * Self.scale, height: Self.renderSize.height * Self.scale))
    }

    /// The scroll view in `TodayChrome` is why this goes through `OffscreenRender`, not `ImageRenderer`.
    private static func render(colorScheme: ColorScheme) -> NSImage {
        let view = TodayChrome(
            content: TodaySampleContent.sample, actions: .noop,
            slots: TodaySlots(live: nil, decisionCount: TodaySampleContent.sample.pendingApprovalCount, actions: .noop, composer: TodaySampleContent.composer)
        )
        .frame(width: renderSize.width, height: renderSize.height, alignment: .topLeading)
        .background(Color.whimbrelBackgroundPage)
        let target = CGSize(width: renderSize.width * scale, height: renderSize.height * scale)
        let image = NSImage(size: target)
        guard let rep = OffscreenRender.bitmap(of: view, size: renderSize, colorScheme: colorScheme, scale: 1, settle: false) else { return image }
        image.lockFocus()
        NSGraphicsContext.current?.imageInterpolation = .high
        rep.draw(in: CGRect(origin: .zero, size: target))
        image.unlockFocus()
        return image
    }
}

/// Five dots in the theme's own colors for the given appearance.
struct ThemeSwatches: View {
    let theme: Theme
    let colorScheme: ColorScheme

    var body: some View {
        let palette = (colorScheme == .dark) ? theme.dark : theme.light
        HStack(spacing: 2) {
            ForEach([ThemeToken.backgroundPage, .backgroundCard, .accentPrimary, .accentSecondary, .textPrimary], id: \.self) { token in
                Circle()
                    .fill(Color(NSColor(srgbHex: palette[token] ?? 0)))
                    .frame(width: 14, height: 14)
                    .overlay(Circle().stroke(Color.whimbrelBorder, lineWidth: 0.5))
            }
        }
    }
}

/// Settings → Appearance: a card per installed skin with a live preview; the selected one offers
/// its palettes. Your skins follow the built-ins, marked as yours, with Rename, Delete and Export;
/// a `.whimbrelskin` file imports as another of yours.
struct SkinGallery: View {
    /// Import waits while a turn runs, since it reinstalls the skins.
    @ObservedObject var skinChat: SkinChatSession
    @State private var importFailure: String?

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")
    @ObservedObject private var skins = SkinStore.shared
    @ObservedObject private var themes = ThemeStore.shared
    @Environment(\.colorScheme) private var colorScheme

    /// Clicking a card picks the skin's first palette, except on the skin already selected (it keeps its current one).
    static func paletteToSelect(isSelected: Bool, current: String, first: String) -> String { isSelected ? current : first }

    /// The selected skin previews with the palette in force, the others with their first.
    static func previewPaletteId(for skin: Skin, skins: SkinStore) -> String {
        skins.selected.id == skin.id ? skins.selectedPaletteId : skins.palettes(for: skin)[0].id
    }

    /// A skin you made (they live in `UserSkinStore`, whose ids all start `user-`).
    static func isUserSkin(_ skin: Skin) -> Bool { skin.id.hasPrefix("user-") }

    /// Built-ins in installed order, then your skins by name.
    static func orderedCards(skins: [Skin]) -> [Skin] {
        skins.filter { !isUserSkin($0) } + skins.filter(isUserSkin).sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    /// The card's preview when already drawn, so a rebuilt card shows it at once instead of a placeholder.
    static func cachedPreview(for skin: Skin, skins: SkinStore, colorScheme: ColorScheme, cache: SkinPreviewCache = .shared) -> NSImage? {
        cache.cachedImage(skinId: skin.id, paletteId: previewPaletteId(for: skin, skins: skins), colorScheme: colorScheme)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 190), spacing: Spacing.md, alignment: .top)], alignment: .leading, spacing: Spacing.md) {
                ForEach(Self.orderedCards(skins: skins.skins)) { skin in
                    SkinCard(skin: skin, skins: skins, skinChat: skinChat, colorScheme: colorScheme)
                }
            }
            HStack(spacing: Spacing.sm) {
                Text("Each Mac keeps its own skin.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Spacer()
                Button("Import Skin…", action: importSkin)
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(skinChat.isRunning)
            }
            if let importFailure {
                Text(importFailure).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
    }

    /// Adds the chosen file as another of your skins and scrolls to its card; the selection stays.
    private func importSkin() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [SkinPackage.utType]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        importFailure = nil
        do {
            let id = try SkinPackage.importPackage(at: url, into: .shared)
            skins.scrollSettingsBack(to: SkinStore.galleryCardId(id))
            skins.reloadUserSkins()
        } catch {
            Self.log.error("skin import failed: \(String(describing: error), privacy: .public)")
            importFailure = "Whimbrel couldn't import that file. It needs to be a skin exported from Whimbrel."
        }
    }
}

private struct SkinCard: View {
    let skin: Skin
    @ObservedObject var skins: SkinStore
    /// Rename, Delete and Export wait while a turn runs, since it would write back to the skin it started for.
    @ObservedObject var skinChat: SkinChatSession
    let colorScheme: ColorScheme
    @State private var failure: String?
    @State private var preview: NSImage?
    /// The name being typed while renaming; nil when not renaming.
    @State private var newName: String?
    @State private var confirmingDelete = false

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")

    private var isSelected: Bool { skins.selected.id == skin.id }
    private var palettes: [Theme] { skins.palettes(for: skin) }
    private var previewPaletteId: String { SkinGallery.previewPaletteId(for: skin, skins: skins) }

    private struct PreviewKey: Equatable {
        let paletteId: String
        let dark: Bool
        let revision: Int
        let themeRevision: Int
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Button { skins.selectFromGallery(skinId: skin.id, paletteId: SkinGallery.paletteToSelect(isSelected: isSelected, current: skins.selectedPaletteId, first: palettes[0].id)) } label: {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    previewImage
                    Text(skin.name).font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                        .lineLimit(2).truncationMode(.tail).fixedSize(horizontal: false, vertical: true)
                    Text(skin.voice.text(.tagline, name: ""))
                        .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                        .lineLimit(2).fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(skin.name)
            .accessibilityHint(skin.voice.text(.tagline, name: ""))
            .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
            .accessibilityIdentifier(SkinStore.galleryCardId(skin.id))

            if SkinGallery.isUserSkin(skin) { yoursRow }

            if isSelected {
                HStack(spacing: Spacing.sm) {
                    ForEach(palettes) { palette in
                        let current = palette.id == skins.selectedPaletteId
                        Button { skins.selectFromGallery(skinId: skin.id, paletteId: palette.id) } label: {
                            ThemeSwatches(theme: palette, colorScheme: colorScheme)
                                .padding(4)
                                .overlay(RoundedRectangle(cornerRadius: 6).stroke(current ? Color.whimbrelAccentPrimary : Color.clear, lineWidth: 2))
                        }
                        .buttonStyle(.plain)
                        .help(palette.name)
                        .accessibilityLabel(palette.name)
                        .accessibilityAddTraits(current ? [.isButton, .isSelected] : .isButton)
                        .accessibilityIdentifier("skin-palette-\(palette.id)")
                    }
                }
            }
        }
        .padding(Spacing.sm)
        .background(Color.whimbrelBackgroundCard)
        .clipShape(RoundedRectangle(cornerRadius: Radius.card))
        .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(isSelected ? Color.whimbrelAccentPrimary : Color.whimbrelBorder, lineWidth: isSelected ? 2 : 1))
        .id(SkinStore.galleryCardId(skin.id))
        .task(id: PreviewKey(paletteId: previewPaletteId, dark: colorScheme == .dark, revision: skins.revision, themeRevision: ThemeStore.shared.revision)) {
            // Yield first so placeholders paint, and retry while another card's render is in progress.
            while !Task.isCancelled {
                await Task.yield()
                if let image = SkinPreviewCache.shared.imageIfIdle(skinId: skin.id, paletteId: previewPaletteId, colorScheme: colorScheme) {
                    preview = image
                    return
                }
                try? await Task.sleep(for: .milliseconds(50))
            }
        }
    }

    /// The "Yours" mark and the card's menu; renaming and the delete confirmation happen right here.
    @ViewBuilder
    private var yoursRow: some View {
        if let name = newName {
            HStack(spacing: Spacing.xs) {
                TextField("Name", text: Binding(get: { name }, set: { newName = $0 }))
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(rename)
                Button("Save", action: rename)
                    .buttonStyle(.whimbrelPrimary)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                Button("Cancel") { newName = nil }
                    .buttonStyle(.whimbrelSecondary)
            }
        } else if confirmingDelete {
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text("Delete “\(skin.name)”? Its chat and earlier versions go too.")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: Spacing.xs) {
                    Button("Delete", action: delete)
                        .buttonStyle(.whimbrelPrimary)
                    Button("Cancel") { confirmingDelete = false }
                        .buttonStyle(.whimbrelSecondary)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Text("Yours")
                    .font(.whimbrelBody(.caption2).bold())
                    .foregroundStyle(Color.whimbrelAccentPrimary)
                    .padding(.horizontal, Spacing.xs)
                    .overlay(Capsule().stroke(Color.whimbrelAccentPrimary, lineWidth: 1))
                Spacer()
                Menu {
                    Button("Rename…") { failure = nil; newName = skin.name }
                        .disabled(skinChat.isRunning)
                    Button("Export…", action: export)
                        .disabled(skinChat.isRunning)
                    Button("Delete…", role: .destructive) { failure = nil; confirmingDelete = true }
                        .disabled(skinChat.isRunning)
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .accessibilityLabel("\(skin.name) options")
            }
            if let failure {
                Text(failure).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            }
        }
    }

    private func rename() {
        guard let name = newName?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty else { return }
        do {
            try UserSkinStore.shared.rename(skinId: skin.id, to: name)
        } catch {
            Self.log.error("skin rename failed: \(String(describing: error), privacy: .public)")
            failure = "Couldn't rename this skin."
        }
        newName = nil
        skins.scrollSettingsBack(to: SkinStore.galleryCardId(skin.id))
        skins.reloadUserSkins()
    }

    private func export() {
        failure = nil
        let panel = NSSavePanel()
        panel.allowedContentTypes = [SkinPackage.utType]
        // The id, not the name: it is always a valid file name (`a-z0-9-`), whatever the name holds.
        panel.nameFieldStringValue = skin.id
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try SkinPackage.export(skinId: skin.id, from: .shared, to: url)
        } catch SkinPackage.PackageError.tooLargeToExport {
            failure = "This skin is too big to export (more than 64 MB or 200 files), so it couldn't be imported again."
        } catch {
            Self.log.error("skin export failed: \(String(describing: error), privacy: .public)")
            failure = "Couldn't export this skin."
        }
    }

    private func delete() {
        do {
            try UserSkinStore.shared.delete(skinId: skin.id)
        } catch {
            Self.log.error("skin delete failed: \(String(describing: error), privacy: .public)")
            failure = "Couldn't delete this skin."
        }
        confirmingDelete = false
        skins.scrollSettingsBack(to: SkinChatView.scrollId)
        skins.reloadUserSkins()
    }

    private var previewImage: some View {
        Group {
            if let preview = preview ?? SkinGallery.cachedPreview(for: skin, skins: skins, colorScheme: colorScheme) {
                Image(nsImage: preview).resizable().interpolation(.high).scaledToFit()
            } else {
                Color.whimbrelBackgroundPage
            }
        }
        .aspectRatio(SkinPreviewCache.renderSize.width / SkinPreviewCache.renderSize.height, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.whimbrelBorder, lineWidth: 0.5))
        .accessibilityHidden(true)
    }
}
