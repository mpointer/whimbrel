import SwiftUI

// The gallery for the redesigned Settings window: the chrome (search, section list or picker, detail column) with
// General, Profile and Plan & billing drawn from sample models. Made-up content only. Rendered with `--settings`.
//
// What these pictures are: the package's own chrome and pane views, fed sample models. They are not the app's panes:
// the app builds the same models from its state, and the skin previews, the skin chat and the permission checklist are
// stand-ins here.

@MainActor
public enum WSettingsWindowGallery {
    // MARK: Sample data

    static let sections: [WSettingsNavItem] = [
        WSettingsNavItem(id: "general", title: "General", icon: "gearshape"),
        WSettingsNavItem(id: "profile", title: "Profile", icon: "person.crop.circle", badge: "2"),
        WSettingsNavItem(id: "plan", title: "Plan & billing", icon: "creditcard"),
        WSettingsNavItem(id: "capture", title: "Memory & Privacy", icon: "eye.slash"),
        WSettingsNavItem(id: "meetings", title: "Meetings", icon: "calendar"),
        WSettingsNavItem(id: "ai", title: "AI", icon: "cpu"),
        WSettingsNavItem(id: "topics", title: "Topics", icon: "tag"),
        WSettingsNavItem(id: "devices", title: "Sync & Devices", icon: "macbook.and.iphone"),
        WSettingsNavItem(id: "connectors", title: "Connectors", icon: "puzzlepiece.extension"),
        WSettingsNavItem(id: "automations", title: "Automations", icon: "clock.badge.checkmark"),
    ]

    static let results: [WSettingsSearchResult] = [
        WSettingsSearchResult(id: "capture.screenCaptures", title: "Show screen captures in Library", section: "Memory & Privacy"),
        WSettingsSearchResult(id: "capture.excludeApps", title: "Exclude apps from screen-text capture", section: "Memory & Privacy"),
        WSettingsSearchResult(id: "capture.yourData", title: "Your data", section: "Memory & Privacy"),
    ]

    static let anchorsGeneral = WGeneralAnchors(
        update: "general.update", language: "general.language", writeIn: "general.writeIn",
        appearance: "general.appearance", skin: "general.skin", permissions: "general.permissions")
    static let anchorsProfile = WProfileAnchors(use: "profile.use", suggest: "profile.suggest", about: "profile.about", facts: "profile.facts")
    static let anchorsPlan = WPlanAnchors(summary: "plan.summary", manage: "plan.manage", provider: "plan.provider")

    static func palette(_ id: String, _ name: String, _ a: UInt32, _ b: UInt32) -> WSkinPalette {
        WSkinPalette(id: id, name: name, colors: [a, b])
    }

    static func skins() -> WSkinGalleryModel {
        WSkinGalleryModel(cards: [
            .init(id: "whimbrel", anchorID: "skin-card-whimbrel", name: "Whimbrel", tagline: "Calm and plain.", isSelected: false),
            .init(id: "thread", anchorID: "skin-card-thread", name: "Thread", tagline: "Clear lines, quiet colour.", isSelected: true,
                  palettes: [palette("a", "Indigo", 0x4F46E5, 0xF8FAFC), palette("b", "Slate", 0x475569, 0xF1F5F9), palette("c", "Moss", 0x3F6B4F, 0xF4F7F2)],
                  selectedPalette: "a"),
            .init(id: "paper", anchorID: "skin-card-paper", name: "Paper", tagline: "Warm, like a notebook.", isSelected: false),
            .init(id: "user-harbor", anchorID: "skin-card-user-harbor", name: "Harbor", tagline: "Blue hour by the water.", isSelected: false, isYours: true),
        ])
    }

    static func skinActions() -> WSkinGalleryActions {
        WSkinGalleryActions(preview: { id in AnyView(SkinMock(seed: id)) })
    }

