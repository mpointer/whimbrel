import SwiftUI

// The app shell: the sidebar with the capture state in its foot, the recording bar at the top of the
// content on every section, and a slot for banners. The sections, their order and their badges come
// from the app; selection stays where the app keeps it.

public struct WShellItem: Identifiable, Equatable, Sendable {
    public let id: String
    public var glyph: String
    public var title: String
    public var badge: String?
    /// "\u{2318}1".
    public var shortcut: String?

    public init(id: String, glyph: String, title: String, badge: String? = nil, shortcut: String? = nil) {
        self.id = id
        self.glyph = glyph
        self.title = title
        self.badge = badge
        self.shortcut = shortcut
    }
}

/// The recording state the bar shows. The bar is drawn only while a meeting is recording, paused or processing.
public struct WShellRecording: Equatable, Sendable {
    public var state: WRecordingState
    public var elapsedSeconds: Int?
    /// While the recording is starting: whether Stop works, is replaced by "Saving...", or is off (the default). See `WStopOffer`.
    public var startingStop: WStopOffer
    public init(state: WRecordingState, elapsedSeconds: Int? = nil, startingStop: WStopOffer = .unavailable) {
        self.state = state
        self.elapsedSeconds = elapsedSeconds
        self.startingStop = startingStop
    }
}

/// The pause choices of the capture state menu: the three timed pauses, and Resume now while paused.
public struct WPauseChoices: View {
    @Environment(\.wStrings) private var strings
    let isPaused: Bool
    let onPause: (CapturePauseChoice) -> Void
    let onResume: () -> Void

    public init(isPaused: Bool, onPause: @escaping (CapturePauseChoice) -> Void, onResume: @escaping () -> Void) {
        self.isPaused = isPaused
        self.onPause = onPause
        self.onResume = onResume
    }

    public var body: some View {
        Button { onPause(.fifteenMinutes) } label: { Label(strings.home.pauseFifteenMinutes, systemImage: "pause") }
        Button { onPause(.oneHour) } label: { Label(strings.home.pauseOneHour, systemImage: "pause") }
        Button { onPause(.untilTomorrow) } label: { Label(strings.home.pauseUntilTomorrow, systemImage: "pause") }
        if isPaused {
            Divider()
            Button(action: onResume) { Label(strings.home.resumeNow, systemImage: "play") }
        }
    }
}

/// A request to set the header field and the chips. The counter makes a repeat of the same values a new request.
public struct WShellPush: Equatable, Sendable {
    public var token: Int
    public var query: String
    public var filter: WHomeFilter
    public init(token: Int, query: String, filter: WHomeFilter) {
        self.token = token
        self.query = query
        self.filter = filter
    }
}

/// The header row: the section's title (and, on Home, the date), then the search bar, then the capture state.
public struct WShellHeader: Equatable, Sendable {
    public var title: String
    public var subtitle: String?
    public var ask: WHomeAsk
    public init(title: String, subtitle: String? = nil, ask: WHomeAsk) {
        self.title = title
        self.subtitle = subtitle
        self.ask = ask
    }
}

/// The "Thread" list in the sidebar (Home only): a part of the line to scroll to.
struct WThreadList: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let entries: [WHomeThreadEntry]
    let onSelect: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: theme.space(8)) {
                WGlyph(name: "clock", size: 13).foregroundStyle(theme.textSecondary)
                Text(strings.home.thread).wFont(.bodyStrong).foregroundStyle(theme.textSecondary)
            }
            .padding(.horizontal, theme.space(10))
            .padding(.top, theme.space(14))
            .padding(.bottom, theme.space(6))
            .accessibilityAddTraits(.isHeader)
            ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                Button { onSelect(entry.id) } label: {
                    HStack(spacing: theme.space(10)) {
                        Circle()
                            .fill(index == 0 ? theme.accentPrimary : theme.backgroundPage)
                            .overlay(Circle().strokeBorder(theme.accentPrimary, lineWidth: 2))
                            .frame(width: 9 * k, height: 9 * k)
                            .frame(width: 20 * k)
                            .accessibilityHidden(true)
                        Text(entry.title).wFont(index == 0 ? .bodyStrong : .body).foregroundStyle(theme.textPrimary).lineLimit(1)
                        Spacer(minLength: theme.space(4))
                        Text(entry.detail).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                    }
                    .padding(.horizontal, theme.space(10))
                    .frame(minHeight: WTokens.Size.target * k)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(strings.joinDetails([entry.title, entry.detail]))
            }
        }
        .accessibilityElement(children: .contain)
    }
}

