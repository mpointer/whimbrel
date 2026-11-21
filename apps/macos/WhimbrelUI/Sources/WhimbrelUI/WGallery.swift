import CoreImage.CIFilterBuiltins
import SwiftUI

/// The four looks the gallery can show.
public enum WGalleryMode: String, CaseIterable, Sendable {
    case light
    case dark
    case highContrastLight = "high-contrast-light"
    case highContrastDark = "high-contrast-dark"

    public var variant: WVariant {
        switch self {
        case .light: return .light
        case .dark: return .dark
        case .highContrastLight: return .highContrastLight
        case .highContrastDark: return .highContrastDark
        }
    }

    public var title: String {
        switch self {
        case .light: return "Light"
        case .dark: return "Dark"
        case .highContrastLight: return "High contrast light"
        case .highContrastDark: return "High contrast dark"
        }
    }
}

/// Fictional sample data for the gallery.
enum WSample {
    static let alex = WPerson(name: "Alex Rivera")
    static let sam = WPerson(name: "Sam Okafor")
    static let priya = WPerson(name: "Priya Shah")
    static let dana = WPerson(name: "Dana Whitfield")
    static let jo = WPerson(name: "Jo at Northwind")
    static let elapsed = 14 * 60 + 8
}

private struct GallerySection<Content: View>: View {
    @Environment(\.wTheme) private var theme
    let title: String
    let content: Content
    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            Text(title).wFont(.title).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct GalleryLabel: View {
    @Environment(\.wTheme) private var theme
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text).wFont(.secondary).foregroundStyle(theme.textSecondary)
    }
}

/// Every component in every state on the Thread theme (or any skin passed in). Public so the app can
/// show it in debug builds.
public struct WGallery: View {
    let mode: WGalleryMode
    let textScale: CGFloat
    let shape: WShape
    let type: WTypeStyle
    let scrollable: Bool
    let width: CGFloat?

    public init(mode: WGalleryMode = .light, textScale: CGFloat = 1, shape: WShape = .thread, type: WTypeStyle = .thread, scrollable: Bool = true, width: CGFloat? = nil) {
        self.mode = mode
        self.textScale = textScale
        self.shape = shape
        self.type = type
        self.scrollable = scrollable
        self.width = width
    }

    public var body: some View {
        let theme = WTheme(palette: mode.variant.threadPalette, variant: mode.variant, shape: shape, type: type)
        Group {
            if scrollable {
                ScrollView { GalleryContent(mode: mode).padding(theme.space(28) * textScale) }
            } else {
                GalleryContent(mode: mode).padding(theme.space(28) * textScale)
            }
        }
        .frame(width: width)
        .background(theme.backgroundPage)
        .environment(\.wTheme, theme)
        .environment(\.wTextScale, textScale)
        .preferredColorScheme(mode.variant.colorScheme)
    }
}

