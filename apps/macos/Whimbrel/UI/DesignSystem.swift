import SwiftUI

/// FR-UI-5: shared design tokens (palette, type scale, spacing, component styles) so
/// Settings and the debug window stop relying on default SwiftUI styling. Extends the
/// palette already established for the app icon (apps/macos/design/app-icon.svg) rather
/// than inventing a new one — rust/slate/ochre, drawn from the whimbrel's own plumage.
/// Timeline, chat, and the menu-bar popover will consume these same tokens in their own
/// later visual passes (FR-UI-2/3/4) — this file doesn't touch those views.

// MARK: - Colors

extension Color {
    /// Adaptive color that resolves to `light` or `dark` based on the current system
    /// appearance — avoids needing an Assets.xcassets Color Set for every token here.
    init(light: Color, dark: Color) {
        self.init(NSColor(name: nil, dynamicProvider: { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return NSColor(isDark ? dark : light)
        }))
    }

    /// Convenience initializer from a hex value, e.g. `Color(hex: 0xA8502E)`.
    init(hex: UInt32) {
        let r = Double((hex >> 16) & 0xFF) / 255
        let g = Double((hex >> 8) & 0xFF) / 255
        let b = Double(hex & 0xFF) / 255
        self.init(red: r, green: g, blue: b)
    }

    static let whimbrelBackgroundPage = Color.theme(.backgroundPage)
    static let whimbrelBackgroundCard = Color.theme(.backgroundCard)
    static let whimbrelBorder = Color.theme(.border)
    static let whimbrelTextPrimary = Color.theme(.textPrimary)
    static let whimbrelTextSecondary = Color.theme(.textSecondary)
    static let whimbrelAccentPrimary = Color.theme(.accentPrimary)
    static let whimbrelAccentSecondary = Color.theme(.accentSecondary)
    static let whimbrelAccentHighlight = Color.theme(.accentHighlight)
    /// Status colors, tuned to read on both card backgrounds, replacing raw `.red`,
    /// `.orange`, `.green` and `.blue` in the views.
    static let whimbrelError = Color.theme(.error)
    static let whimbrelWarning = Color.theme(.warning)
    static let whimbrelSuccess = Color.theme(.success)
    static let whimbrelLink = Color.theme(.link)
    /// The selected-row fill in sidebars.
    static let whimbrelSelection = Color.theme(.selection)
    /// The background of a cited or active transcript line.
    static let whimbrelHighlight = Color.theme(.highlight)
    /// Text and glyphs on an accent-filled button or badge.
    static let whimbrelTextOnAccent = Color.theme(.textOnAccent)
}

// MARK: - Typography

extension Font {
    /// Screen titles and section headers — resolves to New York (serif), pairing with
    /// SF Pro body text the same way Littlebird's reference UI splits headline/body.
    static func whimbrelDisplay(_ style: Font.TextStyle = .title2) -> Font {
        SkinStore.shared.selected.typography.display.font(style: style)
    }

    /// Body/caption text — explicit default-design wrapper so call sites read
    /// consistently with `.whimbrelDisplay()` rather than mixing bare `.system(...)` and
    /// wrapped calls.
    static func whimbrelBody(_ style: Font.TextStyle = .body) -> Font {
        let font = SkinStore.shared.selected.typography.body.font(size: whimbrelPointSize(for: style))
        return style == .headline ? font.weight(.semibold) : font
    }

    /// The macOS type scale: body 14, secondary 12, caption 11 (nothing readable below
    /// 11; `WhimbrelSectionLabel` sets its own 10pt). Other styles use their macOS default size.
    static func whimbrelPointSize(for style: Font.TextStyle) -> CGFloat {
        switch style {
        case .body: return 14
        case .callout: return 13
        case .subheadline, .footnote: return 12
        case .caption, .caption2: return 11
        case .headline: return 13
        case .title3: return 15
        case .title2: return 17
        case .title: return 22
        case .largeTitle: return 26
        default: return 13
        }
    }

    /// The page title: large, regular-weight display face (Today's greeting).
    static var whimbrelPageTitle: Font {
        let display = SkinStore.shared.selected.typography.display
        return SkinFace(display.family, .regular, italic: display.italic).font(size: 28)
    }
}

// MARK: - Spacing

/// 4pt grid, replacing today's inconsistent .padding(24)/.padding(16)/.padding(10) spread
/// through the views.
enum Spacing {
    static let xs: CGFloat = 4
    static let sm: CGFloat = 8
    static let md: CGFloat = 12
    static let lg: CGFloat = 16
    static let xl: CGFloat = 24
    static let xxl: CGFloat = 32
}

/// Corner radii. Cards use `card`; inputs, editors and banners use `control`.
enum Radius {
    static var control: CGFloat { SkinStore.shared.selected.shape.controlRadius }
    static var card: CGFloat { SkinStore.shared.selected.shape.radius }
}