/// The header row of the content area: title on the left, search in the middle, capture state on the right.
struct WShellHeaderRow<Choices: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let header: WShellHeader
    let askStyle: WAskStyle
    let memory: WMemoryState
    let onWhatIsCaptured: () -> Void
    let onSetUpAsk: () -> Void
    let onSubmit: (String, WHomeFilter) -> Void
    @Binding var query: String
    @Binding var filter: WHomeFilter
    let focus: FocusState<Bool>.Binding
    let choices: Choices
    /// A button beside the field that asks with its text ("Ask", with the Command Return keycap). Nil draws none.
    let onAskButton: ((String, WHomeFilter) -> Void)?
    /// An answer is being written: the Ask button stays, switched off, and says why to assistive technology.
    let askBusy: Bool
    @Environment(\.wStrings) private var strings

    var body: some View {
        HStack(alignment: .center, spacing: theme.space(16)) {
            VStack(alignment: .leading, spacing: theme.space(2)) {
                Text(header.title).wFont(.title).foregroundStyle(theme.textPrimary).lineLimit(1).accessibilityAddTraits(.isHeader)
                if let subtitle = header.subtitle {
                    Text(subtitle).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                }
            }
            .fixedSize(horizontal: true, vertical: false)
            Group {
                switch header.ask {
                case .available(let placeholder):
                    if askStyle == .prompt {
                        WPromptField(text: $query, placeholder: placeholder, focus: focus, onSubmit: submit)
                    } else {
                        WSearchBar(text: $query, placeholder: placeholder, focus: focus, onSubmit: submit)
                    }
                case .needsSetup(let title):
                    WAction(title, glyph: "sparkles", kind: .secondary, action: onSetUpAsk)
                }
            }
            .frame(maxWidth: 520 * k)
            if let onAskButton, case .available = header.ask {
                let empty = query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                WButton(strings.find.askButton, glyph: "sparkles", shortcutHint: strings.find.askShortcutHint, kind: .secondary) {
                    onAskButton(query.trimmingCharacters(in: .whitespacesAndNewlines), filter)
                }
                .disabled(empty || askBusy)
                .accessibilityHint(askBusy ? strings.find.askBusyReason : "")
                .help(askBusy ? strings.find.askBusyReason : "")
            }
            Spacer(minLength: 0)
            WCaptureState(state: memory, onWhatIsCaptured: onWhatIsCaptured) { choices }
        }
    }

    private func submit() {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !filter.isEmpty else { return }
        onSubmit(trimmed, filter)
    }
}

