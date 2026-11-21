#if DEBUG
import SwiftUI

/// Every design-system component in every state, for debug builds only. Open it with the launch
/// argument `-WhimbrelGallery` (and `-WhimbrelGallerySection <title>` to open at a section); `-WhimbrelGalleryTheme thread|thread-high-contrast|default` picks
/// the palette (Thread by default). It reads no account and no store.
struct WGallery: View {
    @ObservedObject private var themes = ThemeStore.shared
    @State private var done = false
    @State private var selectedSource = 3
    @State private var filter = true

    /// Thread and its high-contrast set as themes, built from the generated palette.
    static var threadThemes: [Theme] {
        func palette(_ values: [String: UInt32]) -> ThemePalette {
            Dictionary(uniqueKeysWithValues: ThemeToken.allCases.compactMap { token in values[token.rawValue].map { (token, $0) } })
        }
        return [
            Theme(id: "thread", name: "Thread", author: "Whimbrel", light: palette(WThreadPalette.light), dark: palette(WThreadPalette.dark)),
            Theme(id: "thread-high-contrast", name: "Thread High Contrast", author: "Whimbrel",
                  light: palette(WThreadPalette.highContrastLight), dark: palette(WThreadPalette.highContrastDark)),
        ]
    }

    static func installThemes() {
        let store = ThemeStore.shared
        store.install(store.themes + threadThemes)
        let arguments = ProcessInfo.processInfo.arguments
        let wanted = arguments.firstIndex(of: "-WhimbrelGalleryTheme").flatMap { arguments.indices.contains($0 + 1) ? arguments[$0 + 1] : nil }
        store.select(wanted ?? "thread")
    }

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: WTokens.Space.lg) {
                    Picker("Palette", selection: Binding(get: { themes.selectedId }, set: { themes.select($0) })) {
                        ForEach(themes.themes) { Text($0.name).tag($0.id) }
                    }
                    .pickerStyle(.menu)

                    section("Recording indicator") {
                        WRecordingIndicator(state: .recording, elapsedSeconds: 767, onPauseResume: {}, onStop: {})
                        WRecordingIndicator(state: .paused, elapsedSeconds: 767, onPauseResume: {}, onStop: {})
                        WRecordingIndicator(state: .processing, elapsedSeconds: 0)
                        WRecordingIndicator(state: .problem("Microphone not found"), elapsedSeconds: 0)
                        WRecordingIndicator(state: .off, elapsedSeconds: 0)
                        WRecordingIndicator(state: .recording, elapsedSeconds: 3767, size: .pill)
                    }
                    section("Consent banner") { WConsentBanner(onShowNotice: {}, onDone: {}) }
                    section("Search and filters") {
                        WSearchBar {}
                        HStack {
                            WFilterChip(label: "People", selected: filter) { filter.toggle() }
                            WFilterChip(label: "Apps", selected: false) {}
                            WFilterChip(label: "Dates", selected: false) {}
                        }
                    }
                    section("Thread") {
                        WSpineStop(time: "1:30 PM") {
                            WMeetingCard(variant: .upcoming, title: "Design sync", detail: "with Dana Whitfield + 3 · in 3 hr 38 min")
                        }
                        WSpineStop(time: "10:00 AM", node: .now) {
                            WMeetingCard(variant: .next, title: "Pricing review", status: "Next, in 8 minutes", detail: "Last met Tue, Nov 10. Brief ready.",
                                         attendees: ["Sam Okafor", "Priya Shah"]) {
                                HStack {
                                    Button("Open brief") {}.buttonStyle(.w(.secondary, fill: true))
                                    Button("Record") {}.buttonStyle(.w(.recording, fill: true))
                                }
                            }
                        }
                        WNowMarker(time: "9:52 AM")
                        WSpineStop(time: "22 min", node: .past, showsNode: false) {
                            WMemoryBlock(range: "9:30 to 9:52 AM", device: "MacBook Pro", items: [
                                .init(dot: .green, text: "Pricing v3.xlsx in Excel"), .init(dot: .blue, text: "1 email from Priya Shah"),
                                .init(dot: .purple, text: "#pricing in Slack"), .init(dot: .amber, text: "Renewal notes in Notes"),
                            ])
                        }
                        WSpineStop(time: "Mon", node: .past) {
                            WMeetingCard(variant: .past, title: "Northwind renewal call", detail: "Two years if the price holds. Quote due Thursday.")
                        }
                    }
                    section("Transcript") {
                        WTranscriptLine(time: "06:12", speaker: "Jo, Northwind", text: "If the price holds, we can do two years.")
                        WTranscriptLine(time: "18:25", speaker: "Jo, Northwind", text: "There are still a few open items on the questionnaire.", state: .cited)
                        WTranscriptLine(time: "12:40", text: "Let me check what that does to the margin and come back by Friday.", state: .live)
                    }
                    section("Note blocks and sources") {
                        WNoteBlock(kind: .heading, author: .you, text: "Your notes")
                        WNoteBlock(kind: .paragraph, author: .you, text: "jo ok with 2 yr term if price holds")
                        WNoteBlock(kind: .heading, author: .whimbrel, text: "Renewal terms")
                        WNoteBlock(kind: .paragraph, author: .whimbrel, text: "Northwind will renew for two years if the price stays the same.",
                                   source: 1, selected: selectedSource == 1) { selectedSource = 1 }
                        WNoteBlock(kind: .bullet, author: .whimbrel, text: "The security questionnaire still has open items.",
                                   source: 3, selected: selectedSource == 3) { selectedSource = 3 }
                        WSourceCard(number: 3, kind: .transcriptMoment, title: "Northwind renewal call", time: "18:25",
                                    quote: "Jo, Northwind: \u{201C}There are still a few open items on the questionnaire.\u{201D}", selected: true) {}
                        WSourceCard(number: 2, kind: .seenOnScreen, title: "Pricing v3.xlsx in Excel", time: "Today 9:41 AM") {}
                    }
                    section("Action items") {
                        WActionItemRow(text: "Send the two-year renewal quote to Jo.", owner: "Sam Okafor", due: "due Thu, Nov 19", done: done) { done.toggle() }
                        WActionItemRow(text: "Answer the open items on the questionnaire.", owner: "Dana Whitfield", done: true) {}
                    }
                    section("Buttons") {
                        HStack {
                            Button("Primary") {}.buttonStyle(.w(.primary))
                            Button("Secondary") {}.buttonStyle(.w(.secondary))
                            Button("Quiet") {}.buttonStyle(.w(.quiet))
                        }
                        HStack {
                            Button("Delete") {}.buttonStyle(.w(.destructive))
                            Button("Record") {}.buttonStyle(.w(.recording))
                            Button("Disabled") {}.buttonStyle(.w(.primary)).disabled(true)
                        }
                        HStack { WSpeakerChip(name: "Sam Okafor"); WSpeakerChip(name: "Priya Shah") }
                    }
                    section("States") {
                        WOfflineBanner()
                        WStateView(kind: .loading, title: "Loading").wCard()
                        WStateView(kind: .empty, title: "No meetings yet", help: "Meetings you record on your computer or this phone show up here.").wCard()
                        WStateView(kind: .error, title: "Could not load this note", help: "The note is on this phone but could not be read.", action: {}).wCard()
                    }
                }
                .padding(WTokens.Space.lg)
                .frame(maxWidth: WTokens.Size.contentMax)
                .frame(maxWidth: .infinity)
            }
            .background(Color.theme(.backgroundPage))
            .onAppear {
                // `-WhimbrelGallerySection "Thread"` opens at that section, for screenshots.
                let arguments = ProcessInfo.processInfo.arguments
                if let i = arguments.firstIndex(of: "-WhimbrelGallerySection"), arguments.indices.contains(i + 1) {
                    proxy.scrollTo(arguments[i + 1], anchor: .top)
                }
            }
            }
            .navigationTitle("Components")
            .navigationBarTitleDisplayMode(.inline)
        }
        .themed()
    }

    private func section<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: WTokens.Space.sm) {
            Text(title).wType(WTokens.TypeScale.title).foregroundStyle(Color.theme(.textPrimary)).accessibilityAddTraits(.isHeader)
            content()
        }
        .id(title)
    }
}
#endif
