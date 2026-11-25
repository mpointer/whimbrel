import SwiftUI

// The settings window's section list and its search results.

public struct WSettingsNavItem: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let icon: String
    /// A count shown on the trailing edge (suggestions waiting). Read after the title.
    public let badge: String?

    public init(id: String, title: String, icon: String, badge: String? = nil) {
        self.id = id
        self.title = title
        self.icon = icon
        self.badge = badge
    }
}

/// One row of the nav or of the search results: a selection fill, a leading bar and, for the keyboard, a ring.
/// The selected state is the fill and the bar together, so it does not rest on colour alone, and in high contrast it
/// also gets an outline. It does not depend on the window being active.
private struct WSelectableRow<Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wForceFocusRing) private var forced
    @State private var hovering = false
    let isSelected: Bool
    let showsFocus: Bool
    let content: Content

    init(isSelected: Bool, showsFocus: Bool, @ViewBuilder content: () -> Content) {
        self.isSelected = isSelected
        self.showsFocus = showsFocus
        self.content = content()
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        HStack(spacing: 0) {
            Capsule()
                .fill(isSelected ? theme.accentPrimary : .clear)
                .frame(width: 3 * k, height: 18 * k)
                .padding(.leading, theme.space(3))
            content
                .padding(.leading, theme.space(8))
                .padding(.trailing, theme.space(10))
        }
        .frame(minHeight: WTokens.Size.target * k + theme.space(4))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(isSelected ? theme.selection : (hovering ? theme.selection.opacity(0.5) : .clear)))
        .overlay {
            if isSelected && theme.isHighContrast { shape.strokeBorder(theme.textPrimary, lineWidth: theme.shape.highContrastBorderWidth) }
        }
        .overlay {
            if showsFocus || forced && isSelected {
                shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-1).allowsHitTesting(false).accessibilityHidden(true)
            }
        }
        .contentShape(shape)
        .onHover { hovering = $0 }
    }
}

/// The section list of a settings window: icon and title per row, one selected.
///
/// Keyboard: the list is one Tab stop. Up and Down move the selection and open that section at once (as a sidebar does),
/// Home and End jump to the ends, and typing letters jumps to a title that starts with them. Movement does not wrap.
/// VoiceOver: a list of items; the current one has the selected trait and a badge is read after the title.
public struct WSettingsNav: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @FocusState private var focused: Bool
    @State private var typed = ""
    @State private var lastTyped = Date.distantPast
    let items: [WSettingsNavItem]
    let selection: String?
    let label: String?
    let onSelect: (String) -> Void

    public init(items: [WSettingsNavItem], selection: String?, label: String? = nil, onSelect: @escaping (String) -> Void) {
        self.items = items
        self.selection = selection
        self.label = label
        self.onSelect = onSelect
    }

    private var index: Int? { items.firstIndex { $0.id == selection } }
    private var listLabel: String { label ?? strings.settings.sectionsLabel }

    private func move(_ key: WListKey) {
        guard let next = WListNavigation.index(after: index, count: items.count, key: key), items[next].id != selection else { return }
        onSelect(items[next].id)
    }

    private func type(_ characters: String) {
        let now = Date()
        if now.timeIntervalSince(lastTyped) > 1 { typed = "" }
        lastTyped = now
        typed += characters
        let from = typed.count > 1 ? (index ?? 0) - 1 : index
        if let hit = WListNavigation.typeAhead(typed, titles: items.map(\.title), from: from), items[hit].id != selection {
            onSelect(items[hit].id)
        }
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(2)) {
            ForEach(items) { item in
                let on = item.id == selection
                WSelectableRow(isSelected: on, showsFocus: focused && on) {
                    HStack(spacing: theme.space(10)) {
                        WGlyph(name: item.icon, size: 14, weight: .medium)
                            .frame(width: 20 * k)
                            .foregroundStyle(on ? theme.accentPrimary : theme.textSecondary)
                        Text(item.title)
                            .wFont(on ? .bodyStrong : .body)
                            .foregroundStyle(theme.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: theme.space(4))
                        if let badge = item.badge {
                            Text(badge).wFont(.caption).fontWeight(.semibold)
                                .foregroundStyle(theme.textOnAccent)
                                .padding(.horizontal, theme.space(6)).padding(.vertical, theme.space(1))
                                .background(Capsule().fill(theme.accentPrimary))
                        }
                    }
                    .padding(.vertical, theme.space(3))
                }
                .onTapGesture { onSelect(item.id) }
            }
        }
        .padding(theme.space(8))
        .focusable()
        .focused($focused)
        .onKeyPress(.downArrow) { move(.next); return .handled }
        .onKeyPress(.upArrow) { move(.previous); return .handled }
        .onKeyPress(.home) { move(.first); return .handled }
        .onKeyPress(.end) { move(.last); return .handled }
        .onKeyPress(characters: .letters.union(.decimalDigits), phases: .down) { press in
            guard press.modifiers.intersection([.command, .control, .option]).isEmpty else { return .ignored }
            type(press.characters)
            return .handled
        }
        .accessibilityRepresentation {
            List {
                ForEach(items) { item in
                    Button(item.title) { onSelect(item.id) }
                        .accessibilityValue(item.badge ?? "")
                        .accessibilityAddTraits(item.id == selection ? .isSelected : [])
                }
            }
            .accessibilityLabel(listLabel)
        }
    }
}

