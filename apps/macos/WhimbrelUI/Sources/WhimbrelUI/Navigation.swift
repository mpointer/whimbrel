import SwiftUI

// MARK: Sidebar

public struct WSidebarItem: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var hovering = false
    let icon: String
    let label: String
    let badge: String?
    let shortcut: String?
    let isSelected: Bool
    let action: () -> Void

    public init(icon: String, label: String, badge: String? = nil, shortcut: String? = nil, isSelected: Bool = false, action: @escaping () -> Void) {
        self.icon = icon
        self.label = label
        self.badge = badge
        self.shortcut = shortcut
        self.isSelected = isSelected
        self.action = action
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        Button(action: action) {
            HStack(spacing: theme.space(10)) {
                WGlyph(name: icon, size: 14, weight: .medium)
                    .frame(width: 20 * k)
                    .foregroundStyle(isSelected ? theme.accentPrimary : theme.textSecondary)
                Text(label)
                    .wFont(isSelected ? .bodyStrong : .body)
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(1)
                Spacer(minLength: theme.space(4))
                if let badge {
                    Text(badge)
                        .wFont(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.textOnAccent)
                        .padding(.horizontal, theme.space(6))
                        .padding(.vertical, theme.space(1))
                        .background(Capsule().fill(theme.accentPrimary))
                }
                if let shortcut {
                    Text(shortcut).wFont(.secondary).foregroundStyle(theme.textSecondary).accessibilityHidden(true)
                }
            }
            .padding(.horizontal, theme.space(10))
            .frame(minHeight: WTokens.Size.target * k + theme.space(4))
            .background(shape.fill(isSelected ? theme.selection : (hovering ? theme.selection.opacity(0.5) : .clear)))
            .overlay {
                if isSelected && theme.isHighContrast { shape.strokeBorder(theme.border, lineWidth: theme.shape.highContrastBorderWidth) }
            }
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityLabel(label)
        .accessibilityValue(badge ?? "")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

/// The navigation surface: items on glass (a material before macOS 26, solid when Reduce Transparency
/// is on) with a footer slot for the capture state.
public struct WSidebar<Content: View, Footer: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let content: Content
    let footer: Footer

    public init(@ViewBuilder content: () -> Content, @ViewBuilder footer: () -> Footer) {
        self.content = content()
        self.footer = footer()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(2)) {
            content
            Spacer(minLength: theme.space(12))
            footer
        }
        .padding(theme.space(10))
        .frame(width: WTokens.Size.sidebar * k)
        .wGlass(kind: .sidebar)
    }
}

/// A small title above a group of sidebar items.
public struct WSidebarSectionTitle: View {
    @Environment(\.wTheme) private var theme
    let title: String
    public init(_ title: String) { self.title = title }

    public var body: some View {
        Text(theme.sectionLabel(title))
            .wFont(.secondary)
            .fontWeight(.semibold)
            .foregroundStyle(theme.textSecondary)
            .padding(.horizontal, theme.space(10))
            .padding(.top, theme.space(10))
            .padding(.bottom, theme.space(2))
            .accessibilityAddTraits(.isHeader)
    }
}

// MARK: Search and filters

public struct WSearchBar: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Binding var text: String
    let placeholder: String?
    let shortcutHint: String?
    let onClear: (() -> Void)?
    let onSubmit: (() -> Void)?
    let focus: FocusState<Bool>.Binding?

    /// - focus: where the app holds the field's keyboard focus, so a shortcut can move it here.
    public init(text: Binding<String>, placeholder: String? = nil, shortcutHint: String? = "\u{2318}K", focus: FocusState<Bool>.Binding? = nil, onClear: (() -> Void)? = nil, onSubmit: (() -> Void)? = nil) {
        self.focus = focus
        self._text = text
        self.placeholder = placeholder
        self.shortcutHint = shortcutHint
        self.onClear = onClear
        self.onSubmit = onSubmit
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius * 1.5, style: .continuous)
        HStack(spacing: theme.space(8)) {
            WGlyph(name: "magnifyingglass", size: 13).foregroundStyle(theme.textSecondary)
            TextField(placeholder ?? strings.searchPlaceholder, text: $text)
                .textFieldStyle(.plain)
                .wFont(.body)
                .foregroundStyle(theme.textPrimary)
                .accessibilityLabel(placeholder ?? strings.searchPlaceholder)
                .wFocused(focus)
                .onSubmit { onSubmit?() }
            if !text.isEmpty {
                Button {
                    text = ""
                    onClear?()
                } label: {
                    WGlyph(name: "xmark.circle.fill", size: 13).foregroundStyle(theme.textSecondary)
                        .frame(width: WTokens.Size.target * k * 0.8, height: WTokens.Size.target * k * 0.8)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(strings.clearSearch)
            } else if let shortcutHint {
                WKeycap(text: shortcutHint)
            }
        }
        .padding(.horizontal, theme.space(12))
        .frame(minHeight: 34 * k)
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
    }
}