private struct GalleryContent: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let mode: WGalleryMode

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(32) * k) {
            VStack(alignment: .leading, spacing: theme.space(4)) {
                Text("Whimbrel design system, Mac").wFont(.pageTitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                GalleryLabel("Thread theme, \(mode.title.lowercased()), text size \(Int(k * 100)) percent. Sample data is fictional.")
            }
            HStack(alignment: .top, spacing: theme.space(32) * k) {
                VStack(alignment: .leading, spacing: theme.space(32) * k) {
                    recording
                    capture
                    consent
                    buttons
                    searchAndFilters
                    chips
                    settings
                    states
                }
                VStack(alignment: .leading, spacing: theme.space(32) * k) {
                    sidebar
                    meetingCards
                    memoryBlocks
                    transcript
                    marks
                    sourceCards
                    actionItems
                    setupParts
                }
            }
            spine
            finishedNote
        }
    }

    // MARK: Sections

    private var setupParts: some View {
        GallerySection("Setup parts") {
            GalleryLabel("Step line, progress line (still and moving), field with count and error, permission rows")
            WStepLine(step: 2, total: 3)
            VStack(alignment: .leading, spacing: theme.space(10)) {
                WProgressLine(fraction: 0.6, label: "Progress")
                WProgressLine(label: "Working")
            }
            GalleryDemoField()
            ForEach([WPermissionState.allowed, .notYet, .denied], id: \.self) { state in
                WPermissionRow(WPermissionRowModel(id: "\(state)", glyph: "mic.fill", name: "Microphone", reason: WSampleCopy.micReason, state: state, onAllow: {}, onOpenSettings: {}))
            }
            GalleryLabel("Numbered words, read one by one")
            WWordList(words: Array(WSampleCopy.words.prefix(9)), label: "Sample words")
            GalleryLabel("Actions: more than three go in a menu")
            WActionBar(actions: ["Quit", "Reset", "Continue", "Put back", "Retry"].enumerated().map { WOnboardingGallery.action("a\($0.offset)", $0.element, $0.offset == 4 ? .primary : .secondary) })
        }
    }

    private var recording: some View {
        GallerySection("Recording indicator") {
            GalleryLabel("Bar: recording, paused, processing, off, problem")
            VStack(spacing: theme.space(10)) {
                WRecordingIndicator(state: .recording, elapsedSeconds: WSample.elapsed, size: .bar, onPause: {}, onStop: {})
                WRecordingIndicator(state: .paused, elapsedSeconds: WSample.elapsed, size: .bar, onResume: {}, onStop: {})
                WRecordingIndicator(state: .processing, size: .bar)
                WRecordingIndicator(state: .off, size: .bar)
                WRecordingIndicator(state: .problem("Microphone not found"), size: .bar)
            }
            GalleryLabel("Pill")
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                WRecordingIndicator(state: .recording, elapsedSeconds: WSample.elapsed, size: .pill)
                WRecordingIndicator(state: .paused, elapsedSeconds: WSample.elapsed, size: .pill)
                WRecordingIndicator(state: .processing, size: .pill)
                WRecordingIndicator(state: .off, size: .pill)
                WRecordingIndicator(state: .problem("No microphone"), size: .pill)
            }
            GalleryLabel("Menu")
            WFlowLayout(spacing: theme.space(16), lineSpacing: theme.space(6)) {
                WRecordingIndicator(state: .recording, elapsedSeconds: WSample.elapsed, size: .menu)
                WRecordingIndicator(state: .paused, elapsedSeconds: WSample.elapsed, size: .menu)
                WRecordingIndicator(state: .processing, size: .menu)
                WRecordingIndicator(state: .off, size: .menu)
                WRecordingIndicator(state: .problem("No microphone"), size: .menu)
            }
        }
    }

    private var capture: some View {
        GallerySection("Capture state") {
            WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(10)) {
                ForEach([WMemoryState.on, .pausedUntil("10:52"), .off], id: \.title) { state in
                    WCaptureState(state: state, onWhatIsCaptured: {}) {
                        Button("Pause for 15 minutes") {}
                        Button("Pause for 1 hour") {}
                        Button("Pause until tomorrow") {}
                    }
                }
            }
            GalleryLabel("The pause menu opens on click. Its choices are slots.")
        }
    }

    private var consent: some View {
        GallerySection("Consent banner") {
            WConsentBanner(isCopied: false, onCopy: {}, onDone: {})
            WConsentBanner(isCopied: true, onCopy: {}, onDone: {})
        }
    }

    private var buttons: some View {
        GallerySection("Button") {
            WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(10)) {
                WButton("Primary", kind: .primary) {}
                WButton("Secondary", kind: .secondary) {}
                WButton("Quiet", kind: .quiet) {}
                WButton("Destructive", kind: .destructive) {}
            }
            WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(10)) {
                WButton("Start notes", glyph: "pencil", shortcutHint: "\u{2318}\u{21A9}", kind: .primary) {}
                WButton("Copy notice", glyph: "doc.on.doc", kind: .secondary) {}
                WButton("Delete note", glyph: "trash", kind: .destructive) {}
                WButton("Disabled", kind: .primary) {}.disabled(true)
                WButton("Focused", kind: .secondary) {}.wSimulatedFocus(true)
            }
            GalleryLabel("The last button shows a stand-in for the system focus ring.")
        }
    }

    private var searchAndFilters: some View {
        GallerySection("Search and filters") {
            WSearchBar(text: .constant(""))
            WSearchBar(text: .constant("Northwind"))
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                WFilterChip("People", glyph: "person.2") {}
                WFilterChip("Apps", glyph: "square.grid.2x2") {}
                WFilterChip("Dates", glyph: "calendar") {}
                WFilterChip("Sam Okafor", isSelected: true, onRemove: {}) {}
                WFilterChip("Slack", showsChevron: false, isSelected: true) {}
            }
        }
    }

    private var chips: some View {
        GallerySection("Speaker chip") {
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                ForEach([WSample.sam, WSample.priya, WSample.dana, WSample.jo, WSample.alex]) { WSpeakerChip($0) }
            }
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                ForEach([WSample.sam, WSample.priya, WSample.dana, WSample.jo, WSample.alex]) { WSpeakerChip($0, size: .small) }
            }
        }
    }

    private var settings: some View {
        GallerySection("Settings row") {
            VStack(spacing: 0) {
                WSettingsRow(label: "Keep work memory on", description: "Saves context from your screen while you work.", isOn: true, onChange: { _ in })
                Divider().overlay(theme.border.opacity(0.6))
                WSettingsRow(label: "Keep work memory on", isOn: false, onChange: { _ in })
                Divider().overlay(theme.border.opacity(0.6))
                WSettingsRow(label: "Transcription language", description: "Used for new meetings.", value: "English", action: {})
                Divider().overlay(theme.border.opacity(0.6))
                WSettingsRow(label: "Export all notes", buttonTitle: "Export", action: {})
                Divider().overlay(theme.border.opacity(0.6))
                WSettingsRow(label: "Delete work memory", description: "Removes everything Whimbrel saved from your screen.", buttonTitle: "Delete", destructive: true, action: {})
            }
            .wCard(radius: .card)
        }
    }

    private var states: some View {
        GallerySection("Empty, loading and error") {
            VStack(spacing: theme.space(12)) {
                WEmptyState(glyph: "tray", title: "No meetings yet", help: "Meetings show up here when they are on your calendar.", actionTitle: "Connect calendar", action: {})
                    .wCard(radius: .card)
                WLoadingState(title: "Loading your notes")
                    .wCard(radius: .card)
                WErrorState(title: "Could not load your notes", help: "Check your connection, then try again.", onRetry: {})
                    .wCard(radius: .card)
            }
        }
    }

    private var sidebar: some View {
        GallerySection("Sidebar") {
            ZStack(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: theme.shape.surfaceRadius, style: .continuous)
                    .fill(LinearGradient(colors: [theme.selection, theme.backgroundPage, theme.highlight], startPoint: .topLeading, endPoint: .bottomTrailing))
                WSidebar {
                    WSidebarItem(icon: "clock", label: "Today", shortcut: "\u{2318}1", isSelected: true) {}
                    WSidebarItem(icon: "video", label: "Meetings", shortcut: "\u{2318}2") {}
                    WSidebarItem(icon: "books.vertical", label: "Library", badge: "3", shortcut: "\u{2318}3") {}
                    WSidebarItem(icon: "number", label: "Topics", shortcut: "\u{2318}4") {}.wSimulatedFocus(true)
                    WSidebarItem(icon: "bolt", label: "Automations", shortcut: "\u{2318}5") {}
                } footer: {
                    VStack(alignment: .leading, spacing: theme.space(8)) {
                        WCaptureState(state: .on, onWhatIsCaptured: {}) { Button("Pause for 15 minutes") {} }
                        HStack(spacing: theme.space(8)) {
                            WSpeakerChip(WSample.alex, size: .small)
                        }
                    }
                    .padding(.horizontal, theme.space(4))
                }
                .padding(theme.space(14))
            }
            .frame(height: 400 * k)
            GalleryLabel("Items: selected, plain, with a badge, and focused (stand-in ring). Glass, or a material before macOS 26.")
        }
    }

    private var meetingCards: some View {
        GallerySection("Meeting card") {
            WMeetingCard(variant: .next, time: "Today, 10:00 AM. Last met Tue, Nov 10.", title: "Pricing review", attendees: [WSample.sam, WSample.priya, WSample.alex], eyebrow: "Next, in 8 minutes", primaryTitle: "Start notes", primaryGlyph: "pencil", primaryHint: "\u{2318}\u{21A9}", onPrimary: {}, secondaryTitle: "Open notes from Nov 10", onSecondary: {})
            WMeetingCard(variant: .upcoming, time: "3:00 PM", title: "Hiring debrief", attendees: [WSample.priya], eyebrow: "in 5 hr 8 min", primaryTitle: "Start notes", onPrimary: {})
            WMeetingCard(variant: .past, time: "Mon, Nov 16", title: "Northwind renewal call", attendees: [WSample.dana, WSample.jo], gist: "Two year renewal if the price holds. Seats grow to 55 in Q2.", primaryTitle: "Open", onPrimary: {})
        }
    }

    private var memoryBlocks: some View {
        GallerySection("Memory block") {
            WMemoryBlock(timeRange: "9:30 to 9:52 AM", items: [
                WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing v3.xlsx in Excel"),
                WMemoryItem(app: .blue, glyph: "envelope", name: "1 email from Priya Shah"),
            ])
            WMemoryBlock(timeRange: "8:40 to 9:30 AM", items: [
                WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing v3.xlsx in Excel"),
                WMemoryItem(app: .purple, glyph: "number", name: "#pricing in Slack"),
                WMemoryItem(app: .blue, glyph: "envelope", name: "2 emails with Northwind"),
                WMemoryItem(app: .amber, glyph: "doc.text", name: "Renewal terms.docx in Word"),
            ])
        }
    }

    private var transcript: some View {
        GallerySection("Transcript") {
            WTranscriptView(title: "Live transcript", accessory: "Auto-scroll on", entries: [
                WTranscriptEntry(time: "10:12", speaker: WSample.priya, text: "So a usage-based tier, on top of seats?"),
                WTranscriptEntry(time: "10:12", speaker: WSample.sam, text: "Yes. Small base, then pay for what you use."),
                WTranscriptEntry(time: "10:13", speaker: WSample.alex, text: "Do we know what that does to churn on Starter?", state: .cited),
                WTranscriptEntry(time: "10:14", speaker: WSample.sam, text: "On the annual discount, I would start at 15 and see...", state: .live),
            ])
        }
    }

    private var marks: some View {
        GallerySection("Note block and source mark") {
            VStack(alignment: .leading, spacing: theme.space(10)) {
                WNoteBlock(kind: .heading, author: .you, text: "Your notes")
                WNoteBlock(kind: .paragraph, author: .you, text: "jo ok with 2 yr term if price holds")
                WNoteBlock(kind: .bullet, author: .you, text: "seats 40 to 55 in Q2")
                WNoteBlock(kind: .check, author: .you, text: "security questionnaire?? who owns it", isChecked: false, onToggle: { _ in })
                WNoteBlock(kind: .heading, author: .whimbrel(source: nil), text: "Renewal terms")
                WNoteBlock(kind: .paragraph, author: .whimbrel(source: 1), text: "Northwind will renew for two years if the price stays the same.", onSelectSource: {})
                WNoteBlock(kind: .bullet, author: .whimbrel(source: 2), text: "Seats grow from 40 to 55 in Q2.", onSelectSource: {})
                WNoteBlock(kind: .check, author: .whimbrel(source: 4), text: "Send the two year renewal quote to Jo.", isChecked: true, onToggle: { _ in }, onSelectSource: {})
                WNoteBlock(kind: .paragraph, author: .whimbrel(source: 3), text: "The security questionnaire still has open items.", isSelected: true, onSelectSource: {})
            }
            HStack(spacing: theme.space(16)) {
                WSourceMark(number: 1) {}
                WSourceMark(number: 2, isSelected: true) {}
                WSourceMark(number: 12) {}
                GalleryLabel("Source mark: plain, selected, two digits")
            }
        }
    }

    private var sourceCards: some View {
        GallerySection("Source card") {
            WSourceCard(number: 1, size: .margin, title: "Jo, Northwind", time: "06:12", quote: "If the price holds, we can do two years.", onSelect: {}, onOpen: {})
            WSourceCard(number: 3, size: .margin, title: "Jo, Northwind", time: "18:25", quote: "There are still a few open items on the questionnaire.", openTitle: "Open in transcript at 18:25", isSelected: true, onSelect: {}, onOpen: {})
            WSourceCard(number: 2, size: .card, title: "Dana Whitfield", time: "09:40", quote: "We expect fifteen more seats by Q2. I will confirm the date.", openTitle: "Open in transcript at 09:40", onSelect: {}, onOpen: {})
            WSourceCard(number: 5, size: .card, appGlyph: "tablecells", appName: "Excel", kind: .seenOnScreen, title: "Pricing v3.xlsx", time: "9:41 AM", quote: "Starter, 40 seats, annual", openTitle: "Open", isSelected: true, onSelect: {}, onOpen: {})
        }
    }

    private var actionItems: some View {
        GallerySection("Action item") {
            VStack(alignment: .leading, spacing: theme.space(14)) {
                WActionItemRow(text: "Send the two year renewal quote to Jo.", owner: WSample.sam, due: "Due Thu, Nov 19", sourceTime: "28:50", onToggle: { _ in }, onSourceTap: {})
                WActionItemRow(text: "Answer the open items on the questionnaire.", isDone: true, owner: WSample.dana, due: "Due Fri, Nov 20", sourceTime: "29:31", onToggle: { _ in }, onSourceTap: {})
            }
            .padding(theme.space(14))
            .wCard(radius: .card)
        }
    }

    // MARK: Compositions

    private var spine: some View {
        GallerySection("Time line, composed") {
            GalleryLabel("Spine, spine stops, now marker, meeting cards and memory blocks together.")
            WSpine {
                WSpineStop(time: "3:00 PM", node: .upcoming, connector: .dashed, connectsFromAbove: false) {
                    WMeetingCard(variant: .upcoming, time: "3:00 PM", title: "Hiring debrief", attendees: [WSample.priya], eyebrow: "in 5 hr 8 min", primaryTitle: "Start notes", onPrimary: {})
                }
                WSpineStop(time: "10:00 AM", node: .now, connector: .dashed) {
                    WMeetingCard(variant: .next, time: "Today, 10:00 AM. Last met Tue, Nov 10.", title: "Pricing review", attendees: [WSample.sam, WSample.priya, WSample.alex], eyebrow: "Next, in 8 minutes", primaryTitle: "Start notes", primaryGlyph: "pencil", primaryHint: "\u{2318}\u{21A9}", onPrimary: {}, secondaryTitle: "Open notes from Nov 10", onSecondary: {})
                }
                WNowMarker(time: "9:52 AM", label: "Memory on since 8:02 AM", connector: .span)
                WSpineStop(time: "22 min", node: .past, connector: .span) {
                    WMemoryBlock(timeRange: "9:30 to 9:52 AM", items: [
                        WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing v3.xlsx in Excel"),
                        WMemoryItem(app: .blue, glyph: "envelope", name: "1 email from Priya Shah"),
                    ])
                }
                WSpineStop(time: "50 min", node: .past, connector: .span) {
                    WMemoryBlock(timeRange: "8:40 to 9:30 AM", items: [
                        WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing v3.xlsx in Excel"),
                        WMemoryItem(app: .purple, glyph: "number", name: "#pricing in Slack"),
                        WMemoryItem(app: .blue, glyph: "envelope", name: "2 emails with Northwind"),
                    ])
                }
                WSpineStop(time: "Mon, Nov 16", node: .past, connector: .none) {
                    WMeetingCard(variant: .past, time: "Mon, Nov 16", title: "Northwind renewal call", attendees: [WSample.dana, WSample.jo], gist: "Two year renewal if the price holds.", primaryTitle: "Open", onPrimary: {})
                }
            }
            .padding(.vertical, theme.space(8))
            GalleryLabel("Live: the recording node and a red connector")
            WSpine {
                WSpineStop(time: "10:14 AM", caption: "Now", node: .recording, connector: .recording, connectsFromAbove: false) {
                    VStack(spacing: theme.space(10)) {
                        WRecordingIndicator(state: .recording, elapsedSeconds: WSample.elapsed, size: .bar, onPause: {}, onStop: {})
                        WConsentBanner(onCopy: {}, onDone: {})
                    }
                }
                WSpineStop(time: "10:00 AM", caption: "Started", node: .past, connector: .none) {
                    GalleryLabel("Pricing review started")
                        .frame(minHeight: 28 * k, alignment: .leading)
                }
            }
        }
    }

    private var finishedNote: some View {
        GallerySection("Finished note, composed") {
            GalleryLabel("Your notes in the serif. Whimbrel lines in the AI ink with a rule and a numbered mark. Sources line up in the right margin.")
            HStack(alignment: .top, spacing: 0) {
                VStack(alignment: .leading, spacing: theme.space(14)) {
                    Text(theme.sectionLabel("Monday, November 16 \u{B7} 32 min")).wFont(.secondary).fontWeight(.semibold).tracking(theme.type.sectionLabelCaps ? 0.6 : 0).foregroundStyle(theme.textSecondary)
                    Text("Northwind renewal call").wFont(.readingTitle).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                    Text("Sam Okafor, Dana Whitfield, Jo at Northwind, you").wFont(.secondary).foregroundStyle(theme.textSecondary)
                    VStack(alignment: .leading, spacing: theme.space(4)) {
                        WNoteBlock(kind: .heading, author: .you, text: "Your notes")
                        WNoteBlock(author: .you, text: "jo ok with 2 yr term if price holds")
                        WNoteBlock(author: .you, text: "security questionnaire?? who owns it")
                        WNoteBlock(author: .you, text: "seats 40 to 55 in Q2")
                    }
                    WNoteBlock(kind: .heading, author: .whimbrel(source: nil), text: "Renewal terms").padding(.top, theme.space(8))
                    noteRow(source: 1, text: "Northwind will renew for two years if the price stays the same.", who: "Jo, Northwind", time: "06:12", quote: "If the price holds, we can do two years.")
                    noteRow(source: 2, text: "Seats grow from 40 to 55 in Q2. Dana will confirm the start date.", who: "Dana Whitfield", time: "09:40", quote: "We expect fifteen more seats by Q2. I will confirm the date.")
                    WNoteBlock(kind: .heading, author: .whimbrel(source: nil), text: "Risks").padding(.top, theme.space(8))
                    noteRow(source: 3, text: "Northwind's security questionnaire still has open items, and the renewal date is close.", who: "Jo, Northwind", time: "18:25", quote: "There are still a few open items on the questionnaire.", selected: true)
                    WNoteBlock(kind: .heading, author: .whimbrel(source: nil), text: "Next steps").padding(.top, theme.space(8))
                    actionRow(source: 4, text: "Send the two year renewal quote to Jo.", owner: WSample.sam, due: "Due Thu, Nov 19", who: "Sam Okafor", time: "28:50", quote: "I will get the quote to you by Thursday.")
                    actionRow(source: 5, text: "Answer the open items on the questionnaire.", owner: WSample.dana, due: "Due Fri, Nov 20", who: "Dana Whitfield", time: "29:31", quote: "I will answer the open items by Friday.")
                }
                .padding(.trailing, theme.space(24))
                .frame(maxWidth: .infinity, alignment: .leading)
                .layoutPriority(1)
            }
            .padding(theme.space(24))
            .background(RoundedRectangle(cornerRadius: theme.shape.surfaceRadius, style: .continuous).fill(theme.backgroundCard))
            .overlay(RoundedRectangle(cornerRadius: theme.shape.surfaceRadius, style: .continuous).wBorder(theme.border.opacity(theme.isHighContrast ? 1 : 0.6), style: theme.borderStroke))
        }
    }

    /// A note line with its source card beside it in the margin.
    private func noteRow(source: Int, text: String, who: String, time: String, quote: String, selected: Bool = false) -> some View {
        HStack(alignment: .top, spacing: theme.space(20)) {
            WNoteBlock(kind: .paragraph, author: .whimbrel(source: source), text: text, isSelected: selected, onSelectSource: {})
            WSourceCard(number: source, size: .margin, title: who, time: time, quote: quote, openTitle: "Open in transcript at \(time)", isSelected: selected, onSelect: {}, onOpen: {})
                .frame(width: 270 * k)
        }
    }

    private func actionRow(source: Int, text: String, owner: WPerson, due: String, who: String, time: String, quote: String) -> some View {
        HStack(alignment: .top, spacing: theme.space(20)) {
            VStack(alignment: .leading, spacing: theme.space(4)) {
                WNoteBlock(kind: .check, author: .whimbrel(source: source), text: text, onToggle: { _ in }, onSelectSource: {})
                HStack(spacing: theme.space(8)) {
                    WSpeakerChip(owner, size: .small)
                    Text(due).wFont(.secondary).foregroundStyle(theme.textSecondary)
                }
                .padding(.leading, theme.space(36))
            }
            WSourceCard(number: source, size: .margin, title: who, time: time, quote: quote, onSelect: {}, onOpen: {})
                .frame(width: 270 * k)
        }
    }
}


