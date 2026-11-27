import SwiftUI

// The Settings window's chrome: the search field, the section list (or, when the window is narrow or the text large,
// a labelled picker), the search results and the scrolling detail column. The window owns no settings. The app gives
// it the sections, the search results and the detail content, and answers a chosen result.

/// When the section list gives way to a picker above the content.
public enum WSettingsWindowLayout {
    /// The section column's width at 100 percent text.
    public static let navWidth: CGFloat = 200
    /// The least width the detail column keeps before the list is replaced by a picker.
    public static let minimumContent: CGFloat = 440

    /// From this text scale on, the list is replaced by the picker whatever the width.
    public static let largeTextScale: CGFloat = 1.5

    /// True when the text is large, or when the section list would leave the detail column under `minimumContent`
    /// points. The list grows with the text size, so large text reaches the second limit sooner than a narrow window.
    public static func usesPicker(width: CGFloat, textScale: CGFloat) -> Bool {
        textScale >= largeTextScale || width - navWidth * max(textScale, 1) < minimumContent
    }
}

/// The detail column's padding and widest content, in points.
enum WSettingsWindowMetrics {
    static let maxContent: CGFloat = 760
}

public struct WSettingsWindow<Detail: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let items: [WSettingsNavItem]
    let selection: String?
    let onSelect: (String) -> Void
    let query: Binding<String>
    let searchPlaceholder: String
    let results: [WSettingsSearchResult]
    let onOpenResult: (String) -> Void
    let pickerLabel: String
    let scrollTarget: Binding<String?>
    let onScrolled: (String) -> Void
    let onMissing: (String) -> Void
    let hint: String?
    let restoreScroll: () -> String?
    let detail: Detail
    /// Which anchored rows are drawn; the rows report here through the environment.
    @State private var presence = WAnchorPresence()

    /// - results: what the search finds for the current query (empty when there is no query, or nothing matches).
    /// - scrollTarget: set by the app to an anchor id after it has opened that result's section; the window scrolls
    ///   there after the section has replaced the old one, calls `onScrolled` (the app flashes the row) and clears it,
    ///   whether the detail column was already on screen or appeared because of the choice.
    /// - onMissing: called instead of `onScrolled` when the row never appeared (it is not drawn in this state): the section is
    ///   open and there is nothing to scroll to or mark. The app says so through `hint`.
    /// - hint: a short line under the search field (nil: none).
    /// - restoreScroll: asked once when the window appears: an anchor id to put back in view (a skin click rebuilds the
    ///   window and the app wants the same card in view again).
    public init(
        items: [WSettingsNavItem], selection: String?, onSelect: @escaping (String) -> Void,
        query: Binding<String>, searchPlaceholder: String, results: [WSettingsSearchResult],
        onOpenResult: @escaping (String) -> Void, pickerLabel: String, scrollTarget: Binding<String?>,
        onScrolled: @escaping (String) -> Void = { _ in }, onMissing: @escaping (String) -> Void = { _ in }, hint: String? = nil,
        restoreScroll: @escaping () -> String? = { nil },
        @ViewBuilder detail: () -> Detail
    ) {
        self.items = items
        self.selection = selection
        self.onSelect = onSelect
        self.query = query
        self.searchPlaceholder = searchPlaceholder
        self.results = results
        self.onOpenResult = onOpenResult
        self.pickerLabel = pickerLabel
        self.scrollTarget = scrollTarget
        self.onScrolled = onScrolled
        self.onMissing = onMissing
        self.hint = hint
        self.restoreScroll = restoreScroll
        self.detail = detail()
    }

    private var isSearching: Bool { !query.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    public var body: some View {
        GeometryReader { geo in
            Group {
                if WSettingsWindowLayout.usesPicker(width: geo.size.width, textScale: k) {
                    VStack(spacing: 0) {
                        VStack(alignment: .leading, spacing: theme.space(10)) {
                            search
                            if !isSearching { picker }
                        }
                        .padding(theme.space(12))
                        WHairline()
                        if isSearching { resultsColumn } else { detailColumn }
                    }
                } else {
                    HStack(spacing: 0) {
                        VStack(spacing: 0) {
                            search.padding(theme.space(12))
                            ScrollView {
                                if isSearching { resultsList } else { WSettingsNav(items: items, selection: selection, onSelect: onSelect) }
                            }
                        }
                        .frame(width: WSettingsWindowLayout.navWidth * k)
                        .background(theme.backgroundCard)
                        Rectangle().fill(theme.border).frame(width: max(theme.borderWidth, 0.5))
                        detailColumn
                    }
                }
            }
            .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
        }
        .wPageBackground()
        .environment(\.wDisabledButtonLook, true)
    }

    private var search: some View {
        // Return in the field opens the first result, as in every other search field.
        VStack(alignment: .leading, spacing: theme.space(6)) {
            WSearchBar(text: query, placeholder: searchPlaceholder, shortcutHint: nil, onSubmit: {
                if let first = WSettingsHandoff.firstResult(results) { onOpenResult(first.id) }
            })
            if let hint {
                Text(hint).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.updatesFrequently)
            }
        }
    }

    private var resultsList: some View {
        WSettingsSearchResults(results: results, onOpen: onOpenResult)
    }

    private var resultsColumn: some View {
        ScrollView { resultsList.padding(theme.space(4)) }
    }

    private var picker: some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            Text(pickerLabel).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.textSecondary)
            WMenuControl(
                label: pickerLabel, options: items.map { WSegmentOption(id: $0.id, title: $0.title) },
                selection: selection, placeholder: items.first { $0.id == selection }?.title ?? pickerLabel, onSelect: onSelect
            )
        }
    }

    private var detailColumn: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: theme.space(16)) { detail }
                    .environment(\.wAnchorPresence, presence)
                    .padding(theme.space(20))
                    .frame(maxWidth: WSettingsWindowMetrics.maxContent * k, alignment: .leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .task {
                if let id = restoreScroll() { proxy.scrollTo(id, anchor: .center) }
            }
            // `.task(id:)` runs when the column appears as well as when the target changes. In the picker layout the column is
            // not mounted while the search results are shown, so a target set by choosing a result is already there when the
            // column appears and `onChange` would never fire. The task also clears the target when it is done, so choosing the
            // same result twice is two hand-offs; a newer choice cancels this task and starts its own.
            .task(id: scrollTarget.wrappedValue) {
                guard let id = WSettingsHandoff.ready(target: scrollTarget.wrappedValue, detailMounted: true) else { return }
                // Let the chosen section's content replace the old one first.
                try? await Task.sleep(nanoseconds: 150_000_000)
                guard !Task.isCancelled else { return }
                // A row that exists only in some states is not drawn in this one: say so instead of scrolling to nothing.
                guard await presence.waitFor(id), !Task.isCancelled else {
                    if !Task.isCancelled {
                        onMissing(id)
                        scrollTarget.wrappedValue = WSettingsHandoff.afterScroll(target: scrollTarget.wrappedValue, done: id)
                    }
                    return
                }
                if reduceMotion { proxy.scrollTo(id, anchor: .top) } else { withAnimation { proxy.scrollTo(id, anchor: .top) } }
                onScrolled(id)
                // Heavy sections can still be laying out; one more nudge, unless a newer pick replaced this one.
                try? await Task.sleep(nanoseconds: 350_000_000)
                guard !Task.isCancelled else { return }
                proxy.scrollTo(id, anchor: .top)
                scrollTarget.wrappedValue = WSettingsHandoff.afterScroll(target: scrollTarget.wrappedValue, done: id)
            }
        }
    }
}
