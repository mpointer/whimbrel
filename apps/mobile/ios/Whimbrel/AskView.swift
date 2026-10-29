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

    init(sync: SyncModel, model: AskModel, labels: DeviceLabels) {
        self.sync = sync
        self.model = model
        self.labels = labels
        _route = State(initialValue: model.route)
    }

    var body: some View {
        VStack(spacing: 0) {
            content
            if case .ready = route { composer }
        }
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
            empty("Ask on iPhone needs a hosted model. Add an Anthropic, OpenAI, Google or OpenRouter key on your Mac and turn on \"Sync this key\", then pull to refresh.")
        case .localOnly:
            empty("Local-only mode is on, so Ask stays on your Mac.")
        case .ready:
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 20) {
                        if model.turns.isEmpty {
                            Text("Ask about your meetings, screen captures and routine runs.")
                                .foregroundStyle(Color.theme(.textSecondary))
                        }
                        ForEach(model.turns) { turn in
                            TurnView(turn: turn, labels: labels, readAloud: readAloud, openCapture: { captureId = $0 }).id(turn.id)
                        }
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
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
                .foregroundStyle(Color.theme(.textSecondary))
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .refreshable { await sync.syncNow(manual: true) }
    }

    private var composer: some View {
        VStack(spacing: 4) {
            dictationNote
            composerRow
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(Color.theme(.backgroundPage))
    }

    @ViewBuilder private var dictationNote: some View {
        switch dictation.state {
        case .denied:
            VStack(alignment: .leading, spacing: 4) {
                Text("Turn on Microphone and Speech Recognition for Whimbrel in Settings")
                    .font(.footnote).foregroundStyle(Color.theme(.textSecondary))
                Button("Open Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                }
                .font(.subheadline).frame(minHeight: 44)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .unavailable:
            Text("Dictation needs on-device speech recognition, which this iPhone doesn't have for your language.")
                .font(.footnote).foregroundStyle(Color.theme(.textSecondary))
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
            Image(systemName: listening ? "mic.fill" : "mic").frame(width: 44, height: 44)
                .foregroundStyle(listening ? Color.theme(.textOnAccent) : Color.theme(.textPrimary))
                .background(listening ? Color.theme(.accentPrimary) : Color.theme(.backgroundCard), in: Circle())
                .overlay(Circle().stroke(Color.theme(.border), lineWidth: 1))
                .opacity(off ? 0.4 : 1)
        }
        .disabled(off)
        .accessibilityLabel(listening ? "Stop dictation" : "Dictate")
    }

    private var composerRow: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Ask Whimbrel", text: $draft, axis: .vertical)
                .lineLimit(1...5)
                .focused($composerFocused)
                .padding(.horizontal, 12)
                .frame(minHeight: 44)
                .foregroundStyle(Color.theme(.textPrimary))
                .background(Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: 22))
                .overlay(RoundedRectangle(cornerRadius: 22).stroke(Color.theme(.border), lineWidth: 1))
                .submitLabel(.send)
                .onSubmit(send)
                .accessibilityLabel("Question")
            micButton
            if model.isBusy {
                Button { model.stop() } label: {
                    Image(systemName: "stop.fill").frame(width: 44, height: 44)
                        .foregroundStyle(Color.theme(.textOnAccent))
                        .background(Color.theme(.accentPrimary), in: Circle())
                }
                .accessibilityLabel("Stop")
            } else {
                Button(action: send) {
                    Image(systemName: "arrow.up").frame(width: 44, height: 44)
                        .foregroundStyle(Color.theme(.textOnAccent))
                        .background(Color.theme(.accentPrimary), in: Circle())
                        .opacity(canSend ? 1 : 0.4)
                }
                .disabled(!canSend)
                .accessibilityLabel("Send")
            }
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

private struct TurnView: View {
    let turn: AskTurn
    let labels: DeviceLabels
    let readAloud: ReadAloud
    let openCapture: (Int64) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(turn.question)
                .foregroundStyle(Color.theme(.textOnAccent))
                .padding(12)
                .background(Color.theme(.accentPrimary), in: RoundedRectangle(cornerRadius: 16))
                .frame(maxWidth: .infinity, alignment: .trailing)
                .accessibilityLabel("You asked: \(turn.question)")
            answer
        }
    }

    @ViewBuilder private var answer: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !turn.answer.isEmpty {
                Text(turn.answer).foregroundStyle(Color.theme(.textPrimary)).textSelection(.enabled)
            }
            switch turn.state {
            case .streaming where turn.answer.isEmpty:
                ProgressView().accessibilityLabel("Thinking")
            case .streaming, .done: EmptyView()
            case .stopped: note("Stopped")
            case .spendCap: note("Daily spend cap reached on this iPhone")
            case .failed: note("Couldn't get an answer. Try again.", isError: true)
            }
            if !turn.citations.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(turn.citations.enumerated()), id: \.offset) { _, card in
                        CitationChip(card: card, labels: labels, openCapture: openCapture)
                    }
                }
            }
            if turn.state != .streaming, !turn.answer.isEmpty {
                HStack(spacing: 12) {
                    Text(turn.modelLine).font(.caption).foregroundStyle(Color.theme(.textSecondary))
                    Spacer()
                    Button { readAloud.speak(turn.answer) } label: {
                        Label("Read aloud", systemImage: "speaker.wave.2").font(.subheadline)
                            .frame(minWidth: 44, minHeight: 44)
                    }
                    .accessibilityLabel("Read answer aloud")
                    Button { UIPasteboard.general.string = turn.answer } label: {
                        Label("Copy", systemImage: "doc.on.doc").font(.subheadline)
                            .frame(minWidth: 44, minHeight: 44)
                    }
                    .accessibilityLabel("Copy answer")
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.theme(.backgroundCard), in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(Color.theme(.border), lineWidth: 1))
    }

    private func note(_ text: String, isError: Bool = false) -> some View {
        Text(text).font(.subheadline).foregroundStyle(Color.theme(isError ? .error : .textSecondary))
    }
}