    static func general(update: String? = "Whimbrel is up to date (v1.0.0).", restart: Bool = false, locked: String? = nil) -> WGeneralSettingsModel {
        WGeneralSettingsModel(
            appVersion: "1.0.0", coreVersion: "0.9.0", updateStatus: update, updateBusy: false, restartAvailable: restart,
            languageOptions: [WChoice(id: "", title: "Automatic"), WChoice(id: "en", title: "English"), WChoice(id: "es", title: "Espa\u{00F1}ol"), WChoice(id: "fr", title: "Fran\u{00E7}ais")],
            languageSelection: "",
            writeInOptions: [WChoice(id: "", title: "Same as app language"), WChoice(id: "en", title: "English"), WChoice(id: "es", title: "Espa\u{00F1}ol")],
            writeInSelection: "",
            routinesNote: "Routines run on your primary device and write in that device's language.",
            appearanceOptions: [WChoice(id: "system", title: "System"), WChoice(id: "light", title: "Light"), WChoice(id: "dark", title: "Dark")],
            appearanceSelection: "system", appearanceLockedCaption: locked,
            skins: skins(), anchors: anchorsGeneral)
    }

    static func profile(facts: Bool = true, suggestions: Bool = true) -> WProfileSettingsModel {
        WProfileSettingsModel(
            enabled: true, suggestEnabled: true, name: "Sam Rivera", role: "Product lead", organization: "Northwind Studio",
            about: "I run weekly planning and customer calls for a small studio. Short summaries with the decisions first help most.",
            aboutFooter: "119 of 2000 characters sent",
            facts: facts ? [
                .init(id: "f1", text: "Prefers decisions listed before discussion", hasMeeting: true),
                .init(id: "f2", text: "Works in the Pacific time zone"),
            ] : [],
            suggestions: suggestions ? [
                .init(id: "s1", text: "Leads the Harbor redesign project", hasMeeting: true),
                .init(id: "s2", text: "Prefers email follow-ups on Fridays"),
            ] : [],
            preview: "Name: Sam Rivera\nRole: Product lead\nOrganization: Northwind Studio\nAbout: I run weekly planning and customer calls for a small studio." + (facts ? "\nFacts:\n- Prefers decisions listed before discussion\n- Works in the Pacific time zone" : ""),
            anchors: anchorsProfile)
    }

    enum PlanSample { case signedOut, free, plus }

    static func plan(_ sample: PlanSample) -> WPlanSettingsModel {
        switch sample {
        case .signedOut:
            return WPlanSettingsModel(state: .signedOut, anchors: anchorsPlan)
        case .free:
            return WPlanSettingsModel(
                state: .plan, planLine: "Free plan. Sync works across two devices.", showsManage: false, purchase: .plus, anchors: anchorsPlan)
        case .plus:
            return WPlanSettingsModel(
                state: .plan, planLine: "Plus, renews on Jan 14.", showsManage: true, purchase: .none,
                allowanceLine: "Whimbrel AI allowance: $6.40 of $10.00 left this month.", anchors: anchorsPlan)
        }
    }

    // MARK: Views

    private struct SkinMock: View {
        @Environment(\.wTheme) private var theme
        let seed: String
        var body: some View {
            let tint: Color = seed == "thread" ? theme.accentPrimary : (seed == "paper" ? Color(wHex: 0xB45309) : (seed.hasPrefix("user") ? Color(wHex: 0x0E7490) : Color(wHex: 0x64748B)))
            VStack(alignment: .leading, spacing: 5) {
                RoundedRectangle(cornerRadius: 3).fill(tint).frame(width: 60, height: 8)
                RoundedRectangle(cornerRadius: 3).fill(theme.border).frame(height: 6)
                RoundedRectangle(cornerRadius: 3).fill(theme.border).frame(width: 90, height: 6)
                Spacer()
                RoundedRectangle(cornerRadius: 4).fill(tint.opacity(0.25)).frame(height: 22)
            }
            .padding(10)
            .frame(maxWidth: .infinity)
            .aspectRatio(0.75, contentMode: .fit)
            .background(theme.backgroundPage)
            .overlay(Rectangle().strokeBorder(theme.border, lineWidth: 0.5))
        }
    }

