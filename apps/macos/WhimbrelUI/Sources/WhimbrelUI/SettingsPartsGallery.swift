import SwiftUI

// The gallery pages for the settings parts. Made-up content only. `WSettingsPartsGallery.cases()` gives one case per
// part group and variant (light, dark, high contrast, light at 2.0 text); the gallery executable renders each to a PNG
// with `--settings-parts`.

private struct GBlock<Content: View>: View {
    @Environment(\.wTheme) private var theme
    let title: String
    let content: Content
    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            Text(title).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.textSecondary)
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private func noop() {}

@MainActor
public enum WSettingsPartsGallery {
    static let sections: [WSettingsNavItem] = [
        WSettingsNavItem(id: "general", title: "General", icon: "gearshape"),
        WSettingsNavItem(id: "profile", title: "Profile", icon: "person.crop.circle", badge: "3"),
        WSettingsNavItem(id: "plan", title: "Plan & billing", icon: "creditcard"),
        WSettingsNavItem(id: "capture", title: "Capture & Privacy", icon: "eye.slash"),
        WSettingsNavItem(id: "meetings", title: "Meetings", icon: "calendar"),
        WSettingsNavItem(id: "ai", title: "AI", icon: "cpu"),
        WSettingsNavItem(id: "sync", title: "Sync & Devices", icon: "macbook.and.iphone"),
    ]

    // MARK: Pages

    static func nav() -> AnyView {
        AnyView(Nav())
    }