// MARK: - Card style

private struct WhimbrelCardStyle: ViewModifier {
    func body(content: Content) -> some View {
        let shape = SkinStore.shared.selected.shape
        let outline = RoundedRectangle(cornerRadius: shape.radius)
        return content
            .padding(Spacing.lg * shape.density.scale)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(outline)
            .overlay {
                switch shape.border {
                case .none: EmptyView()
                case .solid: outline.stroke(Color.whimbrelBorder, lineWidth: shape.borderWidth)
                case .dashed: outline.stroke(Color.whimbrelBorder, style: StrokeStyle(lineWidth: shape.borderWidth, dash: [4, 3]))
                case .double:
                    ZStack {
                        outline.stroke(Color.whimbrelBorder, lineWidth: shape.borderWidth)
                        outline.inset(by: shape.borderWidth * 2.5).stroke(Color.whimbrelBorder, lineWidth: shape.borderWidth)
                    }
                }
            }
    }
}

extension View {
    func whimbrelCard() -> some View {
        modifier(WhimbrelCardStyle())
    }
}

// MARK: - Pill button

struct WhimbrelPillButtonStyle: ButtonStyle {
    enum Kind {
        case primary
        case secondary
        case quiet
    }

    let kind: Kind

    func makeBody(configuration: Configuration) -> some View {
        WhimbrelPillButtonBody(kind: kind, configuration: configuration)
    }

    /// Environment values (like `isEnabled`) are only resolved correctly when read from
    /// inside a `View`'s `body` — reading `@Environment` on the `ButtonStyle` itself, or
    /// from `makeBody`'s local scope, does not reliably observe the enclosing
    /// `.disabled(...)` modifier. Pulling `configuration.label` into this inner view
    /// makes `@Environment(\.isEnabled)` resolve against the button's actual environment.
    private struct WhimbrelPillButtonBody: View {
        let kind: Kind
        let configuration: Configuration

        @Environment(\.isEnabled) private var isEnabled
        @State private var isHovered = false

        var body: some View {
            configuration.label
                .font(.whimbrelBody(.callout).weight(.medium))
                .padding(.horizontal, Spacing.lg)
                .padding(.vertical, Spacing.sm)
                // The secondary border is a filled capsule behind an inset one, not a stroke: a stroked
                // capsule leaves a sliver at each end when rendered offscreen.
                .background(Capsule().inset(by: kind == .secondary ? 1 : 0).fill(background))
                .background(Capsule().fill(kind == .secondary ? Color.whimbrelBorder : .clear))
                .foregroundStyle(foreground)
                // The hover tint must be an overlay, not a second `.background` — a
                // `.background` behind the opaque fill above would be fully occluded and
                // never actually render. Overlaying it (still before `.clipShape`, so the
                // eventual focus ring's outset isn't cut off by the Capsule mask below)
                // composites the tint on top of the base fill instead.
                .overlay(
                    Capsule()
                        .fill((kind == .primary ? Color.whimbrelTextOnAccent : Color.whimbrelTextPrimary).opacity(isHovered && isEnabled ? (kind == .primary ? 0.08 : 0.06) : 0))
                )
                .opacity(configuration.isPressed ? 0.7 : 1)
                .opacity(isEnabled ? 1 : 0.4)
                .onHover { hovering in isHovered = hovering }
        }

        private var background: Color {
            switch kind {
            case .primary: return .whimbrelAccentPrimary
            case .secondary: return .whimbrelBackgroundCard
            case .quiet: return .clear
            }
        }

        private var foreground: Color {
            switch kind {
            case .primary: return .whimbrelTextOnAccent
            case .secondary, .quiet: return .whimbrelTextPrimary
            }
        }
    }
}

extension ButtonStyle where Self == WhimbrelPillButtonStyle {
    static var whimbrelPrimary: WhimbrelPillButtonStyle { WhimbrelPillButtonStyle(kind: .primary) }
    static var whimbrelSecondary: WhimbrelPillButtonStyle { WhimbrelPillButtonStyle(kind: .secondary) }
    static var whimbrelQuiet: WhimbrelPillButtonStyle { WhimbrelPillButtonStyle(kind: .quiet) }
}

// MARK: - Accent-bar row

/// A colored vertical bar to the left of row content — Timeline's later visual pass
/// (FR-UI-2) will vary the bar color by item type (screen text / meeting / digest); not
/// used anywhere yet, included now as a simple, general-purpose primitive.
struct AccentBarRow<Content: View>: View {
    let color: Color
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: Spacing.sm) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(color)
                .frame(width: 3)
            content
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: - Surface