    struct Slot: View {
        @Environment(\.wTheme) private var theme
        let title: String
        let text: String
        var body: some View {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                Text(text).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private struct Window: View {
        @State var query: String
        @State var selection: String
        let pane: AnyView
        let results: [WSettingsSearchResult]
        var body: some View {
            WSettingsWindow(
                items: WSettingsWindowGallery.sections, selection: selection, onSelect: { selection = $0 },
                query: $query, searchPlaceholder: "Search settings", results: results, onOpenResult: { _ in },
                pickerLabel: "Section", scrollTarget: .constant(nil)
            ) { pane }
        }
    }

    static func window(_ selection: String, query: String = "", results: [WSettingsSearchResult] = [], pane: AnyView) -> AnyView {
        AnyView(Window(query: query, selection: selection, pane: pane, results: results))
    }

    private static func generalPane(update: String? = "Whimbrel is up to date (v1.0.0).", restart: Bool = false, locked: String? = nil, highlighted: String? = nil) -> AnyView {
        AnyView(WGeneralSettingsView(
            model: general(update: update, restart: restart, locked: locked), actions: WGeneralSettingsActions(skins: skinActions()),
            highlighted: highlighted,
            makeSkin: AnyView(Slot(title: "Describe a skin", text: "The skin chat sits here. It is the app's own view, shown as a stand-in.")),
            permissions: AnyView(Slot(title: "Grant Whimbrel access", text: "The permission checklist sits here. It is the app's own view, shown as a stand-in."))))
    }

    private static func profilePane(facts: Bool = true, suggestions: Bool = true) -> AnyView {
        AnyView(WProfileSettingsView(model: profile(facts: facts, suggestions: suggestions), actions: WProfileSettingsActions()))
    }

    private static func planPane(_ s: PlanSample) -> AnyView {
        AnyView(WPlanSettingsView(model: plan(s), actions: WPlanSettingsActions()))
    }

    public static func cases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ mode: WGalleryMode, _ scale: CGFloat = 1, width: CGFloat = 900, height: CGFloat, _ view: AnyView) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: height, view: view))
        }
        let variants: [(String, WGalleryMode, CGFloat)] = [("light", .light, 1), ("dark", .dark, 1), ("high-contrast", .highContrastLight, 1), ("light-2.0", .light, 2)]
        for (suffix, mode, scale) in variants {
            let big: CGFloat = scale > 1 ? 1.8 : 1
            add("settings-general-\(suffix)", mode, scale, width: scale > 1 ? 1400 : 900, height: 1500 * big, window("general", pane: generalPane()))
            add("settings-profile-\(suffix)", mode, scale, width: scale > 1 ? 1400 : 900, height: 1450 * big, window("profile", pane: profilePane()))
        }
        add("settings-general-update-ready-light", .light, height: 560, window("general", pane: generalPane(update: "v1.0.1 is ready.", restart: true)))
        add("settings-general-palette-locked-light", .light, height: 560, window("general", pane: generalPane(locked: "This palette is dark only, so it always uses Dark.")))
        add("settings-general-highlight-skin-light", .light, height: 1500, window("general", pane: generalPane(highlighted: "general.skin")))
        add("settings-profile-empty-light", .light, height: 1050, window("profile", pane: profilePane(facts: false, suggestions: false)))

        add("settings-plan-signed-out-light", .light, height: 460, window("plan", pane: planPane(.signedOut)))
        add("settings-plan-free-light", .light, height: 460, window("plan", pane: planPane(.free)))
        add("settings-plan-plus-light", .light, height: 460, window("plan", pane: planPane(.plus)))
        add("settings-plan-plus-dark", .dark, height: 460, window("plan", pane: planPane(.plus)))
        add("settings-plan-free-high-contrast", .highContrastLight, height: 460, window("plan", pane: planPane(.free)))
        add("settings-plan-plus-light-2.0", .light, 2, width: 1400, height: 800, window("plan", pane: planPane(.plus)))

        add("settings-search-results-light", .light, height: 460, window("plan", query: "capture", results: results, pane: planPane(.free)))
        add("settings-search-results-dark", .dark, height: 460, window("plan", query: "capture", results: results, pane: planPane(.free)))
        add("settings-search-none-light", .light, height: 460, window("plan", query: "zzz", results: [], pane: planPane(.free)))

        add("settings-narrow-general-light", .light, width: 520, height: 1700, window("general", pane: generalPane()))
        add("settings-narrow-search-light", .light, width: 520, height: 520, window("general", query: "capture", results: results, pane: generalPane()))
        add("settings-narrow-plan-dark", .dark, width: 520, height: 520, window("plan", pane: planPane(.plus)))
        out += memoryPrivacyCases()
        out += meetingsAITopicsCases()
        out += syncConnectorsAutomationsCases()
        return out
    }
}