// MARK: Search results

public struct WSettingsSearchResult: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    /// The section it lives in: the caption under the title.
    public let section: String

    public init(id: String, title: String, section: String) {
        self.id = id
        self.title = title
        self.section = section
    }
}

/// The result list for the settings search. One Tab stop; Up and Down move the highlighted result, Return opens it,
/// a click opens it. With no results it says "No matching settings".
public struct WSettingsSearchResults: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @FocusState private var focused: Bool
    @State private var current: Int?
    let results: [WSettingsSearchResult]
    let emptyText: String?
    let onOpen: (String) -> Void

    /// - highlighted: the result that starts highlighted (nil: none until the first arrow key).
    public init(results: [WSettingsSearchResult], emptyText: String? = nil, highlighted: Int? = nil, onOpen: @escaping (String) -> Void) {
        _current = State(initialValue: highlighted)
        self.results = results
        self.emptyText = emptyText
        self.onOpen = onOpen
    }

    private func move(_ key: WListKey) {
        current = WListNavigation.index(after: current, count: results.count, key: key)
    }

    public var body: some View {
        Group {
            if results.isEmpty {
                VStack(spacing: theme.space(8)) {
                    WGlyph(name: "magnifyingglass", size: 22, weight: .light).foregroundStyle(theme.textSecondary)
                    Text(emptyText ?? strings.settings.noMatchingSettings)
                        .wFont(.body).foregroundStyle(theme.textSecondary).multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(theme.space(24))
                .frame(maxWidth: .infinity)
                .accessibilityElement(children: .combine)
            } else {
                VStack(alignment: .leading, spacing: theme.space(2)) {
                    ForEach(Array(results.enumerated()), id: \.element.id) { index, result in
                        let on = index == current
                        WSelectableRow(isSelected: on, showsFocus: focused && on) {
                            HStack(spacing: theme.space(8)) {
                                VStack(alignment: .leading, spacing: theme.space(1)) {
                                    Text(result.title).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                                    Text(result.section).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                                }
                                Spacer(minLength: theme.space(4))
                                WGlyph(name: "chevron.right", size: 11, weight: .semibold).foregroundStyle(theme.textSecondary)
                            }
                            .padding(.vertical, theme.space(5))
                        }
                        .onTapGesture { onOpen(result.id) }
                    }
                }
                .padding(theme.space(8))
                .focusable()
                .focused($focused)
                .onKeyPress(.downArrow) { move(.next); return .handled }
                .onKeyPress(.upArrow) { move(.previous); return .handled }
                .onKeyPress(.home) { move(.first); return .handled }
                .onKeyPress(.end) { move(.last); return .handled }
                .onKeyPress(.return) {
                    guard let current, results.indices.contains(current) else { return .ignored }
                    onOpen(results[current].id)
                    return .handled
                }
                .accessibilityRepresentation {
                    List {
                        ForEach(results) { result in
                            Button(strings.settings.resultInSection(result.title, result.section)) { onOpen(result.id) }
                        }
                    }
                    .accessibilityLabel(strings.settings.searchResultsLabel)
                }
            }
        }
    }
}