// MARK: Setup, first run and launch screens

/// The English the app's string catalog gives these screens, for the gallery and the offscreen render.
enum WSampleCopy {
    static let setupTitle = "Create your recovery key"
    static let setupIntro = "Whimbrel will encrypt the memory database on this Mac with a key kept in your Mac Keychain. If that key is ever lost, your recovery key is the only way back into the memory database. Make it now, before anything is encrypted."
    static let stepPassphrase = "1. Choose a passphrase"
    static let passphraseHelp = "At least 12 characters. A short sentence you will remember works well. The passphrase locks the recovery file and nothing else."
    static let passphraseField = "Passphrase"
    static let confirmField = "Type it again"
    static let passphrasesDiffer = "The two passphrases do not match."
    static let passphraseLockedHint = "To use a different passphrase, start over."
    static let startOver = "Start over…"
    static let createKey = "Create recovery key"
    static let stepFile = "2. Save the recovery file"
    static let fileHelp = "Save Whimbrel recovery key.txt somewhere other than this Mac, such as a USB drive, or attached to an entry in your password manager. It is locked with your passphrase. If you forget the passphrase, the file cannot be opened."
    static let saveFile = "Save recovery file…"
    static let sameDiskWarning = "This file is on the same disk as your memory database, so it may be lost with it. Keep a copy somewhere other than this Mac as well."
    static let stepWords = "3. Write down your 24 recovery words"
    static let wordsAreTheKey = "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline."
    static let printWords = "Print words and QR code…"
    static let printedNote = "Printed."
    static let wroteDown = "I wrote down all 24 words, in order"
    static let qrLabel = "QR code of the 24 words"
    static let stepTest = "4. Test your passphrase"
    static let testHelp = "Type your passphrase once more. Whimbrel checks it against the recovery file it just made, so you know your passphrase opens the recovery file."
    static let testButton = "Test"
    static let testPassed = "That passphrase opens your recovery file."
    static let testFailed = "That passphrase does not open the recovery file. Check it and try again."
    static let iSavedIt = "I have kept my recovery file or my 24 words somewhere other than this Mac"
    static let iSavedItLocked = "First pass the test above, and save the recovery file, print the words or tick that you wrote them down."
    static let lostBoth = "You only need the recovery key if this Mac loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key."
    static let continueDefault = "Continue"
    static let recoveryTitle = "Whimbrel can't open your memory database"
    static let recoveryKeyMissing = "This Mac has no key for the encrypted memory database. That happens after a new user account, a Keychain reset, or when the data folder came from another Mac."
    static let recoveryKeyRejected = "The key in your Mac Keychain does not open this memory database."
    static let recoveryUnchanged = "Your data has not been changed. Use your recovery key to open it."
    static let methodFile = "Recovery file"
    static let methodWords = "24 words"
    static let chooseFile = "Choose recovery file…"
    static let unlockWithFile = "Unlock with file"
    static let unlockWithWords = "Unlock with words"
    static let wordsHelp = "Type or paste your 24 words. Spaces, line breaks and capitals do not matter."
    static let keyForAnotherDatabase = "That recovery key does not open this memory database. It may belong to another Mac. Try another recovery file or set of words."
    static let recoveryLostBoth = "You only need the recovery key if this Mac loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key."
    static let recoveryNeverDeletes = "Whimbrel will not delete or replace this database. It stays on this Mac, unchanged, until you unlock it."
    static let unavailableTitle = "Whimbrel can't read its key right now"
    static let unavailableBody = "macOS did not let Whimbrel read the memory database key from your Keychain. Your key and your data are most likely still there. This is not the same as a lost key, and you do not need your recovery key for it."
    static let unavailableHint = "This can happen while your Mac is locked or just after you sign in. Unlock your Mac, then try again."
    static let unavailableTouchesNothing = "Whimbrel has not opened, changed or deleted anything."
    static let stillUnavailable = "The key still can't be read. Check that your Mac is unlocked, then try again."
    static let tryAgain = "Try again"
    static let trying = "Trying…"
    static let quitAction = "Quit Whimbrel"
    static let migratingTitle = "Encrypting the memory database"
    static let migratingBody = "This can take a few minutes on a large memory database. Keep Whimbrel open and your Mac awake until it finishes."
    static let migratingInterruptHint = "If it is interrupted, Whimbrel finishes or undoes it the next time it starts."
    static let missingTitle = "Whimbrel can't find the memory database"
    static let missingWithBackupBody = "The encrypted memory database is missing, but the unencrypted copy from before encryption is still on this Mac. Whimbrel can put that copy back and open it."
    static let missingWithBackupHint = "The copy holds your data from before encryption started. Anything added after that was in the encrypted file. If you can find the encrypted file, for example in a backup, put it back instead and try again."
    static let restoreCopyAction = "Put back the unencrypted copy…"
    static let errorTitle = "Whimbrel couldn't start"
    static let errorOpenFailed = "The memory database could not be opened. Whimbrel has not changed it."
    static let errorTryAgainHint = "Try again. If this keeps happening, quit Whimbrel and write to hello@whimbrelapp.com."
    static let continueWithoutStore = "Continue without the memory database"
    static let starting = "Starting Whimbrel…"
    static let cancel = "Cancel"
    static let wrongPassphrase = "That passphrase does not open this recovery file, or the file is damaged. Check the passphrase and try again."
    static let restored = "Unlocked. Whimbrel is opening your memory database."
    static let allowedRequired = "Required"
    static let optional = "Optional"
    static let welcome = "Welcome to Whimbrel"
    static let firstRunIntro = "Two permissions and you're ready for your first meeting. No bot joins the call, and your recordings stay on this Mac."
    static let getStarted = "Get started"
    static let changeLater = "You can change these later in Settings, General."
    static let consentRules = "Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are."
    static let anythingElse = "Anything else, Whimbrel asks for when you first use it. Its models are downloading in the background."
    static let checklistTitle = "Grant Whimbrel access"
    static let checklistIntro = "Whimbrel works with only some of these turned on. Each one adds one thing. Turn them on one at a time."
    static let micName = "Microphone"
    static let micReason = "Lets Whimbrel capture your voice during meetings, on-device, alongside system audio."
    static let screenName = "Screen & System Audio Recording"
    static let screenReason = "Lets Whimbrel record the sound your Mac plays, so it can hear the other people in a meeting. No bot joins the call."
    static let accessibilityName = "Accessibility"
    static let accessibilityReason = "Lets Whimbrel read the text of your active window to build a searchable timeline. Text only, no screenshots."
    static let calendarName = "Calendar"
    static let calendarReason = "Lets Whimbrel see events that have a meeting link or other people, so it can offer to record shortly before they start."
    static let notificationsName = "Notifications"
    static let notificationsReason = "Lets Whimbrel alert you when a detected meeting is about to start, even if the app isn't in front."
    static let refresh = "Refresh"
    static let allSet = "All set"
    static let recoveryMethod = "Recovery method"
    static let modelTitle = "Set up models"
    static let modelStatus = "Whimbrel needs a few model files for transcription and speaker detection."
    static let modelFailed = "Setup couldn't finish."
    static let modelFailedHelp = "The model files did not download. Check your connection, then try again."
    static let skipForNow = "Skip for now"
    static let downloadModels = "Download models"
    static let checkAgain = "Check again"
    static let syncTitle = "Waiting for approval on another device"
    static let syncMessage = "Open Whimbrel on a device that's already signed in, go to Settings \u{2192} Devices, and approve this one. It will be picked up automatically within a few seconds."
    static let startFresh = "Start fresh"
    static let freshPrompt = "Priya's MacBook isn't available?"
    static let freshWarning = "Starting fresh sets this device up as new. Your data on Priya's MacBook is not copied here."
    static let words = ["orbit", "lantern", "marble", "copper", "willow", "harbor", "velvet", "ember", "quartz", "meadow", "falcon", "ribbon", "summit", "cactus", "pepper", "violet", "anchor", "glacier", "maple", "tundra", "saffron", "bridge", "nickel", "plume"]
}

