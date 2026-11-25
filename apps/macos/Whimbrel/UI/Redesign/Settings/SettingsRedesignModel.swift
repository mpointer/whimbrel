import SwiftUI
import WhimbrelUI

/// The pure parts of the redesigned Settings window: which sections it lists, what they are called, what search finds
/// and how the window scales with text size. No views and no app state, so the tests can run them.
enum SettingsRedesign {
    /// The section's name in the redesigned window. "Capture & Privacy" is "Memory & Privacy" (decision 1); every other
    /// name is the one the classic window uses.
    static func title(_ section: SettingsSection) -> String {
        switch section {
        case .capture: return String(localized: "Memory & Privacy", comment: "Name of the Settings section for what Whimbrel remembers and keeps private. Settings search still finds it by the word capture.")
        default: return section.title
        }
    }

    /// The section list: the classic window's sections in the same order, Diagnostics only where it was already listed.
    static func navItems(sections: [SettingsSection] = SettingsSection.visibleCases, pendingSuggestions: Int) -> [WSettingsNavItem] {
        sections.map { section in
            WSettingsNavItem(
                id: section.rawValue, title: title(section), icon: section.systemImage,
                badge: section == .profile && pendingSuggestions > 0 ? String(pendingSuggestions) : nil)
        }
    }

    /// What search finds: exactly what the classic window's search finds (`SettingsSearch.results`), each with the
    /// section's name as the redesigned window shows it.
    static func results(for query: String) -> [WSettingsSearchResult] {
        entries(for: query).map { WSettingsSearchResult(id: $0.id, title: $0.title, section: title($0.section)) }
    }

    /// The classic search's entries, then the Memory & Privacy entries the section's new name also matches ("memory").
    /// Only this window uses it, so the classic window's results do not change.
    static func entries(for query: String, entries all: [SettingsSearchEntry] = SettingsSearch.entries, captureTitle: String = title(.capture)) -> [SettingsSearchEntry] {
        let classic = SettingsSearch.results(for: query)
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return classic }
        let extra = all.filter { entry in
            entry.section == .capture && !classic.contains(entry) && matchesSectionName(q, name: captureTitle)
        }
        return classic + extra
    }

    /// True when every word of `query` is a whole word of the section's name or the word "memory". Typing "m" or "&" is not a
    /// word of either, so it does not pull in every entry of the section; "memory" and "memory privacy" do. Words of the name
    /// shorter than 3 characters ("e" in "Memória e privacidade", "&", "de") never count, so a letter typed alone finds nothing
    /// in any language.
    static func matchesSectionName(_ query: String, name: String, extraKeywords: [String] = ["memory"]) -> Bool {
        func words(_ text: String) -> [String] {
            text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
                .split(whereSeparator: { !$0.isLetter && !$0.isNumber }).map(String.init)
        }
        let wanted = words(query)
        guard !wanted.isEmpty else { return false }
        let known = Set((words(name) + extraKeywords.flatMap(words)).filter { $0.count >= 3 })
        return wanted.allSatisfy(known.contains)
    }

    /// The section a search result opens.
    static func section(forResult id: String) -> SettingsSection? {
        SettingsSearch.entries.first { $0.id == id }?.section
    }

    /// The anchor ids a redrawn section applies, or nil while the section still hosts its classic panes (those carry the
    /// ids themselves). A redrawn section must declare every id its registry entries have.
    static func declaredAnchorIDs(for section: SettingsSection) -> [String]? {
        switch section {
        case .general: return SettingsAnchorID.general.all
        case .profile: return SettingsAnchorID.profile.all
        case .plan: return SettingsAnchorID.plan.all
        case .capture: return SettingsAnchorID.memoryPrivacy.all
        case .meetings: return SettingsAnchorID.meetings.all
        case .ai: return SettingsAnchorID.ai.all
        case .topics: return SettingsAnchorID.topics.all
        case .devices: return SettingsAnchorID.sync.all
        case .connectors: return SettingsAnchorID.connectors.all
        // Routines have no search entries, so there is nothing to declare.
        case .automations: return []
        default: return nil
        }
    }

    static func hostsClassicPanes(_ section: SettingsSection) -> Bool { declaredAnchorIDs(for: section) == nil }

    /// How much larger than normal the window's parts draw at a given text size. The macOS text size setting reaches the
    /// window as a SwiftUI dynamic type size; the parts scale with this number.
    static func textScale(for size: DynamicTypeSize) -> CGFloat {
        switch size {
        case .xSmall, .small, .medium, .large: return 1
        case .xLarge: return 1.1
        case .xxLarge: return 1.25
        case .xxxLarge: return 1.4
        case .accessibility1: return 1.6
        case .accessibility2: return 1.8
        case .accessibility3: return 2
        case .accessibility4: return 2.3
        case .accessibility5: return 2.6
        @unknown default: return 1
        }
    }
}

/// Gives a settings view the app's theme (skin and palette), its words, the settings parts' words and the text scale.
private struct SettingsRedesignThemed: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.dynamicTypeSize) private var typeSize

    func body(content: Content) -> some View {
        var strings = WhimbrelUIStrings.make()
        strings.settings = SettingsStringsRedesign.make()
        return content
            .environment(\.wTheme, RedesignBridge.theme(colorScheme: colorScheme, contrast: contrast))
            .environment(\.wStrings, strings)
            .environment(\.wTextScale, SettingsRedesign.textScale(for: typeSize))
    }
}

extension View {
    func settingsRedesignThemed() -> some View { modifier(SettingsRedesignThemed()) }
}