public struct WShell<Choices: View, Footer: View, Banner: View, Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @State private var ownQuery = ""
    @State private var ownFilter = WHomeFilter()
    @FocusState private var fieldFocused: Bool
    let items: [WShellItem]
    let selection: String
    let onSelect: (String) -> Void
    let header: WShellHeader
    let elements: WHomeElements
    let people: [String]
    let appChoices: [WHomeAppChoice]
    let showsFilters: Bool
    let onSubmit: (String, WHomeFilter) -> Void
    let onSetUpAsk: () -> Void
    let thread: [WHomeThreadEntry]
    let onThread: (String) -> Void
    let memory: WMemoryState
    let onWhatIsCaptured: () -> Void
    let recording: WShellRecording?
    let level: Float?
    let onPause: (() -> Void)?
    let onResume: (() -> Void)?
    let onStop: (() -> Void)?
    let choices: Choices
    let footer: Footer
    let banner: Banner
    let content: Content
    /// Told on each change of the field and of the chips (Library's Find page keeps a copy, without redrawing the shell on a keystroke).
    let onQueryChange: ((String) -> Void)?
    let onFilterChange: ((WHomeFilter) -> Void)?
    /// A new `token` sets the field and the chips to these (the page starting a search of its own).
    let push: WShellPush?
    /// Cmd Return in the window: the text and filters as they are. Nil leaves the key alone.
    let onAsk: ((String, WHomeFilter) -> Void)?
    /// A change moves the keyboard focus to the field (the app's Search Library command).
    let focusRequest: Int
    /// At the right of the filter chips ("3 results").
    let filterTrailing: String?
    /// Command Return asks only while the header field has the keyboard, or while this is true (the Find page is showing). Anywhere else the
    /// key belongs to what is there: an editor, a sheet.
    let askKeyActive: Bool
    /// Draws an Ask button beside the field (Home and the other sections, where Enter only searches).
    let showsAskButton: Bool
    /// An answer is being written on Find: the Ask button beside the field is off, with the reason available to assistive technology.
    let askBusy: Bool

    public init(
        items: [WShellItem], selection: String, onSelect: @escaping (String) -> Void,
        header: WShellHeader, elements: WHomeElements = WHomeElements(),
        people: [String] = [], appChoices: [WHomeAppChoice] = [], showsFilters: Bool = false,
        onSubmit: @escaping (String, WHomeFilter) -> Void, onSetUpAsk: @escaping () -> Void = {},
        thread: [WHomeThreadEntry] = [], onThread: @escaping (String) -> Void = { _ in },
        memory: WMemoryState, onWhatIsCaptured: @escaping () -> Void,
        recording: WShellRecording? = nil, level: Float? = nil, onPause: (() -> Void)? = nil, onResume: (() -> Void)? = nil, onStop: (() -> Void)? = nil,
        @ViewBuilder pauseChoices: () -> Choices, @ViewBuilder footer: () -> Footer,
        @ViewBuilder banner: () -> Banner, @ViewBuilder content: () -> Content,
        initialQuery: String = "", initialFilter: WHomeFilter = WHomeFilter(),
        onQueryChange: ((String) -> Void)? = nil, onFilterChange: ((WHomeFilter) -> Void)? = nil, push: WShellPush? = nil,
        onAsk: ((String, WHomeFilter) -> Void)? = nil, focusRequest: Int = 0, filterTrailing: String? = nil,
        askKeyActive: Bool = false, showsAskButton: Bool = false, askBusy: Bool = false
    ) {
        self.askBusy = askBusy
        self.askKeyActive = askKeyActive
        self.showsAskButton = showsAskButton
        self._ownQuery = State(initialValue: initialQuery)
        self._ownFilter = State(initialValue: initialFilter)
        self.onQueryChange = onQueryChange
        self.onFilterChange = onFilterChange
        self.push = push
        self.onAsk = onAsk
        self.focusRequest = focusRequest
        self.filterTrailing = filterTrailing
        self.items = items
        self.selection = selection
        self.onSelect = onSelect
        self.header = header
        self.elements = elements
        self.people = people
        self.appChoices = appChoices
        self.showsFilters = showsFilters
        self.onSubmit = onSubmit
        self.onSetUpAsk = onSetUpAsk
        self.thread = thread
        self.onThread = onThread
        self.memory = memory
        self.onWhatIsCaptured = onWhatIsCaptured
        self.recording = recording
        self.level = level
        self.onPause = onPause
        self.onResume = onResume
        self.onStop = onStop
        self.choices = pauseChoices()
        self.footer = footer()
        self.banner = banner()
        self.content = content()
    }

    private var query: Binding<String> { $ownQuery }
    private var filter: Binding<WHomeFilter> { $ownFilter }

    private var searchAvailable: Bool {
        if case .available = header.ask { return true } else { return false }
    }

    public var body: some View {
        HStack(spacing: 0) {
            WSidebar {
                VStack(alignment: .leading, spacing: theme.space(2)) {
                    VStack(alignment: .leading, spacing: theme.space(2)) {
                        ForEach(items) { item in
                            WSidebarItem(icon: item.glyph, label: item.title, badge: item.badge, shortcut: item.shortcut, isSelected: item.id == selection) { onSelect(item.id) }
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel(strings.home.sections)
                    if !thread.isEmpty { WThreadList(entries: thread, onSelect: onThread) }
                }
            } footer: {
                footer
            }
            .padding(theme.space(8))
            VStack(spacing: 0) {
                banner
                if let recording {
                    WRecordingIndicator(
                        state: recording.state, elapsedSeconds: recording.elapsedSeconds, size: .bar, level: level,
                        onPause: onPause, onResume: onResume, onStop: onStop, startingStop: recording.startingStop
                    )
                    .padding(.horizontal, theme.space(20))
                    .padding(.top, theme.space(12))
                }
                VStack(alignment: .leading, spacing: theme.space(12)) {
                    WShellHeaderRow(
                        header: header, askStyle: elements.ask, memory: memory, onWhatIsCaptured: onWhatIsCaptured,
                        onSetUpAsk: onSetUpAsk, onSubmit: onSubmit, query: query, filter: filter, focus: $fieldFocused, choices: choices,
                        onAskButton: showsAskButton ? onAsk : nil, askBusy: askBusy)
                    if showsFilters && searchAvailable { WHomeFilterChips(people: people, apps: appChoices, filter: filter, trailing: filterTrailing) }
                }
                .padding(.horizontal, theme.space(20))
                .padding(.vertical, theme.space(12))
                .environment(\.wActionStyle, elements.actions)
                content
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .wPageBackground()
        }
        .background(theme.backgroundPage)
        .background {
            // Command K puts the keyboard in the field, as the keycap on it says. Command Return asks, where the app handles it.
            Button("") { if searchAvailable { fieldFocused = true } }
                .keyboardShortcut("k", modifiers: .command)
                .opacity(0).frame(width: 0, height: 0).accessibilityHidden(true)
            if let onAsk, fieldFocused || askKeyActive {
                Button("") {
                    let trimmed = query.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines)
                    onAsk(trimmed, filter.wrappedValue)
                }
                .keyboardShortcut(.return, modifiers: .command)
                .opacity(0).frame(width: 0, height: 0).accessibilityHidden(true)
            }
        }
        .onChange(of: focusRequest) { _, _ in if searchAvailable { fieldFocused = true } }
        .onChange(of: ownQuery) { _, new in onQueryChange?(new) }
        .onChange(of: ownFilter) { _, new in onFilterChange?(new) }
        .onChange(of: push) { _, new in
            guard let new else { return }
            ownQuery = new.query
            ownFilter = new.filter
        }
    }
}
