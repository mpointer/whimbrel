import SwiftUI

/// Ask about your meetings, captures and routine runs, answered by the user's own hosted key.
struct AskView: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject var model: AskModel
    let labels: DeviceLabels
    @State private var route: AskRoute
    @State private var draft = ""
    @State private var captureId: Int64?
    @State private var draftBeforeDictation = ""
    @StateObject private var dictation = Dictation()
    @State private var readAloud = ReadAloud()
    @FocusState private var composerFocused: Bool
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.dynamicTypeSize) private var typeSize
    /// The round buttons grow with the text, up to a point, so a larger glyph stays inside its circle.
    @ScaledMetric(relativeTo: .body) private var scaledControl: CGFloat = WTokens.Size.target
    private var control: CGFloat { min(scaledControl, 76) }

    init(sync: SyncModel, model: AskModel, labels: DeviceLabels) {
        self.sync = sync
        self.model = model
        self.labels = labels
        _route = State(initialValue: model.route)
    }

    /// The answer block marks Whimbrel's words with its ink and rule, and the question above it is
    /// read as "You asked", so a screen reader is not also given an English-only prefix.
    private var noteStrings: WStrings {
        var s = WStrings.app
        s.whimbrelWrote = { $0 }
        return s
    }

    var body: some View {
        VStack(spacing: 0) {
            content
            if case .ready = route { composer }
        }
        .environment(\.wStrings, noteStrings)
        .themed()
        .background(Color.theme(.backgroundPage).ignoresSafeArea())
        .navigationTitle("Ask")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(for: CitationTarget.self) { target in
            switch target {
            case .meeting(let id): MeetingDetailView(meetingId: id, sync: sync, labels: labels)
            case .routineRun(let id): RoutineRunLoader(id: id, sync: sync, labels: labels)
            case .capture, .none: EmptyView()
            }
        }
        .sheet(item: Binding(get: { captureId.map(CaptureID.init) }, set: { captureId = $0?.id })) { CaptureSheet(id: $0.id, sync: sync) }
        // A key can arrive one sync round after its toggle: look again whenever one ends.
        .onAppear { route = model.route }
        .onChange(of: sync.isSyncing) { _, syncing in if !syncing { route = model.route } }
        .onDisappear { dictation.stop(); readAloud.stop() }  // the Ask run itself stops on leaving the tab (RootView)
        .onChange(of: scenePhase) { _, phase in if phase == .active { dictation.recheck() } }
        .onChange(of: dictation.transcript) { _, text in
            if !text.isEmpty { draft = Dictation.merge(base: draftBeforeDictation, transcript: text) }
        }
    }

    @ViewBuilder private var content: some View {
        switch route {
        case .noHostedKey:
            empty(String(localized: "Ask on iPhone needs a hosted model. Add an Anthropic, OpenAI, Google or OpenRouter key on your Mac and turn on \"Sync this key\", then pull to refresh."))
        case .localOnly:
            empty(String(localized: "Local-only mode is on, so Ask stays on your Mac."))
        case .ready:
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: WTokens.Space.xl) {
                        if model.turns.isEmpty {
                            Text("Ask about your meetings, screen captures and routine runs.")
                                .wType(WTokens.TypeScale.body)
                                .foregroundStyle(Color.theme(.textSecondary))
                        }
                        ForEach(model.turns) { turn in
                            TurnView(turn: turn, labels: labels, readAloud: readAloud, openCapture: { captureId = $0 }).id(turn.id)
                        }
                    }
                    .padding(.horizontal, WTokens.Space.lg)
                    .padding(.vertical, WTokens.Space.md)
                    .frame(maxWidth: WTokens.Size.contentMax, alignment: .leading)
                    .frame(maxWidth: .infinity)
                }
                .scrollDismissesKeyboard(.interactively)
                .onChange(of: model.turns.last?.answer) { _, _ in
                    if let id = model.turns.last?.id { proxy.scrollTo(id, anchor: .bottom) }
                }
                .onChange(of: model.turns.count) { _, _ in
                    if let id = model.turns.last?.id { proxy.scrollTo(id, anchor: .bottom) }
                }
            }
        }
    }

    /// Pull to refresh runs a sync round, which also pulls the settings and keys.
    private func empty(_ text: String) -> some View {
        ScrollView {
            Text(text)
                .wType(WTokens.TypeScale.body)
                .foregroundStyle(Color.theme(.textPrimary))
                .padding(WTokens.Space.lg)
                .frame(maxWidth: .infinity, alignment: .leading)
                .wCard()
                .padding(.horizontal, WTokens.Space.lg)
                .padding(.vertical, WTokens.Space.md)
                .frame(maxWidth: WTokens.Size.contentMax)
                .frame(maxWidth: .infinity)
        }
        .refreshable { await sync.syncNow(manual: true) }
    }

    private var composer: some View {
        VStack(spacing: WTokens.Space.xs) {
            dictationNote
            composerRow
        }
        .frame(maxWidth: WTokens.Size.contentMax)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, WTokens.Space.lg)
        .padding(.vertical, WTokens.Space.sm)
        .background(Color.theme(.backgroundPage))
    }

    @ViewBuilder private var dictationNote: some View {
        switch dictation.state {
        case .denied:
            VStack(alignment: .leading, spacing: WTokens.Space.xs) {
                Text("Turn on Microphone and Speech Recognition for Whimbrel in Settings")
                    .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
                Button("Open Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                }
                .wType(WTokens.TypeScale.time).frame(minHeight: WTokens.Size.target)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .unavailable:
            Text("Dictation needs on-device speech recognition, which this iPhone doesn't have for your language.")
                .wType(WTokens.TypeScale.secondary).foregroundStyle(Color.theme(.textSecondary))
                .frame(maxWidth: .infinity, alignment: .leading)
        case .idle, .starting, .listening: EmptyView()
        }
    }

    private var micButton: some View {
        let listening = dictation.state == .listening
        let off = dictation.state == .denied || dictation.state == .unavailable || dictation.state == .starting
        return Button {
            if listening { dictation.stop() } else {
                draftBeforeDictation = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                dictation.transcript = ""
                dictation.start()
            }
        } label: {
            Image(systemName: listening ? "mic.fill" : "mic").frame(width: control, height: control)
                .foregroundStyle(listening ? Color.theme(.textOnAccent) : Color.theme(.textPrimary))
                .background(listening ? Color.theme(.accentPrimary) : Color.theme(.backgroundCard), in: Circle())
                .overlay(Circle().stroke(Color.theme(.border), lineWidth: 1))
                .opacity(off ? 0.4 : 1)
        }
        .disabled(off)
        .accessibilityLabel(listening ? "Stop dictation" : "Dictate")
    }

    /// At the accessibility text sizes the field takes the full width and the buttons sit under it,
    /// so the placeholder and what you type are not squeezed into a third of the row.
    @ViewBuilder private var composerRow: some View {
        if typeSize.isAccessibilitySize {
            VStack(alignment: .trailing, spacing: WTokens.Space.sm) {
                questionField
                HStack(spacing: WTokens.Space.sm) { micButton; sendOrStop }
            }
        } else {
            HStack(alignment: .bottom, spacing: WTokens.Space.sm) { questionField; micButton; sendOrStop }
        }
    }

    private var questionField: some View {
            TextField("Ask Whimbrel", text: $draft, axis: .vertical)
                .wType(WTokens.TypeScale.body)
                .lineLimit(1...5)
                .focused($composerFocused)
                .padding(.horizontal, WTokens.Space.md)
                .padding(.vertical, WTokens.Space.sm)
                .frame(minHeight: WTokens.Size.target)
                .foregroundStyle(Color.theme(.textPrimary))
                .background(Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: 22))
                .overlay(RoundedRectangle(cornerRadius: 22).stroke(Color.theme(.border), lineWidth: 1))
                .submitLabel(.send)
                .onSubmit(send)
                .accessibilityLabel("Question")
    }

    @ViewBuilder private var sendOrStop: some View {
            if model.isBusy {
                Button { model.stop() } label: {
                    Image(systemName: "stop.fill").frame(width: control, height: control)
                        .foregroundStyle(Color.theme(.textOnAccent))
                        .background(Color.theme(.accentPrimary), in: Circle())
                }
                .accessibilityLabel("Stop")
            } else {
                Button(action: send) {
                    Image(systemName: "arrow.up").frame(width: control, height: control)
                        .foregroundStyle(Color.theme(.textOnAccent))
                        .background(Color.theme(.accentPrimary), in: Circle())
                        .opacity(canSend ? 1 : 0.4)
                }
                .disabled(!canSend)
                .accessibilityLabel("Send")
            }
    }

    private var canSend: Bool { !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    private func send() {
        guard canSend, !model.isBusy else { return }
        dictation.stop()
        model.send(draft)
        draft = ""
    }
}

private struct CaptureID: Identifiable { let id: Int64 }

/// One question and its answer. The question is in your ink and is the turn's heading; the
/// answer is in Whimbrel's ink with its rule, as on the note page.
private struct TurnView: View {
    let turn: AskTurn
    let labels: DeviceLabels
    let readAloud: ReadAloud
    let openCapture: (Int64) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: WTokens.Space.sm) {
            Text(turn.question)
                .wType(WTokens.TypeScale.subtitle)
                .foregroundStyle(Color.theme(.textPrimary))
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityLabel("You asked: \(turn.question)")
                .accessibilityAddTraits(.isHeader)
            if !turn.answer.isEmpty {
                WNoteBlock(kind: .paragraph, author: .whimbrel, text: turn.answer).textSelection(.enabled)
            }
            switch turn.state {
            case .streaming where turn.answer.isEmpty:
                ProgressView().frame(minHeight: WTokens.Size.target).accessibilityLabel("Thinking")
            case .streaming, .done: EmptyView()
            case .stopped: note(String(localized: "Stopped"))
            case .spendCap: note(String(localized: "Daily spend cap reached on this iPhone"))
            case .failed: note(String(localized: "Couldn't get an answer. Try again."), isError: true)
            }
            if !turn.citations.isEmpty {
                VStack(alignment: .leading, spacing: WTokens.Space.sm) {
                    ForEach(Array(turn.citations.enumerated()), id: \.offset) { _, card in
                        CitationChip(card: card, labels: labels, openCapture: openCapture)
                    }
                }
            }
            if turn.state != .streaming, !turn.answer.isEmpty { footer }
        }
    }

    /// Which model answered and whose key paid, on its own line so large text does not squeeze
    /// it, then the two things you can do with the answer.
    private var footer: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(turn.modelLine).wType(WTokens.TypeScale.caption).foregroundStyle(Color.theme(.textSecondary))
            HStack(spacing: WTokens.Space.lg) {
                Button { readAloud.speak(turn.answer) } label: {
                    Label("Read aloud", systemImage: "speaker.wave.2").wType(WTokens.TypeScale.time)
                        .frame(minWidth: WTokens.Size.target, minHeight: WTokens.Size.target)
                }
                .accessibilityLabel("Read answer aloud")
                Button { UIPasteboard.general.string = turn.answer } label: {
                    Label("Copy", systemImage: "doc.on.doc").wType(WTokens.TypeScale.time)
                        .frame(minWidth: WTokens.Size.target, minHeight: WTokens.Size.target)
                }
                .accessibilityLabel("Copy answer")
            }
        }
    }

    private func note(_ text: String, isError: Bool = false) -> some View {
        Label(text, systemImage: isError ? "exclamationmark.triangle" : "stop.circle")
            .wType(WTokens.TypeScale.secondary)
            .foregroundStyle(Color.theme(isError ? .error : .textSecondary))
    }
}