public struct WFilterChip: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let title: String
    let glyph: String?
    let showsChevron: Bool
    let isSelected: Bool
    let onRemove: (() -> Void)?
    let action: () -> Void

    public init(_ title: String, glyph: String? = nil, showsChevron: Bool = true, isSelected: Bool = false, onRemove: (() -> Void)? = nil, action: @escaping () -> Void) {
        self.title = title
        self.glyph = glyph
        self.showsChevron = showsChevron
        self.isSelected = isSelected
        self.onRemove = onRemove
        self.action = action
    }

    public var body: some View {
        HStack(spacing: theme.space(2)) {
            Button(action: action) {
                HStack(spacing: theme.space(6)) {
                    if isSelected { WGlyph(name: "checkmark", size: 10, weight: .bold) }
                    else if let glyph { WGlyph(name: glyph, size: 12) }
                    Text(title).wFont(.bodyStrong).lineLimit(1)
                    if showsChevron && onRemove == nil { WGlyph(name: "chevron.down", size: 9, weight: .bold) }
                }
                .foregroundStyle(isSelected ? theme.accentPrimary : theme.textPrimary)
                .padding(.leading, theme.space(12))
                .padding(.trailing, onRemove == nil ? theme.space(12) : theme.space(4))
                .frame(minHeight: WTokens.Size.target * k)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(title)
            .accessibilityAddTraits(isSelected ? .isSelected : [])
            if let onRemove {
                Button(action: onRemove) {
                    WGlyph(name: "xmark", size: 10, weight: .bold)
                        .foregroundStyle(theme.textSecondary)
                        .frame(width: WTokens.Size.target * k * 0.8, height: WTokens.Size.target * k)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(strings.removeFilter(title))
            }
        }
        .background(Capsule().fill(isSelected ? theme.selection : theme.backgroundCard))
        .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(isSelected ? theme.accentPrimary : theme.border, style: theme.borderStroke.widened(theme.isHighContrast || isSelected ? 1 : 0.75)))
    }
}

// MARK: Spine

public enum WSpineNode: Sendable { case upcoming, now, past, recording }
public enum WSpineConnector: Sendable { case none, solid, dashed, span, recording }

/// The time line. Stack `WSpineStop` and `WNowMarker` inside it.
public struct WSpine<Content: View>: View {
    @Environment(\.wStrings) private var strings
    let content: Content
    public init(@ViewBuilder content: () -> Content) { self.content = content() }
    public var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(strings.timeline)
    }
}

struct WSpineNodeView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let node: WSpineNode

    var body: some View {
        let d = 14 * k
        ZStack {
            switch node {
            case .upcoming:
                Circle().fill(theme.backgroundPage)
                Circle().strokeBorder(theme.accentSecondary, lineWidth: 2)
            case .now:
                Circle().fill(theme.backgroundPage)
                Circle().strokeBorder(theme.accentPrimary, lineWidth: 2.5)
                Circle().fill(theme.accentPrimary).frame(width: d * 0.5, height: d * 0.5)
            case .past:
                Circle().fill(theme.backgroundPage)
                Circle().strokeBorder(theme.accentPrimary, lineWidth: 2)
            case .recording:
                Circle().fill(theme.recording)
                Circle().strokeBorder(theme.backgroundPage, lineWidth: 2).padding(1)
            }
        }
        .frame(width: d, height: d)
        .accessibilityHidden(true)
    }
}