/// One screen of the onboarding set, rendered with made-up data.
public struct WOnboardingScreen: Identifiable {
    public let id: String
    public let view: AnyView
}

private func sampleQR() -> NSImage? {
    let filter = CIFilter.qrCodeGenerator()
    filter.message = Data("sample only, not a key".utf8)
    filter.correctionLevel = "M"
    guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else { return nil }
    let rep = NSCIImageRep(ciImage: output)
    let image = NSImage(size: rep.size)
    image.addRepresentation(rep)
    return image
}

private struct KeySetupDemo: View {
    @State private var passphrase: String
    @State private var confirmation: String
    @State private var test = ""
    @State private var wroteDown = false
    @State private var saved = false
    let model: WKeySetupModel
    init(model: WKeySetupModel, passphrase: String = "", confirmation: String = "") {
        self.model = model
        _passphrase = State(initialValue: passphrase)
        _confirmation = State(initialValue: confirmation)
        _wroteDown = State(initialValue: model.hasKey)
        _saved = State(initialValue: false)
    }
    var body: some View {
        WKeySetupView(model: model, passphrase: $passphrase, confirmation: $confirmation, testPassphrase: $test, wroteDown: $wroteDown, savedConfirmed: $saved)
    }
}

private struct RecoveryDemo: View {
    @State private var method: WRecoveryMethod
    @State private var passphrase: String
    @State private var words: String
    let model: WRecoveryModel
    init(model: WRecoveryModel, method: WRecoveryMethod, passphrase: String = "", words: String = "") {
        self.model = model
        _method = State(initialValue: method)
        _passphrase = State(initialValue: passphrase)
        _words = State(initialValue: words)
    }
    var body: some View {
        WRecoveryView(model: model, method: $method, passphrase: $passphrase, words: $words)
    }
}

public enum WOnboardingGallery {
    private static func rows(_ ids: [(String, String, String, String, WPermissionState)]) -> [WPermissionRowModel] {
        ids.map { WPermissionRowModel(id: $0.0, glyph: $0.1, name: $0.2, reason: $0.3, state: $0.4, onAllow: {}, onOpenSettings: {}) }
    }

    static var firstRun: WFirstRunModel {
        var m = WFirstRunModel()
        m.title = WSampleCopy.welcome
        m.intro = WSampleCopy.firstRunIntro
        m.groups = [WPermissionGroup(id: "required", heading: WSampleCopy.allowedRequired, rows: rows([
            ("mic", "mic.fill", WSampleCopy.micName, WSampleCopy.micReason, .allowed),
            ("screen", "display", WSampleCopy.screenName, WSampleCopy.screenReason, .denied),
        ]))]
        m.finePrint = [WSampleCopy.changeLater, WSampleCopy.consentRules, WSampleCopy.anythingElse]
        m.primaryTitle = WSampleCopy.getStarted
        return m
    }

    static var checklist: WPermissionChecklistModel {
        var m = WPermissionChecklistModel()
        m.title = WSampleCopy.checklistTitle
        m.intro = WSampleCopy.checklistIntro
        m.groups = [
            WPermissionGroup(id: "required", heading: WSampleCopy.allowedRequired, rows: rows([
                ("mic", "mic.fill", WSampleCopy.micName, WSampleCopy.micReason, .allowed),
                ("screen", "display", WSampleCopy.screenName, WSampleCopy.screenReason, .denied),
            ])),
            WPermissionGroup(id: "optional", heading: WSampleCopy.optional, rows: rows([
                ("ax", "accessibility", WSampleCopy.accessibilityName, WSampleCopy.accessibilityReason, .notYet),
                ("cal", "calendar", WSampleCopy.calendarName, WSampleCopy.calendarReason, .allowed),
                ("notif", "bell.fill", WSampleCopy.notificationsName, WSampleCopy.notificationsReason, .notYet),
            ])),
        ]
        m.progress = "2 of 5 allowed"
        m.refreshTitle = WSampleCopy.refresh
        return m
    }

    static func notice(title: String, paragraphs: [String], hints: [String] = [], error: String? = nil, fraction: Double? = nil, busy: Bool = false, actions: [WScreenAction]) -> WNoticeScreenModel {
        var m = WNoticeScreenModel()
        m.title = title
        m.paragraphs = paragraphs
        m.hints = hints
        m.errorLine = error
        m.fraction = fraction
        m.busy = busy
        m.actions = actions
        return m
    }

    static func action(_ id: String, _ title: String, _ kind: WButtonKind = .secondary, enabled: Bool = true) -> WScreenAction {
        WScreenAction(id: id, title: title, kind: kind, isEnabled: enabled, run: {})
    }

    static func keySetup(hasKey: Bool) -> WKeySetupModel {
        var m = WKeySetupModel()
        m.title = WSampleCopy.setupTitle
        m.intro = WSampleCopy.setupIntro
        m.passphraseHeading = WSampleCopy.stepPassphrase
        m.passphraseHelp = WSampleCopy.passphraseHelp
        m.passphraseLabel = WSampleCopy.passphraseField
        m.confirmLabel = WSampleCopy.confirmField
        m.passphraseLockedHint = WSampleCopy.passphraseLockedHint
        m.fileHeading = WSampleCopy.stepFile
        m.fileHelp = WSampleCopy.fileHelp
        m.saveFileTitle = WSampleCopy.saveFile
        m.wordsHeading = WSampleCopy.stepWords
        m.wordsNotice = WSampleCopy.wordsAreTheKey
        m.printTitle = WSampleCopy.printWords
        m.wroteDownLabel = WSampleCopy.wroteDown
        m.qrLabel = WSampleCopy.qrLabel
        m.testHeading = WSampleCopy.stepTest
        m.testHelp = WSampleCopy.testHelp
        m.testLabel = WSampleCopy.passphraseField
        m.testButtonTitle = WSampleCopy.testButton
        m.savedItLabel = WSampleCopy.iSavedIt
        m.savedItLockedHint = WSampleCopy.iSavedItLocked
        m.finePrint = WSampleCopy.lostBoth
        m.createTitle = WSampleCopy.createKey
        m.continueTitle = WSampleCopy.continueDefault
        m.startOverTitle = WSampleCopy.startOver
        m.leaveTitle = WSampleCopy.quitAction
        if hasKey {
            m.hasKey = true
            m.passphraseLocked = true
            m.passphraseCount = "16 of at least 12 characters"
            m.meetsMinimum = true
            m.words = WSampleCopy.words
            m.qrImage = sampleQR()
            m.savedLine = "Saved to /Volumes/Backup/Whimbrel recovery key.txt"
            m.printedNote = WSampleCopy.printedNote
            m.testPassedLine = WSampleCopy.testPassed
            m.canTest = true
            m.canConfirmSaved = true
            m.canContinue = false
        } else {
            m.passphraseCount = "8 of at least 12 characters"
            m.passphraseProblem = "Use at least 12 characters."
            m.canCreate = false
        }
        return m
    }