/// A larger, purpose-built content container -- distinct from `.whimbrelCard()` (a
/// small bordered card for list-style rows) in that it's meant for one dominant block
/// of content (a Meeting Brief, an active-note canvas), with more generous internal
/// spacing and no implied "one of many equal items" connotation.
struct WhimbrelSurface<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        content
            .padding(Spacing.xl)
            .background(Color.whimbrelBackgroundCard)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .overlay(
                RoundedRectangle(cornerRadius: 16)
                    .stroke(Color.whimbrelBorder, lineWidth: 1)
            )
    }
}

// MARK: - Section label

/// A small uppercase section header (e.g. "SUGGESTED QUESTIONS", "OPEN LOOPS") --
/// distinct from `.whimbrelDisplay()`, which is for the page/meeting title itself.
struct WhimbrelSectionLabel: View {
    let text: String

    var body: some View {
        let typography = SkinStore.shared.selected.typography
        let small = typography.caps == .small
        let labelFont = SkinFace(typography.label.family, .semibold, italic: typography.label.italic).font(size: 10)
        Text(typography.caps.apply(text))
            .font(small ? labelFont.smallCaps() : labelFont)
            .tracking(0.6)
            .foregroundStyle(Color.whimbrelTextSecondary)
    }
}

// MARK: - Metadata chip

/// A small subdued pill for time/attendees/source-count/local-vs-external/model
/// metadata -- never used for a primary action.
struct WhimbrelMetadataChip: View {
    let systemImage: String?
    let text: String

    init(systemImage: String? = nil, text: String) {
        self.systemImage = systemImage
        self.text = text
    }

    var body: some View {
        HStack(spacing: Spacing.xs) {
            if let systemImage {
                Image(systemName: systemImage)
                    .font(.system(size: 10))
            }
            Text(text)
        }
        .font(.whimbrelBody(.caption))
        .foregroundStyle(Color.whimbrelTextSecondary)
        .padding(.horizontal, Spacing.sm)
        .padding(.vertical, 3)
        .background(Color.whimbrelBackgroundPage)
        .clipShape(Capsule())
    }
}

// MARK: - Empty state

/// A calm, honest empty/unavailable state -- for "No brief yet", "No calendar
/// connected", "Nothing waiting on you", etc. Never implies a failure when there's
/// simply nothing to show.
struct WhimbrelEmptyState: View {
    let systemImage: String
    let title: String
    let message: String
    var actionTitle: String?
    var action: (() -> Void)?

    var body: some View {
        VStack(spacing: Spacing.md) {
            Image(systemName: systemImage)
                .font(.system(size: 28))
                .foregroundStyle(Color.whimbrelTextSecondary)
            Text(title)
                .font(.whimbrelBody(.body).weight(.medium))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text(message)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .multilineTextAlignment(.center)
            if let actionTitle, let action {
                Button(actionTitle, action: action)
                    .buttonStyle(.whimbrelSecondary)
            }
        }
        .frame(maxWidth: 320)
        .padding(Spacing.xl)
        .accessibilityElement(children: .combine)
    }
}

// MARK: - Source chip

/// A provenance chip naming what backs a piece of content (e.g. "2 past meetings",
/// "1 Gmail thread", "Local calendar"). Interactive ONLY when a real detail path
/// exists -- pass `onTap = nil` to render a plain, non-interactive chip rather than
/// implying a destination that doesn't exist yet.
struct WhimbrelSourceChip: View {
    let systemImage: String
    let text: String
    /// An image file shown in place of the icon once it loads (a slide's thumbnail).
    var thumbnailPath: String? = nil
    var onTap: (() -> Void)?
    @State private var thumbnail: NSImage?

    var body: some View {
        let content = HStack(spacing: Spacing.xs) {
            if let thumbnail {
                Image(nsImage: thumbnail).resizable().scaledToFit()
                    .frame(height: 28)
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            } else {
                Image(systemName: systemImage)
                    .font(.system(size: 11))
            }
            Text(text)
                .font(.whimbrelBody(.caption))
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .foregroundStyle(Color.whimbrelAccentSecondary)
        .padding(.horizontal, Spacing.sm)
        .padding(.vertical, 4)
        .background(Color.whimbrelAccentSecondary.opacity(0.1))
        .clipShape(Capsule())
        .task(id: thumbnailPath) {
            thumbnail = nil
            if let thumbnailPath { thumbnail = await SnapshotThumbnails.image(for: thumbnailPath) }
        }

        if let onTap {
            Button(action: onTap) { content }
                .buttonStyle(.plain)
        } else {
            content
        }
    }
}

/// The device a row came from, as a chip, with the full device id as its tooltip.
/// One component so Library rows and every detail view label devices the same way.
struct DeviceChip: View {
    @EnvironmentObject var appState: AppState
    let deviceId: String

    var body: some View {
        WhimbrelMetadataChip(text: appState.deviceLabel(for: deviceId))
            .help(deviceId)
    }
}
