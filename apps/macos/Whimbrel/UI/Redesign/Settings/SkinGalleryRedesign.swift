import AppKit
import OSLog
import SwiftUI
import UniformTypeIdentifiers
import WhimbrelUI

/// The skin gallery of the redesigned General section: the model for the package's `WSkinGalleryView` and the
/// store calls behind its cards. The calls are the classic gallery's own (`SkinGallery`, `SkinCard`): the same
/// `SkinStore`, `UserSkinStore` and `SkinPackage` functions, the same messages and the same scroll-back.
enum SkinGalleryAdapter {
    /// The cards, in the classic gallery's order. Only the selected skin shows its palettes.
    static func model(
        skins: [Skin], redesign: Bool, selectedSkinId: String, selectedPaletteId: String,
        palettes: (Skin) -> [Theme], dark: Bool, isBusy: Bool
    ) -> WSkinGalleryModel {
        let cards = SkinGallery.orderedCards(skins: skins, redesign: redesign).map { skin -> WSkinGalleryModel.Card in
            let selected = skin.id == selectedSkinId
            return WSkinGalleryModel.Card(
                id: skin.id, anchorID: SkinStore.galleryCardId(skin.id), name: skin.name,
                tagline: skin.voice.text(.tagline, name: ""), isSelected: selected, isYours: SkinGallery.isUserSkin(skin),
                palettes: selected ? palettes(skin).map { paletteDot($0, dark: dark) } : [],
                selectedPalette: selected ? selectedPaletteId : nil)
        }
        return WSkinGalleryModel(cards: cards, isBusy: isBusy, labels: labels())
    }

    /// Two colours of the palette in the current appearance: the accent, then the page.
    static func paletteDot(_ theme: Theme, dark: Bool) -> WSkinPalette {
        let palette = dark ? theme.dark : theme.light
        return WSkinPalette(id: theme.id, name: theme.name, colors: [palette[.accentPrimary] ?? 0, palette[.backgroundPage] ?? 0])
    }

    static func labels() -> WSkinGalleryLabels {
        var l = WSkinGalleryLabels()
        l.footer = String(localized: "Each Mac keeps its own skin.")
        l.importTitle = String(localized: "Import Skin…")
        l.rename = String(localized: "Rename…")
        l.export = String(localized: "Export…")
        l.delete = String(localized: "Delete…")
        l.deleteConfirm = String(localized: "Delete")
        l.nameField = String(localized: "Name")
        return l
    }
}