    static func recovery(rejected: Bool) -> WRecoveryModel {
        var m = WRecoveryModel()
        m.title = WSampleCopy.recoveryTitle
        m.reason = rejected ? WSampleCopy.recoveryKeyRejected : WSampleCopy.recoveryKeyMissing
        m.unchanged = WSampleCopy.recoveryUnchanged
        m.methodLabel = WSampleCopy.recoveryMethod
        m.fileMethodTitle = WSampleCopy.methodFile
        m.wordsMethodTitle = WSampleCopy.methodWords
        m.chooseFileTitle = WSampleCopy.chooseFile
        m.passphraseLabel = WSampleCopy.passphraseField
        m.wordsLabel = WSampleCopy.methodWords
        m.wordsHelp = WSampleCopy.wordsHelp
        m.finePrint = [WSampleCopy.recoveryNeverDeletes, WSampleCopy.recoveryLostBoth]
        m.quitTitle = WSampleCopy.quitAction
        if rejected {
            m.unlockTitle = WSampleCopy.unlockWithWords
            m.canUnlock = true
            m.errorLine = WSampleCopy.keyForAnotherDatabase
        } else {
            m.unlockTitle = WSampleCopy.unlockWithFile
            m.chosenFileName = "Whimbrel recovery key.txt"
            m.canUnlock = true
        }
        return m
    }

    static var sync: WSyncWaitingModel {
        var m = WSyncWaitingModel()
        m.title = WSampleCopy.syncTitle
        m.message = WSampleCopy.syncMessage
        m.freshStartPrompt = WSampleCopy.freshPrompt
        m.startFreshTitle = WSampleCopy.startFresh
        m.cancelTitle = WSampleCopy.cancel
        return m
    }

    /// Every screen, in the order the app shows them.
    public static func screens() -> [WOnboardingScreen] {
        func s(_ id: String, _ view: some View) -> WOnboardingScreen { WOnboardingScreen(id: id, view: AnyView(view)) }
        let quit = action("quit", WSampleCopy.quitAction)
        return [
            s("first-run", WFirstRunView(model: firstRun)),
            s("permissions", WPermissionChecklistView(model: checklist)),
            s("launch-starting", WLaunchStartingView(message: WSampleCopy.starting, delay: .zero)),
            s("launch-migrating", WNoticeScreenView(model: notice(
                title: WSampleCopy.migratingTitle, paragraphs: [WSampleCopy.migratingBody], hints: [WSampleCopy.migratingInterruptHint], fraction: 0.42, actions: []))),
            s("launch-blocked-missing", WNoticeScreenView(model: notice(
                title: WSampleCopy.missingTitle, paragraphs: [WSampleCopy.missingWithBackupBody], hints: [WSampleCopy.missingWithBackupHint],
                actions: [quit, action("restore", WSampleCopy.restoreCopyAction, .primary)]))),
            s("launch-blocked-error", WNoticeScreenView(model: notice(
                title: WSampleCopy.errorTitle, paragraphs: [WSampleCopy.errorOpenFailed], hints: [WSampleCopy.errorTryAgainHint],
                actions: [quit, action("continue", WSampleCopy.continueWithoutStore), action("retry", WSampleCopy.tryAgain, .primary)]))),
            s("key-unavailable", WNoticeScreenView(model: notice(
                title: WSampleCopy.unavailableTitle, paragraphs: [WSampleCopy.unavailableBody], hints: [WSampleCopy.unavailableHint, WSampleCopy.unavailableTouchesNothing],
                error: WSampleCopy.stillUnavailable, actions: [quit, action("retry", WSampleCopy.tryAgain, .primary)]))),
            s("key-setup-passphrase", KeySetupDemo(model: keySetup(hasKey: false), passphrase: "pinecone", confirmation: "pinecone")),
            s("key-setup-ready", KeySetupDemo(model: keySetup(hasKey: true), passphrase: "correct horse battery", confirmation: "correct horse battery")),
            s("recovery-needs-key", RecoveryDemo(model: recovery(rejected: false), method: .file, passphrase: "correct horse battery")),
            s("recovery-key-rejected", RecoveryDemo(model: recovery(rejected: true), method: .words, words: "orbit lantern marble copper willow harbor")),
            s("model-setup", WNoticeScreenView(model: notice(
                title: WSampleCopy.modelTitle, paragraphs: [WSampleCopy.modelStatus], fraction: 0.35, busy: true,
                actions: [action("skip", WSampleCopy.skipForNow), action("download", "Downloading\u{2026}", .primary, enabled: false)]))),
            s("model-setup-failed", WNoticeScreenView(model: notice(
                title: WSampleCopy.modelTitle, paragraphs: [WSampleCopy.modelFailed], error: WSampleCopy.modelFailedHelp,
                actions: [action("skip", WSampleCopy.skipForNow), action("download", WSampleCopy.tryAgain, .primary)]))),
            s("sync-waiting", WSyncWaitingView(model: sync)),
        ]
    }
}

private struct GalleryDemoField: View {
    @State private var text = "pinecone"
    @State private var other = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            WField(label: "Passphrase", text: $text, help: "At least 12 characters.", count: "8 of at least 12 characters", error: "Use at least 12 characters.", isSecure: true)
            WField(label: "Type it again", text: $other, isSecure: true, isEnabled: false)
        }
    }
}

// MARK: Home and the shell

/// One rendered case of Home inside the app shell, with made-up content (the Northwind fixtures).
public struct WHomeGalleryCase {
    public let id: String
    public let mode: WGalleryMode
    public let textScale: CGFloat
    public let width: CGFloat
    public let height: CGFloat
    public let view: AnyView
}

@MainActor
public enum WHomeGallery {
    static func meetingNext() -> WHomeMeeting {
        WHomeMeeting(
            id: "m-pricing", kind: .upcoming, time: "10:00 AM", timeRange: "Today, 10:00 AM. Last met Tue, Nov 10.", title: "Pricing review",
            attendees: ["Sam Okafor", "Priya Shah", "You"], eyebrow: "Next, in 8 minutes", startTitle: "Start notes", briefTitle: "Open notes from Nov 10",
            facts: [WHomeFact(label: "Time", value: "Today, 10:00 AM"), WHomeFact(label: "With", value: "Sam Okafor, Priya Shah, you"), WHomeFact(label: "Last met", value: "Tue, Nov 10")]
        )
    }

    /// Newest at the top: later meetings, (the hero goes here), now, blocks and past meetings newest first, then Yesterday and Last week.
    static func stops(paused: Bool = false, history: Bool = true) -> [WHomeStop] {
        let roadmap = WHomeMeeting(id: "m-roadmap", kind: .past, time: "8:00 AM", title: "Roadmap check-in", attendees: ["Dana Whitfield", "You"], gist: "Three launch dates moved by a week.", openTitle: "Open")
        let blocks: [WHomeStop] = paused
            ? [.notice("Memory is paused until 3:10 PM.")]
            : [
                .memory(WHomeBlock(id: "b3", timeRange: "9:30 to 9:52 AM", duration: "22 min", items: [
                    WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing v3.xlsx in Excel"),
                    WMemoryItem(app: .blue, glyph: "envelope", name: "1 email from Priya Shah"),
                ], openTitle: "Open")),
                .memory(WHomeBlock(id: "b2", timeRange: "8:40 to 9:30 AM", duration: "50 min", items: [
                    WMemoryItem(app: .green, glyph: "tablecells", name: "Pricing v3.xlsx in Excel"),
                    WMemoryItem(app: .purple, glyph: "number", name: "#pricing in Slack"),
                    WMemoryItem(app: .blue, glyph: "envelope", name: "2 emails with Northwind"),
                ], openTitle: "Open")),
                .memory(WHomeBlock(id: "b1", timeRange: "8:05 to 8:40 AM", duration: "35 min", items: [
                    WMemoryItem(app: .blue, glyph: "envelope", name: "Inbox, 11 emails in Mail"),
                    WMemoryItem(app: .purple, glyph: "number", name: "#general in Slack"),
                ], openTitle: "Open")),
            ]
        var out: [WHomeStop] = [
            .meeting(WHomeMeeting(id: "m-hiring", kind: .upcoming, time: "3:00 PM", title: "Hiring debrief", attendees: ["Priya Shah", "You"], eyebrow: "in 5 hr 8 min", startTitle: "Start")),
            .meeting(WHomeMeeting(id: "m-design", kind: .upcoming, time: "1:30 PM", title: "Design sync", attendees: ["Dana Whitfield", "Lee Brandt", "You"], eyebrow: "in 3 hr 38 min", startTitle: "Start")),
            .now(time: "9:52 AM", label: paused ? "Paused until 3:10 PM" : "Memory on"),
        ]
        out += blocks
        out.append(.meeting(roadmap))
        if history {
            out.append(.history(WHomeHistory(id: "yesterday", title: "Yesterday", dateText: "Mon, Nov 16", meetings: [
                WHomeMeeting(id: "y1", kind: .past, time: "2:00 PM", title: "Roadmap check-in", attendees: ["Dana Whitfield"], gist: "Two launch dates moved.", openTitle: "Open"),
                WHomeMeeting(id: "y2", kind: .past, time: "10:00 AM", title: "Northwind renewal call", attendees: ["Sam Okafor"], gist: "Renewal moved to next quarter.", openTitle: "Open"),
            ])))
            out.append(.history(WHomeHistory(id: "lastWeek", title: "Earlier", dateText: "Tue, Nov 10", meetings: [
                WHomeMeeting(id: "w1", kind: .past, time: "10:00 AM", title: "Pricing review", attendees: ["Sam Okafor", "Priya Shah"], gist: "List price holds.", openTitle: "Open"),
            ])))
        }
        return out
    }