struct WSpineLine: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wSpineDividers) private var spineDividers
    @Environment(\.wTextScale) private var k
    let connector: WSpineConnector

    var body: some View {
        switch connector {
        case .none:
            Color.clear.frame(width: 1)
        case .solid:
            if spineDividers && theme.shape.dividers != .none {
                WDividerStrokes(axis: .vertical, color: theme.accentPrimary.opacity(theme.isHighContrast ? 1 : 0.45), usesStrokeOpacity: false)
            } else {
                Rectangle().fill(theme.accentPrimary.opacity(theme.isHighContrast ? 1 : 0.45)).frame(width: 2)
            }
        case .dashed:
            if spineDividers && theme.shape.dividers != .none {
                WDividerStrokes(axis: .vertical, color: theme.accentSecondary.opacity(theme.isHighContrast ? 1 : 0.7), usesStrokeOpacity: false)
            } else {
                VLine().stroke(theme.accentSecondary.opacity(theme.isHighContrast ? 1 : 0.7), style: StrokeStyle(lineWidth: 1.5, dash: [3, 4])).frame(width: 2)
            }
        case .span:
            Capsule().fill(theme.accentPrimary.opacity(theme.isHighContrast ? 1 : 0.5)).frame(width: 6 * k)
        case .recording:
            Rectangle().fill(theme.recording).frame(width: 3)
        }
    }
}

private struct VLine: Shape {
    func path(in rect: CGRect) -> Path {
        var p = Path()
        p.move(to: CGPoint(x: rect.midX, y: rect.minY))
        p.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
        return p
    }
}

/// One row of the time line: a time label, a node, a connector and content beside it.
public struct WSpineStop<Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let time: String
    let caption: String?
    let node: WSpineNode
    let connector: WSpineConnector
    let connectsFromAbove: Bool
    let content: Content

    public init(time: String, caption: String? = nil, node: WSpineNode, connector: WSpineConnector = .solid, connectsFromAbove: Bool = true, @ViewBuilder content: () -> Content) {
        self.time = time
        self.caption = caption
        self.node = node
        self.connector = connector
        self.connectsFromAbove = connectsFromAbove
        self.content = content()
    }

    public var body: some View {
        let nodeY = theme.space(20) * k
        let timeWidth = (WTokens.Size.spine - 24) * k
        HStack(alignment: .top, spacing: 0) {
            VStack(alignment: .trailing, spacing: 0) {
                Text(time)
                    .wFont(node == .past ? .secondary : .time)
                    .foregroundStyle(node == .now ? theme.accentPrimary : (node == .past ? theme.textSecondary : theme.textPrimary))
                    .fontWeight(node == .past ? .regular : .semibold)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                if let caption {
                    Text(caption).wFont(.caption).foregroundStyle(theme.textSecondary).multilineTextAlignment(.trailing)
                }
            }
            .frame(width: timeWidth, alignment: .trailing)
            .padding(.top, max(0, nodeY - 9 * k))
            Color.clear.frame(width: 24 * k, height: 1)
            content
                .padding(.bottom, theme.space(8))
                .padding(.leading, theme.space(4))
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(alignment: .topLeading) {
            ZStack(alignment: .top) {
                WSpineLine(connector: connector)
                    .padding(.top, connectsFromAbove ? 0 : nodeY)
                    .frame(maxHeight: .infinity)
                WSpineNodeView(node: node)
                    .padding(.top, nodeY - 7 * k)
            }
            .frame(width: 24 * k)
            .frame(maxHeight: .infinity, alignment: .top)
            .padding(.leading, timeWidth)
        }
        .accessibilityElement(children: .contain)
    }
}

/// "Now" on the time line, with the capture state beside it.
public struct WNowMarker: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let time: String
    let label: String
    let connector: WSpineConnector

    public init(time: String, label: String, connector: WSpineConnector = .solid) {
        self.time = time
        self.label = label
        self.connector = connector
    }

    public var body: some View {
        WSpineStop(time: time, node: .now, connector: connector) {
            HStack(spacing: theme.space(8)) {
                HStack(spacing: theme.space(8)) {
                    Text(strings.now).wFont(.bodyStrong)
                    Text(label).wFont(.body)
                }
                .foregroundStyle(theme.textOnAccent)
                .padding(.horizontal, theme.space(12))
                .frame(minHeight: WTokens.Size.target * k)
                .background(Capsule().fill(theme.accentPrimary))
                .overlay(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(theme.isHighContrast ? theme.textPrimary : .clear, style: StrokeStyle(lineWidth: theme.shape.highContrastBorderWidth)))
                Rectangle().fill(theme.accentPrimary.opacity(0.5)).frame(height: 1.5)
            }
            .frame(minHeight: 28 * k)
            .padding(.top, theme.space(6) * k)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(strings.joinDetails([strings.now, label]))
        }
    }
}

extension View {
    /// Ties a field to a focus state when the app gave one.
    @ViewBuilder
    func wFocused(_ binding: FocusState<Bool>.Binding?) -> some View {
        if let binding { self.focused(binding) } else { self }
    }
}