/// What the gallery's cards call. Each function is the classic card's code, moved here.
@MainActor
struct SkinGalleryActionsAdapter {
    let skins: SkinStore
    let themes: ThemeStore
    let colorScheme: ColorScheme

    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "skins")

    func actions() -> WSkinGalleryActions {
        WSkinGalleryActions(
            select: select, selectPalette: selectPalette, rename: rename, export: export, delete: delete,
            importSkin: importSkin,
            preview: { id in AnyView(SkinCardPreview(skinId: id, skins: skins, colorScheme: colorScheme)) },
            optionsLabel: { name in String(localized: "\(name) options") },
            deleteQuestion: { name in String(localized: "Delete “\(name)”? Its chat and earlier versions go too.") })
    }

    private func skin(_ id: String) -> Skin? { skins.skins.first { $0.id == id } }

    /// Clicking a card picks the skin's first palette, except on the skin already selected (it keeps its current one).
    private func select(_ id: String) {
        guard let skin = skin(id) else { return }
        let isSelected = skins.selected.id == id
        skins.selectFromGallery(
            skinId: id,
            paletteId: SkinGallery.paletteToSelect(isSelected: isSelected, current: skins.selectedPaletteId, first: skins.palettes(for: skin)[0].id))
    }

    private func selectPalette(_ id: String, _ paletteId: String) {
        skins.selectFromGallery(skinId: id, paletteId: paletteId)
    }

    private func rename(_ id: String, _ name: String) -> String? {
        var failure: String?
        do {
            try UserSkinStore.shared.rename(skinId: id, to: name)
        } catch {
            Self.log.error("skin rename failed: \(String(describing: error), privacy: .public)")
            failure = String(localized: "Couldn't rename this skin.")
        }
        skins.scrollSettingsBack(to: SkinStore.galleryCardId(id))
        skins.reloadUserSkins()
        return failure
    }

    private func export(_ id: String) -> String? {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [SkinPackage.utType]
        // The id, not the name: it is always a valid file name (`a-z0-9-`), whatever the name holds.
        panel.nameFieldStringValue = id
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        do {
            try SkinPackage.export(skinId: id, from: .shared, to: url)
            return nil
        } catch SkinPackage.PackageError.tooLargeToExport {
            return String(localized: "This skin is too big to export (more than 64 MB or 200 files), so it couldn't be imported again.")
        } catch {
            Self.log.error("skin export failed: \(String(describing: error), privacy: .public)")
            return String(localized: "Couldn't export this skin.")
        }
    }

    private func delete(_ id: String) -> String? {
        var failure: String?
        do {
            try UserSkinStore.shared.delete(skinId: id)
        } catch {
            Self.log.error("skin delete failed: \(String(describing: error), privacy: .public)")
            failure = String(localized: "Couldn't delete this skin.")
        }
        skins.scrollSettingsBack(to: SkinChatView.scrollId)
        skins.reloadUserSkins()
        return failure
    }

    /// Adds the chosen file as another of your skins and scrolls to its card; the selection stays.
    private func importSkin() -> String? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [SkinPackage.utType]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        do {
            let id = try SkinPackage.importPackage(at: url, into: .shared)
            skins.scrollSettingsBack(to: SkinStore.galleryCardId(id))
            skins.reloadUserSkins()
            return nil
        } catch {
            Self.log.error("skin import failed: \(String(describing: error), privacy: .public)")
            return String(localized: "Whimbrel couldn't import that file. It needs to be a skin exported from Whimbrel.")
        }
    }
}

/// The card's live preview: the cached image at once, else drawn through `SkinPreviewCache` (retrying while another
/// card's render is in progress), as the classic card does.
struct SkinCardPreview: View {
    let skinId: String
    @ObservedObject var skins: SkinStore
    let colorScheme: ColorScheme
    @State private var preview: NSImage?

    private struct PreviewKey: Equatable {
        let paletteId: String
        let dark: Bool
        let revision: Int
        let themeRevision: Int
    }

    private var skin: Skin? { skins.skins.first { $0.id == skinId } }
    private var previewPaletteId: String { skin.map { SkinGallery.previewPaletteId(for: $0, skins: skins) } ?? "" }

    var body: some View {
        Group {
            if let skin, let image = preview ?? SkinGallery.cachedPreview(for: skin, skins: skins, colorScheme: colorScheme) {
                Image(nsImage: image).resizable().interpolation(.high).scaledToFit()
            } else {
                Color.whimbrelBackgroundPage
            }
        }
        .aspectRatio(SkinPreviewCache.renderSize.width / SkinPreviewCache.renderSize.height, contentMode: .fit)
        .overlay(Rectangle().stroke(Color.whimbrelBorder, lineWidth: 0.5))
        .accessibilityHidden(true)
        .task(id: PreviewKey(paletteId: previewPaletteId, dark: colorScheme == .dark, revision: skins.revision, themeRevision: ThemeStore.shared.revision)) {
            // Yield first so placeholders paint, and retry while another card's render is in progress.
            while !Task.isCancelled {
                await Task.yield()
                if let image = SkinPreviewCache.shared.imageIfIdle(skinId: skinId, paletteId: previewPaletteId, colorScheme: colorScheme) {
                    preview = image
                    return
                }
                try? await Task.sleep(for: .milliseconds(50))
            }
        }
    }
}