    static func sections() -> [WHomeSection] {
        [
            WHomeSection(kind: .approvals, title: "Needs your decision", rows: [WHomeRow(id: "a", title: "2 pending approvals", actionTitle: "Review")]),
            WHomeSection(kind: .followups, title: "Follow-ups", rows: [
                WHomeRow(id: "f1", title: "Send Priya the revised pricing sheet", actionTitle: "Done", actionName: "Done, Send Priya the revised pricing sheet"),
                WHomeRow(id: "f2", title: "Ask Sam about the Northwind renewal date", actionTitle: "Done", actionName: "Done, Ask Sam about the Northwind renewal date"),
            ]),
            WHomeSection(kind: .recap, title: "Daily recap", rows: [WHomeRow(id: "r", title: "Yesterday you met twice. The Northwind renewal moved to next quarter and Sam owns the follow-up.")]),
            WHomeSection(kind: .week, title: "This week", rows: [
                WHomeRow(id: "w1", time: "9:00 AM", title: "Quarterly planning", detail: "4 invited", heading: "Tomorrow"),
                WHomeRow(id: "w2", time: "2:00 PM", title: "Northwind check-in", detail: "2 invited"),
                WHomeRow(id: "w3", time: "11:00 AM", title: "Roadmap review", detail: "5 invited", heading: "Thu, Nov 19"),
            ]),
        ]
    }

    static func model(_ elements: WHomeElements = WHomeElements(), paused: Bool = false, sections withSections: Bool = true) -> WHomeModel {
        var m = WHomeModel()
        m.hero = .next(meetingNext())
        m.timeline = stops(paused: paused)
        m.sections = withSections ? sections() : []
        m.elements = elements
        return m
    }

    static let items: [WShellItem] = [
        WShellItem(id: "today", glyph: "clock", title: "Today", shortcut: "\u{2318}1"),
        WShellItem(id: "meetings", glyph: "video", title: "Meetings", shortcut: "\u{2318}2"),
        WShellItem(id: "library", glyph: "books.vertical", title: "Library", shortcut: "\u{2318}3"),
        WShellItem(id: "topics", glyph: "tag", title: "Topics", shortcut: "\u{2318}6"),
        WShellItem(id: "automations", glyph: "bolt", title: "Automations", badge: "2", shortcut: "\u{2318}4"),
        WShellItem(id: "settings", glyph: "gearshape", title: "Settings", shortcut: "\u{2318}5"),
    ]

    static let apps = [WHomeAppChoice(id: "com.microsoft.Excel", name: "Excel"), WHomeAppChoice(id: "com.tinyspeck.slackmacgap", name: "Slack"), WHomeAppChoice(id: "com.apple.mail", name: "Mail")]

    /// Home inside the shell. The recording bar offers Stop only, because that is all the Mac app can do.
    static func shell(
        _ model: WHomeModel, memory: WMemoryState = .on, recording: WShellRecording? = nil, ask: WHomeAsk = .available(placeholder: "Search your work memory"),
        live: AnyView = AnyView(EmptyView()), height: CGFloat = 900
    ) -> AnyView {
        AnyView(
            WShell(
                items: items, selection: "today", onSelect: { _ in },
                header: WShellHeader(title: "Today", subtitle: "Tuesday, November 17", ask: ask), elements: model.elements,
                people: ["Priya Shah", "Sam Okafor", "Dana Whitfield"], appChoices: apps, showsFilters: true, onSubmit: { _, _ in },
                thread: model.thread(todayTitle: "Today", nowTime: "9:52 AM"), onThread: { _ in },
                memory: memory, onWhatIsCaptured: {},
                recording: recording, onStop: recording == nil ? nil : {},
                pauseChoices: { WPauseChoices(isPaused: { if case .pausedUntil = memory { return true } else { return false } }(), onPause: { _ in }, onResume: {}) },
                footer: {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("This Mac").font(.caption).foregroundStyle(.secondary)
                        Text("Synced").font(.caption).foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 10)
                },
                banner: { EmptyView() },
                content: { WHomeView(model: model, actions: WHomeActions()) { live }.environment(\.wScrollsContent, true) }
            )
            .frame(height: height)
        )
    }

    static func otherSection() -> AnyView {
        AnyView(
            WShell(
                items: items, selection: "meetings", onSelect: { _ in },
                header: WShellHeader(title: "Meetings", ask: .available(placeholder: "Search your work memory")),
                onSubmit: { _, _ in }, memory: .on, onWhatIsCaptured: {}, recording: WShellRecording(state: .recording, elapsedSeconds: 848), onStop: {},
                pauseChoices: { WPauseChoices(isPaused: false, onPause: { _ in }, onResume: {}) },
                footer: { Text("This Mac").font(.caption).foregroundStyle(.secondary).padding(.horizontal, 10) }, banner: { EmptyView() },
                content: { WEmptyState(glyph: "video", title: "Meetings", help: "A section other than Today keeps its own screen.").frame(maxHeight: .infinity) }
            )
            .frame(height: 560)
        )
    }

    static func liveModel() -> WHomeModel {
        var m = model()
        m.hero = .live(title: "Pricing review", elapsedSeconds: 848)
        return m
    }

    public static func cases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ view: AnyView, mode: WGalleryMode = .light, scale: CGFloat = 1, width: CGFloat = 1200, height: CGFloat = 900) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: height, view: view))
        }
        let base = model()
        add("home-light", shell(base, height: 1500), height: 1500)
        add("home-dark", shell(base, height: 1500), mode: .dark, height: 1500)
        add("home-light-2.0", shell(base, height: 1400), scale: 2, width: 1500, height: 1400)
        add("home-high-contrast", shell(base, height: 1500), mode: .highContrastLight, height: 1500)
        add("home-scoreboard", shell(model(WHomeElements(nextMeeting: .scoreboard)), height: 1100), height: 1100)
        add("home-infobox", shell(model(WHomeElements(nextMeeting: .infobox)), height: 1100), height: 1100)
        add("home-ruled", shell(model(WHomeElements(timeline: .ruled)), height: 1300), height: 1300)
        add("home-ask-prompt", shell(model(WHomeElements(ask: .prompt)), height: 700), height: 700)
        add("home-actions-links", shell(model(WHomeElements(actions: .links)), height: 1100), height: 1100)

        var empty = WHomeModel()
        empty.hero = .noCalendar(title: "Nothing on today yet.", help: "Connect a calendar to see upcoming meetings here.", actionTitle: "Connect Calendar")
        empty.timeline = [.now(time: "9:52 AM", label: "Memory on")]
        add("home-empty", shell(empty, height: 700), height: 700)
        var loading = model(sections: false)
        loading.load = .loading
        add("home-loading", shell(loading, height: 800), height: 800)
        var failed = model(sections: false)
        failed.load = .failed(title: "Today could not load.", help: "Whimbrel could not read your meetings. Your notes are safe. Try again.")
        add("home-error", shell(failed, height: 700), height: 700)
        add("home-capture-paused", shell(model(paused: true, sections: false), memory: .pausedUntil("3:10 PM"), height: 1000), height: 1000)
        add("home-recording-live", shell(liveModel(), recording: WShellRecording(state: .recording, elapsedSeconds: 848), live: AnyView(
            WTranscriptView(accessory: "Listening", entries: [
                WTranscriptEntry(time: "10:12", speaker: WPerson(name: "Sam Okafor"), text: "We can hold the list price and change the tiers."),
                WTranscriptEntry(time: "10:13", speaker: WPerson(name: "Priya Shah"), text: "That works for the Northwind renewal.", state: .live),
            ])
        ), height: 1100), height: 1100)
        add("shell-other-section-recording", otherSection(), height: 560)
        return out
    }
}

// MARK: Live meeting

