import SwiftUI

// Library's views around Find: the switch, and Recent, Episodes and Digest drawn with the same cards and day grouping.

/// The page of a Library view: the app's status header and notices, the view switch, then the view and the right hand panel.
public struct WLibraryPage<Header: View, Panel: View, Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let selection: WLibraryView
    let onSelect: (WLibraryView) -> Void
    let header: Header
    let panel: Panel
    let content: Content

    public init(
        selection: WLibraryView, onSelect: @escaping (WLibraryView) -> Void,
        @ViewBuilder header: () -> Header, @ViewBuilder panel: () -> Panel, @ViewBuilder content: () -> Content
    ) {
        self.selection = selection
        self.onSelect = onSelect
        self.header = header()
        self.panel = panel()
        self.content = content()
    }

    private func title(_ view: WLibraryView) -> String {
        switch view {
        case .find: return strings.find.viewFind
        case .recent: return strings.find.viewRecent
        case .episodes: return strings.find.viewEpisodes
        case .digest: return strings.find.viewDigest
        }
    }

    public var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: theme.space(12)) {
                    header
                    viewSwitch
                }
                .padding(.horizontal, theme.space(20))
                .padding(.bottom, theme.space(8))
                content
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            Divider().overlay(theme.border.opacity(0.6))
            panel
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .wPageBackground()
    }

    private var viewSwitch: some View {
        HStack(spacing: theme.space(4)) {
            ForEach(WLibraryView.allCases, id: \.self) { view in
                let on = view == selection
                Button { onSelect(view) } label: {
                    Text(title(view)).wFont(.bodyStrong)
                        .foregroundStyle(on ? theme.accentOnSelection : theme.textSecondary)
                        .padding(.horizontal, theme.space(14)).frame(minHeight: WTokens.Size.target * k)
                        .background(Capsule().fill(on ? theme.selection : .clear))
                        .overlay { if on && theme.isHighContrast { Capsule().strokeBorder(theme.border, lineWidth: theme.shape.highContrastBorderWidth) } }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(title(view))
                .accessibilityAddTraits(on ? .isSelected : [])
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.find.viewSwitchName)
    }
}

/// Recent and Episodes: rows of cards under day headings, newest first. A toggle on the right is the view's own switch ("Show screen
/// captures", "Show hidden episodes"). A meeting row keeps Open in new window and Forget meeting.
public struct WLibraryListView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.wScrollsContent) private var scrolls
    let model: WLibraryListModel
    let actions: WLibraryActions
    let toggleTitle: String?
    let toggleOn: Binding<Bool>?

    public init(model: WLibraryListModel, actions: WLibraryActions, toggleTitle: String? = nil, toggleOn: Binding<Bool>? = nil) {
        self.model = model
        self.actions = actions
        self.toggleTitle = toggleTitle
        self.toggleOn = toggleOn
    }

    /// One group per day: its heading and its rows, so a screen reader meets a named group, as on the Find page.
    private struct Group: Identifiable {
        var id: String
        var heading: String
        var rows: [WLibraryRow]
    }

    private var groups: [Group] {
        let cards = model.rows.map(\.card)
        var byCard: [String: WLibraryRow] = [:]
        for row in model.rows { byCard[row.card.id] = row }
        return WFindDays.group(cards, today: strings.home.today, yesterday: strings.home.yesterday, undated: strings.find.noLongerOnDevice).map { day in
            Group(id: wid("group", day.id), heading: day.heading, rows: day.cards.compactMap { byCard[$0.id] })
        }
    }

    public var body: some View {
        let list = LazyVStack(alignment: .leading, spacing: 0) {
            if let toggleTitle, let toggleOn {
                HStack {
                    Spacer()
                    Toggle(toggleTitle, isOn: toggleOn).toggleStyle(.switch).wFont(.body).tint(theme.accentPrimary)
                }
                .padding(.bottom, theme.space(6))
            }
            if model.rows.isEmpty {
                if model.isLoading { WLoadingState() } else { WEmptyState(glyph: "tray", title: model.emptyTitle, help: model.emptyHelp) }
            } else {
                ForEach(groups) { group in
                    VStack(alignment: .leading, spacing: 0) {
                        WSectionLabel(group.heading).padding(.top, theme.space(14)).padding(.bottom, theme.space(6))
                            .accessibilityAddTraits(.isHeader)
                        ForEach(group.rows) { row in rowView(row).padding(.bottom, theme.space(10)) }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel(group.heading)
                }
            }
        }
        .padding(.horizontal, theme.space(20))
        .padding(.vertical, theme.space(8))
        .frame(maxWidth: .infinity, alignment: .topLeading)
        if scrolls { ScrollView { list } } else { list }
    }

    private func rowView(_ row: WLibraryRow) -> some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            WFindCardView(card: row.card, isSelected: false, onOpen: actions.open, tag: row.isHidden ? strings.find.hiddenEpisode : nil, status: row.statusLine)
            if row.canOpenInNewWindow || row.canForget {
                HStack(spacing: theme.space(8)) {
                    if row.canOpenInNewWindow {
                        WButton(strings.find.openInNewWindow, glyph: "macwindow", kind: .quiet) { actions.openInNewWindow(row.id) }
                    }
                    if row.canForget {
                        WButton(strings.find.forgetMeeting, glyph: "trash", kind: .quiet) { actions.forget(row.id) }
                    }
                    Spacer(minLength: 0)
                }
            }
        }
        .contextMenu {
            if row.canOpenInNewWindow { Button(strings.find.openInNewWindow) { actions.openInNewWindow(row.id) } }
            if row.canForget { Button(strings.find.forgetMeeting, role: .destructive) { actions.forget(row.id) } }
        }
    }
}

/// Digest: the button, and what it made.
public struct WLibraryDigestView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    @Environment(\.wScrollsContent) private var scrolls
    let model: WLibraryDigestModel
    let actions: WLibraryActions
    let hasDigest: Bool

    public init(model: WLibraryDigestModel, actions: WLibraryActions) {
        self.model = model
        self.actions = actions
        if case .ready = model.state { hasDigest = true } else { hasDigest = false }
    }

    public var body: some View {
        let page = VStack(alignment: .leading, spacing: theme.space(14)) {
            WButton(hasDigest ? strings.find.regenerateDigest : strings.find.generateDigest, glyph: "sparkles", kind: .primary, action: actions.generateDigest)
                .disabled(!model.canGenerate)
            switch model.state {
            case .idle(let help):
                Text(help).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            case .generating(let line):
                WLoadingState(title: line)
            case .failed(let message):
                Text(message).wFont(.body).foregroundStyle(theme.error).fixedSize(horizontal: false, vertical: true)
            case .ready(let summary, let loops):
                VStack(alignment: .leading, spacing: theme.space(10)) {
                    Text(summary).wFont(.reading).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
                    if !loops.isEmpty {
                        WSectionLabel(strings.find.openLoops)
                        ForEach(loops, id: \.self) { loop in
                            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                                Text("\u{2022}").accessibilityHidden(true)
                                Text(loop).fixedSize(horizontal: false, vertical: true)
                            }
                            .wFont(.body).foregroundStyle(theme.textSecondary)
                        }
                    }
                }
                .padding(theme.space(16))
                .frame(maxWidth: .infinity, alignment: .leading)
                .wCard(radius: .card)
            }
        }
        .padding(.horizontal, theme.space(20))
        .padding(.vertical, theme.space(8))
        .frame(maxWidth: .infinity, alignment: .topLeading)
        if scrolls { ScrollView { page } } else { page }
    }
}