    private struct Nav: View {
        @Environment(\.wTheme) private var theme
        @Environment(\.wTextScale) private var k
        var body: some View {
            VStack(alignment: .leading, spacing: 20) {
                WFlowLayout(spacing: 24, lineSpacing: 20) {
                    GBlock("WSettingsNav, Profile selected, with a badge") {
                        WSettingsNav(items: WSettingsPartsGallery.sections, selection: "profile") { _ in }
                            .frame(width: 210 * k)
                            .background(RoundedRectangle(cornerRadius: 10).fill(theme.backgroundPage))
                            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(theme.border, lineWidth: 0.75))
                    }
                    GBlock("Keyboard focus on the list (ring on the current row)") {
                        WSettingsNav(items: WSettingsPartsGallery.sections, selection: "capture") { _ in }
                            .environment(\.wForceFocusRing, true)
                            .frame(width: 210 * k)
                            .background(RoundedRectangle(cornerRadius: 10).fill(theme.backgroundPage))
                            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(theme.border, lineWidth: 0.75))
                    }
                    VStack(alignment: .leading, spacing: 20) {
                        GBlock("WSettingsSearchResults, second result highlighted") {
                            WSettingsSearchResults(results: [
                                WSettingsSearchResult(id: "a", title: "Language", section: "General"),
                                WSettingsSearchResult(id: "b", title: "Write notes and summaries in", section: "General"),
                                WSettingsSearchResult(id: "c", title: "Live transcript during meetings", section: "Meetings"),
                            ], highlighted: 1) { _ in }
                            .frame(width: 300 * k)
                            .background(RoundedRectangle(cornerRadius: 10).fill(theme.backgroundPage))
                            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(theme.border, lineWidth: 0.75))
                        }
                        GBlock("No results") {
                            WSettingsSearchResults(results: []) { _ in }
                                .frame(width: 300 * k)
                                .background(RoundedRectangle(cornerRadius: 10).fill(theme.backgroundPage))
                                .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(theme.border, lineWidth: 0.75))
                        }
                    }
                }
            }
        }
    }

    static func toggles() -> AnyView {
        AnyView(
            VStack(alignment: .leading, spacing: 20) {
                GBlock("WSettingsCard with heading, description and a status chip. Toggle rows: on, off, checkbox, disabled with a reason, highlighted") {
                    WSettingsCard(heading: "Capture", description: "What Whimbrel keeps on this Mac.", statusText: "Memory on", statusTone: .good) {
                        WSettingsRow(label: "Show screen captures in Library", description: "Off by default. The Recent tab shows meetings only. Screen text is still recorded and searchable; this only controls whether captures appear in the Recent list.", isOn: true) { _ in }
                        WSettingsRow(label: "Keep an Obsidian vault up to date", isOn: false) { _ in }
                        WSettingsRow(label: "Alert me when someone asks me something", description: "Lines that mention you go to your Ask model.", isOn: false, isEnabled: false, disabledReason: "Add your name in Profile first.") { _ in }
                        WSettingsRow(label: "Remind me to tell people I'm recording", isOn: true, highlighted: true) { _ in }
                        WSettingsRow(label: "More devices", description: "Checkbox style, for a list of independent choices.", isOn: true, style: .checkbox) { _ in }
                        WSettingsRow(label: "Backups", isOn: false, style: .checkbox) { _ in }
                    }
                }
                GBlock("Other rows: value with chevron, button, destructive, link (external and another section)") {
                    WSettingsCard(heading: "More") {
                        WSettingsRow(label: "Language", value: "Automatic") {}
                        WSettingsRow(label: "Check for Updates", buttonTitle: "Check now") {}
                        WSettingsRow(label: "Delete everything on this Mac", description: "Removes recordings, transcripts and settings.", buttonTitle: "Delete\u{2026}", destructive: true) {}
                        WSettingsLinkRow(label: "Read the full list on the privacy page", kind: .external) {}
                        WSettingsLinkRow(label: "Open AI settings", description: "Where your provider and keys live.", kind: .section) {}
                        WSettingsLinkRow(label: "Manage subscription", kind: .external, isEnabled: false, disabledReason: "Sign in to sync first.") {}
                    }
                }
            }
        )
    }

    private struct Controls: View {
        @State private var days = 30
        var body: some View {
            VStack(alignment: .leading, spacing: 20) {
                GBlock("Picker rows: menu, segmented (3), segmented over four falls back to a menu, no options, disabled with a reason") {
                    WSettingsCard(heading: "Pickers") {
                        WSettingsPickerRow(label: "Language", options: [
                            WSettingsOption(id: "auto", title: "Automatic"), WSettingsOption(id: "en", title: "English"), WSettingsOption(id: "es", title: "Espa\u{00F1}ol"),
                        ], selection: "auto") { _ in }
                        WSettingsPickerRow(label: "Quality", description: "The best engine this computer runs well.", options: [
                            WSettingsOption(id: "auto", title: "Automatic"), WSettingsOption(id: "better", title: "Better"), WSettingsOption(id: "fast", title: "Fast"),
                        ], selection: "better", style: .segmented) { _ in }
                        WSettingsPickerRow(label: "Default provider", options: ["Anthropic", "OpenAI", "Google", "OpenRouter", "Ollama"].map { WSettingsOption(id: $0, title: $0) }, selection: "Google", style: .segmented) { _ in }
                        WSettingsPickerRow(label: "Default model", options: [WSettingsOption<String>](), selection: nil, noOptionsText: "No models loaded. Save a key above, then refresh models.") { _ in }
                        WSettingsPickerRow(label: "Appearance", options: [
                            WSettingsOption(id: "system", title: "System"), WSettingsOption(id: "light", title: "Light"), WSettingsOption(id: "dark", title: "Dark"),
                        ], selection: "light", style: .segmented, isEnabled: false, disabledReason: "The skin you picked sets this.") { _ in }
                    }
                }
                GBlock("Number rows: stepper with a unit, companion checkbox off and on (the 30 stays), whole-number step 5") {
                    WSettingsCard(heading: "Retention") {
                        WSettingsNumberRow(label: "Meetings", value: days, range: 1...3650, unit: { "\($0) days" }, companionTitle: "Keep forever", companionOn: false, onCompanionChange: { _ in }) { days = $0 }
                        WSettingsNumberRow(label: "Conversations", value: 30, range: 1...3650, unit: { "\($0) days" }, companionTitle: "Keep forever", companionOn: true, onCompanionChange: { _ in }) { _ in }
                        WSettingsNumberRow(label: "Prepare a brief before each meeting", value: 15, range: 5...60, step: 5, unit: { "\($0) min" }) { _ in }
                    }
                }
                GBlock("Number fields: money with blank allowed, whole number with an error shown, a time, a text field") {
                    WSettingsCard(heading: "Fields") {
                        WSettingsNumberFieldRow(label: "Monthly budget", value: 12.5, rule: WNumberRule(minimum: 0, maximum: 100000, allowsEmpty: true, fractionDigits: 2), prefix: "$", placeholder: "None") { _ in }
                        WSettingsNumberFieldRow(label: "Daily cap per identity", description: "Leave blank to use the built-in default. 0 turns the cap off.", value: nil, rule: WNumberRule(minimum: 0, maximum: 1_000_000, allowsEmpty: true), suffix: "cents", placeholder: "default") { _ in }
                        WSettingsNumberFieldRow(label: "Daily cap, global", value: 500, rule: WNumberRule(minimum: 0, maximum: 1_000_000, allowsEmpty: true), suffix: "cents", placeholder: "default", initialText: "-5") { _ in }
                        WSettingsTextFieldRow(label: "Runs daily at", text: .constant("08:30"), fieldWidth: 100, monospaced: true) 
                        WSettingsTextFieldRow(label: "Name", description: "Shown on the meeting recording notice.", text: .constant("Alex Rivera"))
                        WSettingsTextFieldRow(label: "Run time", text: .constant("25:99"), fieldWidth: 100, monospaced: true, error: "Use a time such as 08:30.")
                        WSettingsTextEditorRow(label: "About me and my work", text: .constant("I lead product at a small team. Summaries should name decisions first."), footer: "68 of 600 characters sent")
                    }
                }
            }
        }
    }

    static func controls() -> AnyView { AnyView(Controls()) }

    static func secrets() -> AnyView {
        AnyView(
            VStack(alignment: .leading, spacing: 20) {
                GBlock("Secret rows: empty, saved (masked, nothing to reveal), replacing a saved key, and a failed save") {
                    WSettingsCard(heading: "API keys") {
                        WSettingsSecretRow(label: "Anthropic API key", description: "Stored in the macOS Keychain, never in a file.", isSaved: false, placeholder: "sk-ant-\u{2026}", formatHint: "Starts with sk-ant-", status: .info("Not configured."), onSave: { _ in true }, onRemove: {})
                        WSettingsSecretRow(label: "OpenAI API key", isSaved: true, status: .good("Saved. The key works."), onSave: { _ in true }, onRemove: {})
                        WSettingsSecretRow(label: "OpenRouter API key", isSaved: true, placeholder: "sk-or-\u{2026}", initiallyReplacing: true, onSave: { _ in true }, onRemove: {})
                        WSettingsSecretRow(label: "Google API key", isSaved: false, status: .problem("Failed to save key."), error: "That key was refused. Check it and try again.", onSave: { _ in true }, onRemove: {})
                        WSettingsSecretRow(label: "TypeSafe API key", isSaved: false, isEnabled: false, disabledReason: "Local-only mode blocks this.", onSave: { _ in true }, onRemove: {})
                    }
                }
                GBlock("A pair under one Save: a client id above the secret") {
                    WSettingsCard(heading: "Use your own Google Cloud project") {
                        WSettingsSecretRow(label: "Client secret", description: "For Gmail reading. Save sets both values.", isSaved: false, canSave: true, onSave: { _ in true }, onRemove: {}) {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("Client ID").wFont(.bodyStrong)
                                WTextBox(label: "Client ID", placeholder: "Client ID", text: .constant("1234-example.apps.example.com"))
                            }
                        }
                    }
                }
            }
        )
    }

    private struct ListPage: View {
        @State private var search = ""
        @State private var newSite = "Example.com"
        @State private var editText = "Prefers short summaries"
        var body: some View {
            VStack(alignment: .leading, spacing: 20) {
                GBlock("Lists: chips, notes, trailing actions, a toggle, an inline edit, an inline two-step confirm") {
                    WSettingsCard(heading: "Accounts and facts") {
                        WSettingsList(label: "Google Calendar accounts", items: [
                            WSettingsListItem(id: "1", title: "alex@example.com", caption: "12 events found in the next 7 days.", chip: "Default", chipTone: .good, actions: [WSettingsListAction("Remove", kind: .destructive, run: noop)]),
                            WSettingsListItem(id: "2", title: "alex.rivera@example.org", caption: "Connected.", note: "Reconnect needed. Add the account again and sign in as this address.", noteTone: .warning, actions: [WSettingsListAction("Make Default", run: noop), WSettingsListAction("Remove", kind: .destructive, run: noop)]),
                            WSettingsListItem(id: "3", title: "Prefers short summaries", chip: "Learned", edit: WSettingsListEdit(text: $editText, onSave: noop, onCancel: noop)),
                            WSettingsListItem(id: "4", title: "Zoom", caption: "us.example.zoom", toggle: (true, { _ in })),
                            WSettingsListItem(id: "5", title: "Old laptop", caption: "Last ran Nov 18", inline: AnyView(WInlineConfirm(question: "Revoke this device?", confirmTitle: "Revoke", onConfirm: noop, onCancel: noop))),
                        ])
                    }
                }
                GBlock("List with a search field and an add field (the host holds the typed text), and an empty list") {
                    WSettingsCard(heading: "Sites never captured") {
                        WSettingsList(label: "Excluded sites", description: "Pages on these sites are skipped before any text is read.", items: [
                            WSettingsListItem(id: "a", title: "example.com", actions: [WSettingsListAction("Remove", spokenTitle: "Remove example.com", run: noop)]),
                            WSettingsListItem(id: "b", title: "mail.example.org", actions: [WSettingsListAction("Remove", spokenTitle: "Remove mail.example.org", run: noop)]),
                        ], search: $search, searchPlaceholder: "Search excluded sites", add: WSettingsListAdd(label: "Add a site", placeholder: "example.com", text: $newSite, onAdd: { _ in }))
                        WSettingsList(label: "Names I go by", items: [], emptyText: "No names yet. Add the names people use for you.", add: WSettingsListAdd(label: "Add a name", placeholder: "Add a name", text: .constant(""), onAdd: { _ in }))
                    }
                }
            }
        }
    }

    static func lists() -> AnyView { AnyView(ListPage()) }

    private struct ValuesPage: View {
        @State private var open = true
        var body: some View {
            VStack(alignment: .leading, spacing: 20) {
                GBlock("Value list (path cut in the middle, full value on hover and for VoiceOver, Copy button) and progress rows") {
                    WSettingsCard(heading: "This Mac") {
                        WSettingsValueList(items: [
                            WSettingsValueItem(label: "App version", value: "0.9.82"),
                            WSettingsValueItem(label: "Data folder", value: "/Users/alex/Library/Application Support/Whimbrel/Data/Memory/Archive", copyable: true, truncation: .middle(limit: 44)),
                            WSettingsValueItem(label: "Model file", value: "File not found", isProblem: true),
                            WSettingsValueItem(label: "Meetings", value: "184"),
                        ])
                        WSettingsProgressRow(label: "Storage", valueText: "6.2 GB of 18 GB on this Mac", fraction: 0.34)
                        WSettingsProgressRow(label: "Downloading the better model", valueText: "412 of 574 MB", fraction: 0.72, cancelTitle: "Cancel", onCancel: {})
                        WSettingsProgressRow(label: "Sorting your items into topics", valueText: "Working", fraction: nil, cancelTitle: "Cancel", onCancel: {})
                    }
                }
                GBlock("Disclosure (collapsed and expanded) and shortcut rows (set, recording, empty, refused)") {
                    WSettingsCard(heading: "Advanced") {
                        WSettingsDisclosure("Advanced", description: "Use your own Google project.", isExpanded: .constant(false)) { EmptyView() }
                        WSettingsDisclosure("Advanced", isExpanded: .constant(true)) {
                            WSettingsRow(label: "Use your own Google Cloud project", description: "For Gmail reading.", buttonTitle: "Set up") {}
                        }
                        WSettingsShortcutRow(label: "Ask about this window", description: "Press this anywhere.", keys: ["\u{2303}", "\u{2325}", "Space"], onRecord: {}, onStopRecording: {}, onClear: {}, onReset: {}, resetEnabled: false)
                        WSettingsShortcutRow(label: "Ask about this window", keys: [], isRecording: true, onRecord: {}, onStopRecording: {}, onClear: {}, onReset: {})
                        WSettingsShortcutRow(label: "Pause memory", keys: [], onRecord: {}, onStopRecording: {}, onClear: {})
                        WSettingsShortcutRow(label: "Quick note", keys: ["A"], error: "A has no modifier key, so it would block normal typing. Include Control or Option.", onRecord: {}, onStopRecording: {}, onReset: {})
                    }
                }
            }
        }
    }

    static func values() -> AnyView { AnyView(ValuesPage()) }

    static func chips() -> AnyView {
        AnyView(
            VStack(alignment: .leading, spacing: 20) {
                GBlock("WStatusChip: neutral, good, warning, problem") {
                    WFlowLayout(spacing: 10, lineSpacing: 10) {
                        WStatusChip("Default")
                        WStatusChip("Encrypted", tone: .good)
                        WStatusChip("Needs attention", tone: .warning)
                        WStatusChip("Not encrypted", tone: .problem)
                        WStatusChip("Beta coming soon", glyph: "hourglass")
                    }
                }
                GBlock("WNoticeBox: info, warning, problem, with actions") {
                    VStack(spacing: 10) {
                        WNoticeBox(tone: .info, text: "Whimbrel works without any AI model. Capture, transcripts and search still work. Chat and summaries need one.", actions: [WNoticeAction("Install Ollama", run: noop), WNoticeAction("Get a free Gemini key", run: noop)])
                        WNoticeBox(tone: .warning, title: "A plain backup is still on this Mac", text: "The unencrypted copy from before encryption was turned on is still there.", actions: [WNoticeAction("Delete the unencrypted copy", kind: .destructive, run: noop)])
                        WNoticeBox(tone: .problem, title: "Could not update the vault", text: "Check that the folder exists and Whimbrel can write to it.")
                    }
                }
                GBlock("Inline lines: error, warning, success, status") {
                    VStack(alignment: .leading, spacing: 6) {
                        WErrorLine(text: "That doesn't look like a site. Enter a domain such as example.com.")
                        WWarningLine(text: "Credit is running low.")
                        WSuccessLine(text: "Exported 12 meetings: 48 files, 3.1 MB.")
                        WStatusText("Last updated 10:42 AM")
                    }
                }
            }
        )
    }

    private struct SheetPage: View {
        var body: some View {
            VStack(alignment: .leading, spacing: 20) {
                WFlowLayout(spacing: 24, lineSpacing: 20) {
                    GBlock("WSheet, destructive, typed word not matched (confirm is off)") {
                        WSheet(
                            title: "Delete everything on this Mac?", message: "This removes 184 meetings, 2,310 screen captures and your settings. It cannot be undone.",
                            width: 420, checkbox: WSheetCheckbox("Also delete my sync account", isOn: true, note: "The encrypted copy on the sync server is deleted and every device is signed out.", noteTone: .warning) { _ in },
                            typedConfirm: WSheetTypedConfirm(word: "DELETE", text: .constant("DELET")),
                            cancel: WSheetButton("Cancel", run: noop), confirm: WSheetButton("Delete and restart", kind: .destructive, run: noop)
                        ) { EmptyView() }
                    }
                    GBlock("Same sheet, word matched (confirm is on, still not the Return key)") {
                        WSheet(
                            title: "Delete everything on this Mac?", message: "This removes 184 meetings, 2,310 screen captures and your settings. It cannot be undone.",
                            width: 420, typedConfirm: WSheetTypedConfirm(word: "DELETE", text: .constant("DELETE")), error: "Could not remove the media folder. Quit and remove it by hand.",
                            cancel: WSheetButton("Cancel", run: noop), confirm: WSheetButton("Delete and restart", kind: .destructive, run: noop)
                        ) { EmptyView() }
                    }
                }
                WFlowLayout(spacing: 24, lineSpacing: 20) {
                    GBlock("WSheet with fields, primary confirm") {
                        WSheet(title: "Add topic", width: 420, cancel: WSheetButton("Cancel", run: noop), confirm: WSheetButton("Save", kind: .primary, run: noop)) {
                            VStack(alignment: .leading, spacing: 10) {
                                WTextBox(label: "Name", placeholder: "Name", text: .constant("Northwind"))
                                WTextBox(label: "Aliases", placeholder: "Aliases, separated by commas", text: .constant(""))
                                WSettingsPickerRow(label: "Kind", options: ["None", "Customer", "Product"].map { WSettingsOption(id: $0, title: $0) }, selection: "Customer") { _ in }
                            }
                        }
                    }
                    VStack(alignment: .leading, spacing: 20) {
                        GBlock("WConfirm, alert style") {
                            WConfirm(title: "Revoke this device?", message: "This device will lose access immediately. It would need to sign in again and be approved.", confirmTitle: "Revoke", onConfirm: noop, onCancel: noop)
                        }
                        GBlock("WInlineConfirm, in place of a row's button") {
                            WInlineConfirm(question: "Delete this skin?", confirmTitle: "Delete", onConfirm: noop, onCancel: noop)
                        }
                        GBlock("Keyboard focus on Cancel (the safe button)") {
                            WInlineConfirm(question: "Remove this client?", confirmTitle: "Remove", onConfirm: noop, onCancel: noop)
                                .environment(\.wForceFocusRing, true)
                        }
                    }
                }
            }
        }
    }

    static func sheets() -> AnyView { AnyView(SheetPage()) }

    static func skins() -> AnyView {
        func swatch(_ top: UInt32, _ mid: UInt32, _ low: UInt32) -> some View {
            VStack(spacing: 0) {
                Rectangle().fill(Color(wHex: top)).frame(height: 40)
                Rectangle().fill(Color(wHex: mid)).frame(height: 22)
                Rectangle().fill(Color(wHex: low)).frame(height: 22)
            }
        }
        let palettes = [
            WSkinPalette(id: "a", name: "Dawn", colors: [0x4F46E5, 0xE0E7FF]),
            WSkinPalette(id: "b", name: "Moss", colors: [0x2F6B4F, 0xDCEFE3]),
            WSkinPalette(id: "c", name: "Ember", colors: [0xB4461E, 0xFBE3D6]),
        ]
        return AnyView(
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 230), spacing: 14, alignment: .top)], alignment: .leading, spacing: 14) {
                WSkinCard(name: "Thread", tagline: "Quiet lines and warm paper.", isSelected: true, palettes: palettes, selectedPalette: "a", onSelect: {}) { swatch(0xE0E7FF, 0xFFFFFF, 0xF1F5F9) }
                WSkinCard(name: "Plain", tagline: "The standard look.", palettes: [], onSelect: {}) { swatch(0xF1F5F9, 0xFFFFFF, 0xE2E8F0) }
                WSkinCard(name: "Harbor", tagline: "Made with the skin chat.", isYours: true, palettes: palettes, selectedPalette: nil, onSelect: {}, preview: { swatch(0x1E293B, 0x334155, 0x475569) }) {
                    WOverflowMenu { Button("Rename\u{2026}") {}; Button("Export\u{2026}") {}; Button("Delete\u{2026}", role: .destructive) {} }
                }
                WSkinCard(name: "Moss and a long name that has to wrap to two lines", tagline: "A selected skin of your own.", isSelected: true, isYours: true, palettes: palettes, selectedPalette: "b", onSelect: {}, preview: { swatch(0xDCEFE3, 0xFFFFFF, 0xEEF6F0) }) {
                    WOverflowMenu { Button("Rename\u{2026}") {} }
                }
            }
        )
    }

    // MARK: Cases

    struct Page {
        let id: String
        let build: @MainActor () -> AnyView
        let width: CGFloat
    }

    static let pages: [Page] = [
        Page(id: "nav", build: nav, width: 940),
        Page(id: "toggles-links", build: toggles, width: 760),
        Page(id: "pickers-numbers-fields", build: controls, width: 760),
        Page(id: "secrets", build: secrets, width: 760),
        Page(id: "lists", build: lists, width: 760),
        Page(id: "values-progress-disclosure-shortcut", build: values, width: 760),
        Page(id: "chips-notices-lines", build: chips, width: 760),
        Page(id: "sheets-confirms", build: sheets, width: 940),
        Page(id: "skin-cards", build: skins, width: 760),
    ]

    static let variants: [(suffix: String, mode: WGalleryMode, scale: CGFloat)] = [
        ("light", .light, 1), ("dark", .dark, 1), ("high-contrast", .highContrastLight, 1), ("light-2.0", .light, 2),
    ]

    public static func cases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        for page in pages {
            for v in variants {
                // At 2.0 text the pages are rendered wider so rows and sheets keep their shape; the width is part of the case.
                let width = v.scale > 1 ? page.width + 240 : page.width
                out.append(WHomeGalleryCase(id: "settings-parts-\(page.id)-\(v.suffix)", mode: v.mode, textScale: v.scale, width: width, height: 100, view: page.build()))
            }
        }
        return out
    }
}