/// The live meeting screen and the in-meeting window, with made-up content.
@MainActor
public enum WLiveGallery {
    struct SampleEditor: View {
        @Environment(\.wTheme) private var theme
        let cursor: Bool
        static let rows: [(String, String)] = [
            ("sam wants usage tier", "10:03"), ("annual discount 15 or 20?", "10:09"), ("priya: check churn on starter first", "10:12"),
        ]
        var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(Self.rows.enumerated()), id: \.offset) { _, row in
                    line(row.0, time: row.1)
                }
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text("\u{2022}").foregroundStyle(theme.textPrimary)
                    Text(cursor ? "sam: start at 15, then" : "").wFont(.subtitle).fontWeight(.regular).foregroundStyle(theme.textSecondary)
                    if cursor { Rectangle().fill(theme.accentPrimary).frame(width: 2, height: 20) }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, 10)
            }
        }
        private func line(_ text: String, time: String) -> some View {
            VStack(spacing: 0) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text("\u{2022}").foregroundStyle(theme.textPrimary)
                    Text(text).wFont(.subtitle).fontWeight(.regular).foregroundStyle(theme.textPrimary)
                    Spacer(minLength: 8)
                    Text(time).wFont(.secondary).foregroundStyle(theme.textSecondary)
                }
                .padding(.vertical, 10)
                Divider().overlay(theme.border.opacity(0.6))
            }
        }
    }

    struct SampleAsk: View {
        @Environment(\.wTheme) private var theme
        @Environment(\.wStrings) private var strings
        var body: some View {
            HStack(spacing: 8) {
                WGlyph(name: "sparkles", size: 13).foregroundStyle(theme.textSecondary)
                Text(strings.live.askAboutMeeting).wFont(.body).foregroundStyle(theme.textSecondary)
                Spacer(minLength: 0)
                WGlyph(name: "paperplane", size: 13).foregroundStyle(theme.textSecondary)
            }
            .padding(.horizontal, 12).frame(minHeight: 36)
            .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.backgroundCard))
            .overlay(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).strokeBorder(theme.border, lineWidth: 1))
        }
    }

    struct SampleWarning: View {
        @Environment(\.wTheme) private var theme
        var body: some View {
            HStack(spacing: 10) {
                WGlyph(name: "exclamationmark.triangle", size: 14).foregroundStyle(theme.error)
                Text("Whimbrel isn't hearing anything. Check that the meeting has started and your audio devices are right.").wFont(.body).foregroundStyle(theme.textPrimary)
                Spacer(minLength: 0)
            }
            .padding(12)
            .background(RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous).fill(theme.backgroundCard))
            .overlay(RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous).strokeBorder(theme.error, lineWidth: 1))
        }
    }

    static let transcript: [WTranscriptEntry] = [
        WTranscriptEntry(time: "10:12", speaker: WPerson(name: "Priya Shah"), text: "So a usage-based tier, on top of seats?"),
        WTranscriptEntry(time: "10:12", speaker: WPerson(name: "Sam Okafor"), text: "Yes. Small base, then pay for what you use. Starter would like it most."),
        WTranscriptEntry(time: "10:13", speaker: WPerson(name: "You"), text: "Do we know what that does to churn on Starter?"),
        WTranscriptEntry(time: "10:14", speaker: WPerson(name: "Sam Okafor"), text: "On the annual discount, I would start at 15 and see...", state: .live),
    ]

    /// The newest line's caption follows the state: Listening, Paused or Resuming.
    static func transcript(for state: WLiveState) -> [WTranscriptEntry] {
        var rows = transcript
        guard let last = rows.popLast() else { return rows }
        let line: WTranscriptLineState = state == .paused ? .paused : (state == .resuming ? .resuming : (state == .recording ? .live : .normal))
        rows.append(WTranscriptEntry(time: last.time, speaker: last.speaker, text: last.text, state: line))
        return rows
    }

    static let stops: [WLiveStop] = [
        WLiveStop(id: "next", time: "1:30 PM", caption: "Design sync", kind: .next),
        WLiveStop(id: "now", time: "10:14 AM", caption: "Now", kind: .now),
        WLiveStop(id: "start", time: "10:00 AM", caption: "Started", kind: .started),
    ]

    static func model(
        state: WLiveState = .recording, seconds: Int = 848, level: Float = 0.62, transcript withTranscript: Bool = true,
        notice: Bool = true, marks: Bool = true
    ) -> WLiveMeetingModel {
        WLiveMeetingModel(
            title: "Pricing review", detail: "Today, 10:00 AM. Sam Okafor, Priya Shah, you.", state: state, recordedSeconds: seconds,
            level: level, showsNotice: notice, marks: marks ? [WLiveMark(id: "m1", timeOfDay: "10:12", label: "Priya: check churn on starter first")] : [],
            stops: stops, showsTranscript: withTranscript, transcript: transcript(for: state), transcriptAccessory: state == .paused ? "Paused" : (state == .resuming ? "Resuming" : "Auto-scroll on"))
    }

    static func screen(_ model: WLiveMeetingModel, warning: Bool = false, height: CGFloat = 900) -> AnyView {
        AnyView(
            WShell(
                items: WHomeGallery.items, selection: "today", onSelect: { _ in },
                header: WShellHeader(title: "Pricing review", subtitle: "Now, 10:14 AM", ask: .available(placeholder: "Search your work memory")),
                onSubmit: { _, _ in }, memory: .on, onWhatIsCaptured: {}, recording: nil,
                pauseChoices: { WPauseChoices(isPaused: false, onPause: { _ in }, onResume: {}) },
                footer: { Text("This Mac").font(.caption).foregroundStyle(.secondary).padding(.horizontal, 10) },
                banner: { EmptyView() },
                content: {
                    WLiveMeetingView(
                        model: model, actions: WLiveActions(),
                        warnings: { if warning { SampleWarning() } },
                        editor: { SampleEditor(cursor: model.state == .recording || model.state == .paused) },
                        alerts: { EmptyView() }, ask: { SampleAsk() }
                    )
                    .environment(\.wScrollsContent, false)
                }
            )
            .frame(height: height)
        )
    }

    static func bar(_ state: WLiveState, level: Float, seconds: Int = 848, seed: [Float] = []) -> AnyView {
        AnyView(
            WMeetingBar(model: WMeetingBarModel(title: "Pricing review", state: state, recordedSeconds: seconds, level: level), actions: WLiveActions()) { SampleAsk() }
                .padding(24)
        )
    }

    public static func cases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ view: AnyView, mode: WGalleryMode = .light, scale: CGFloat = 1, width: CGFloat = 1440, height: CGFloat = 900) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: height, view: view))
        }
        add("live-light", screen(model()))
        add("live-dark", screen(model()), mode: .dark)
        add("live-paused", screen(model(state: .paused, level: 0)))
        add("live-resuming", screen(model(state: .resuming, level: 0)))
        add("live-starting", screen(model(state: .starting, seconds: 0, level: 0, transcript: false, marks: false)))
        add("live-processing", screen(model(state: .processing(stage: "Transcribing"), level: 0, notice: false)))
        add("live-problem", screen(model(state: .problem("Microphone not found"), level: 0), warning: true))
        add("live-no-transcript", screen(model(transcript: false)))
        add("live-read-only", screen(model(state: .readOnly, level: 0, notice: false)))
        add("live-light-2.0", screen(model(), height: 2300), scale: 2, width: 1500, height: 2300)
        add("live-high-contrast", screen(model()), mode: .highContrastLight)
        let seed: [Float] = [0.1, 0.3, 0.55, 0.4, 0.7, 0.5, 0.25, 0.6, 0.45, 0.2, 0.5, 0.35, 0.65, 0.3, 0.45, 0.62]
        add("meeting-window-recording", AnyView(WMeetingBar(model: WMeetingBarModel(title: "Pricing review", state: .recording, recordedSeconds: 848, level: 0.62), actions: WLiveActions()) { SampleAsk() }.padding(24)), width: 360, height: 300)
        add("meeting-window-paused", bar(.paused, level: 0), width: 360, height: 300)
        add("meeting-window-dark", bar(.recording, level: 0.5), mode: .dark, width: 360, height: 300)
        add("meeting-window-light-2.0", bar(.recording, level: 0.5), scale: 2, width: 560, height: 520)
        _ = seed
        return out
    }
}

// MARK: Finished note

/// The finished note page and its transcript view, with made-up content (a renewal call).
@MainActor
public enum WNoteGallery {
    static let sam = "Sam Okafor", dana = "Dana Whitfield", jo = "Jo (Northwind)", alex = "Alex Rivera", you = "You"

    static func seg(_ id: String, _ sec: Int, _ speaker: String, _ text: String, length: Int? = nil, part: Int = 0, divider: String? = nil) -> WNoteSegment {
        let start = Int64(sec * 100)
        return WNoteSegment(
            id: id, startCs: start, endCs: length.map { Int64((sec + $0) * 100) }, time: WLaneMath.clock(cs: start), speakerKey: speaker,
            speaker: speaker, text: text, partIndex: part, dividerBefore: divider)
    }

    static let segments: [WNoteSegment] = [
        seg("s1", 0, sam, "Thanks for making the time, Jo. I want to walk through the renewal.", length: 20),
        seg("s2", 23, jo, "Sure. We have been happy with the product since the spring.", length: 80),
        seg("s3", 372, jo, "If the price holds, we can do two years.", length: 40),
        seg("s4", 415, sam, "That works for us. We can hold the list price.", length: 120),
        seg("s5", 580, dana, "We expect fifteen more seats by Q2. I will confirm the date.", length: 60),
        seg("s6", 650, jo, "Good. Seats matter more to us than the term.", length: 150),
        seg("s7", 900, you, "Let me note that down.", length: 20),
        seg("s8", 1105, jo, "There are still a few open items on the questionnaire.", length: 70),
        seg("s9", 1190, dana, "Who on your side is the owner for that?", length: 40),
        seg("s10", 1243, jo, "That is our IT lead, and she is out until Monday.", length: 100),
        seg("s11", 1440, sam, "So the next step is the quote, and then the questionnaire.", length: 280),
        seg("s12", 1730, sam, "I will get the quote to you by Thursday.", length: 30),
        seg("s13", 1771, dana, "I will answer the open items by Friday.", length: 25),
        seg("s14", 1862, alex, "I will check the seat price before we send it.", length: 30),
        seg("s15", 1900, sam, "Thanks everyone. Talk Thursday.", length: 15),
    ]

    /// A meeting that was continued: the second part keeps the meeting's clock and starts with its own divider.
    static let continuedSegments: [WNoteSegment] = Array(segments.prefix(8)) + [
        seg("c1", 1500, "Speaker 1", "Back again. Where did we leave the seats?", length: 4, part: 1, divider: "\u{2014} rejoined 10:25 \u{2014}"),
        seg("c2", 1512, "Speaker 2", "At fifty five, if the price holds.", length: 4, part: 1),
        seg("c3", 1560, "Speaker 1", "Then I will write that up.", length: 3, part: 1),
        seg("c4", 1690, "Speaker 2", "One more thing on the questionnaire.", length: 5, part: 1),
    ]

    static func source(_ number: Int, _ id: String, line: String) -> WNoteSource {
        let seg = segments.first { $0.id == id }!
        return WNoteSource(number: number, segmentId: id, speaker: seg.speaker, time: seg.time, quote: seg.text, lineText: line)
    }

    static let marks: [WNoteMark] = [
        WNoteMark(id: "k1", atCs: 49500, clock: "8:15", label: "Ask Jo about the term"),
        WNoteMark(id: "k2", atCs: 144200, clock: "24:02", label: "Who owns the questionnaire?"),
    ]

    /// Moments marked in the silence after the last line and in the gap before a later part.
    static let outlyingMarks: [WNoteMark] = [
        WNoteMark(id: "k3", atCs: 125000, clock: "20:50", label: "Gap between the parts"),
        WNoteMark(id: "k4", atCs: 196000, clock: "32:40", label: "After the last line"),
    ]

    static let decision1 = "Northwind will renew for two years if the price stays the same."
    static let decision2 = "Seats grow from 40 to 55 in Q2. Dana will confirm the start date."
    static let question1 = "Northwind's security questionnaire still has open items, and the renewal date is close."
    static let action1 = "Send the two-year renewal quote to Jo."
    static let action2 = "Answer the open items on the questionnaire."
    static let action3 = "Check that 55 seats still fits the current price."

    static func model(
        mode: WNoteMode = .note, sources withSources: Bool = true, selected: Int? = 3, state: WNoteState = .ready,
        highlighted: String? = nil, continued: Bool = false, back: String? = "Meetings", editing: Bool = false,
        readingScale: CGFloat = 1, marks marked: [WNoteMark]? = nil, segments overrideSegments: [WNoteSegment]? = nil
    ) -> WNoteModel {
        func src(_ number: Int, _ id: String, _ line: String) -> WNoteSource? { withSources ? source(number, id, line: line) : nil }
        let all = [src(1, "s3", decision1), src(2, "s5", decision2), src(3, "s8", question1), src(4, "s12", action1), src(5, "s13", action2), src(6, "s14", action3)]
        var m = WNoteModel(
            mode: mode, backTitle: back, title: "Northwind renewal call", eyebrow: "Mon, Nov 16, 10:00 AM \u{00B7} 32 min",
            attendees: "Sam Okafor, Dana Whitfield, Jo (Northwind), you",
            yourNotes: "jo ok with 2 yr term if price holds\nsecurity questionnaire?? who owns it\nseats 40 to 55 in Q2",
            state: state, canEdit: true, isEditing: editing,
            summary: WNoteLine(id: "summary", text: "Northwind will renew for two years if the price stays the same, with seats growing from 40 to 55 in Q2. The security questionnaire is the open risk."),
            decisions: [
                WNoteLine(id: "d1", text: decision1, source: all[0]),
                WNoteLine(id: "d2", text: decision2, source: all[1]),
            ],
            openQuestions: [
                WNoteLine(id: "q1", text: question1, source: all[2]),
            ],
            actionItems: [
                WNoteActionItem(id: "a1", line: WNoteLine(id: "a1", text: action1, source: all[3]), owner: sam, due: "Thu, Nov 19"),
                WNoteActionItem(id: "a2", line: WNoteLine(id: "a2", text: action2, source: all[4]), owner: dana, due: "Fri, Nov 20"),
                WNoteActionItem(id: "a3", line: WNoteLine(id: "a3", text: action3, source: all[5]), owner: alex, due: "Tue, Nov 24"),
            ],
            marks: marked ?? marks, provenance: ["Generated by gpt-5 (hosted)"],
            selectedSource: withSources ? selected.flatMap { n in all.compactMap { $0 }.first { $0.number == n }?.id } : nil,
            readingScale: readingScale, segments: overrideSegments ?? (continued ? continuedSegments : segments), highlightedSegmentId: highlighted)
        if case .processing = state {
            m.summary = nil; m.decisions = []; m.openQuestions = []; m.actionItems = []; m.provenance = []; m.canSendRecap = false
            m.segments = []; m.transcriptMessage = "Transcribing in the background. Check back shortly."
        }
        if case .failed = state { m.summary = nil; m.decisions = []; m.openQuestions = []; m.actionItems = []; m.provenance = [] }
        return m
    }

    /// A notice the app owns (the summary fallback, names changed, the device chip), drawn here as a plain line.
    struct SampleNotice: View {
        @Environment(\.wTheme) private var theme
        let text: String
        var body: some View {
            HStack(spacing: 8) {
                WGlyph(name: "info.circle", size: 13).foregroundStyle(theme.textSecondary)
                Text(text).wFont(.secondary).foregroundStyle(theme.textSecondary)
                Spacer(minLength: 0)
            }
        }
    }

    /// The chip the app shows above a meeting another device recorded.
    struct SampleDeviceChip: View {
        @Environment(\.wTheme) private var theme
        let text: String
        var body: some View {
            HStack(spacing: 0) {
                Text(text).wFont(.caption).foregroundStyle(theme.textSecondary)
                    .padding(.horizontal, 8).padding(.vertical, 2)
                    .background(Capsule().fill(theme.selection.opacity(0.6)))
                Spacer(minLength: 0)
            }
        }
    }

    /// Where the app's editors go while editing: a placeholder, so the layout around them shows.
    struct SampleEditors: View {
        @Environment(\.wTheme) private var theme
        var body: some View {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(["Summary", "Decisions", "Action items", "Open questions"], id: \.self) { title in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(title).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.textSecondary)
                        RoundedRectangle(cornerRadius: 6, style: .continuous).fill(theme.backgroundPage)
                            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(theme.border, lineWidth: 1))
                            .frame(height: title == "Summary" ? 80 : 44)
                    }
                }
            }
            .padding(16).frame(maxWidth: .infinity, alignment: .leading).wCard(radius: .card)
        }
    }

    static func page(_ model: WNoteModel, notice: String? = nil, device: String? = nil) -> some View {
        WNoteView(
            model: model, actions: WNoteActions(),
            options: { WGlyph(name: "ellipsis.circle", size: 16).frame(width: 28, height: 28) },
            notices: {
                VStack(alignment: .leading, spacing: 8) {
                    if let device { SampleDeviceChip(text: device) }
                    if let notice { SampleNotice(text: notice) }
                }
            },
            editor: { SampleEditors() }, extras: { EmptyView() },
            ask: { WLiveGallery.SampleAsk().padding(12) }, speaker: { WNoteSpeakerName($0.speaker) })
            .environment(\.wScrollsContent, false)
    }

    static func screen(_ model: WNoteModel, notice: String? = nil, device: String? = nil, height: CGFloat = 1150) -> AnyView {
        AnyView(
            WShell(
                items: WHomeGallery.items, selection: "meetings", onSelect: { _ in },
                header: WShellHeader(title: "Meetings", ask: .available(placeholder: "Search your work memory")),
                onSubmit: { _, _ in }, memory: .on, onWhatIsCaptured: {}, recording: nil,
                pauseChoices: { WPauseChoices(isPaused: false, onPause: { _ in }, onResume: {}) },
                footer: { Text("This Mac").font(.caption).foregroundStyle(.secondary).padding(.horizontal, 10) },
                banner: { EmptyView() },
                content: { page(model, notice: notice, device: device) }
            )
            .frame(height: height)
        )
    }

    /// A long meeting with many speakers, to show the lanes held to their own height.
    static let crowdedSegments: [WNoteSegment] = (0..<60).map { i in
        seg("x\(i)", i * 40, "Speaker \(i % 20 + 1)", "Line \(i + 1) of a long meeting with many people on the call.", length: 30)
    }

    public static func cases() -> [WHomeGalleryCase] {
        var out: [WHomeGalleryCase] = []
        func add(_ id: String, _ view: AnyView, mode: WGalleryMode = .light, scale: CGFloat = 1, width: CGFloat = 1440, height: CGFloat = 1150) {
            out.append(WHomeGalleryCase(id: id, mode: mode, textScale: scale, width: width, height: height, view: view))
        }
        func both(_ name: String, _ view: @autoclosure () -> AnyView, width: CGFloat = 1440, height: CGFloat = 1150) {
            add("\(name)-light", view(), width: width, height: height)
            add("\(name)-dark", view(), mode: .dark, width: width, height: height)
        }
        both("note-sources", screen(model(), height: 1250), height: 1250)
        both("note-no-sources", screen(model(sources: false)))
        both("note-narrow", AnyView(page(model(selected: 3, back: nil)).frame(height: 1500)), width: 600, height: 1500)
        both("note-processing", screen(model(state: .processing(stage: "Transcribing"))))
        both("note-regenerating", screen(model(state: .processing(stage: "Regenerating\u{2026}"))))
        both("note-other-device", screen(model(selected: nil), device: "Dana's MacBook", height: 1250), height: 1250)
        both("note-editing", screen(model(editing: true), height: 1250), height: 1250)
        both("transcript-lanes", screen(model(mode: .transcript, highlighted: "s8"), height: 1250), height: 1250)
        add("note-failed-light", screen(model(state: .failed("Whimbrel could not write the summary. Check your connection and try again."))))
        add("note-sources-light-2.0", screen(model(), height: 2400), scale: 2, width: 1500, height: 2400)
        add("note-sources-high-contrast", screen(model(), height: 1250), mode: .highContrastLight, height: 1250)
        add("note-sources-light-reading-size", screen(model(readingScale: WReadingScale.scale(fontSize: 18)), height: 1500), height: 1500)
        add("note-editing-high-contrast", screen(model(editing: true), height: 1250), mode: .highContrastLight, height: 1250)
        add("transcript-continued-light", screen(model(mode: .transcript, highlighted: "c2", continued: true, marks: marks + outlyingMarks), height: 1250), height: 1250)
        add("transcript-many-speakers-light-2.0", screen(model(mode: .transcript, marks: [], segments: crowdedSegments), height: 1600), scale: 2, width: 1500, height: 1600)
        return out
    }
}
