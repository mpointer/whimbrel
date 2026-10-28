import AppKit
import SwiftUI
import os

/// Library's Ask mode: the conversation sidebar, the chat transcript, and the
/// blocking and streaming ask paths.
extension TimelineView {
    @ViewBuilder
    var usageTodayLine: some View {
        if appState.routing.userDailyCapCents != 0, let spendTodayCents {
            let capCents = appState.routing.userDailyCapCents.map(Int64.init) ?? RoutingSettings.defaultUserDailyCapCents
            Text("\(spendTodayCents)¢ / \(capCents)¢ today")
                .font(.whimbrelBody(.caption2))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
    }

    var conversationSidebar: some View {
        List {
            ForEach(sortedForSidebar(conversationList)) { conversation in
                HStack {
                    Text(conversation.title)
                        .lineLimit(1)
                        .fontWeight(conversation.id == activeConversationId ? .semibold : .regular)
                    Spacer()
                    Button {
                        toggleStarred(conversation)
                    } label: {
                        Image(systemName: conversation.starred ? "star.fill" : "star")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(conversation.starred ? "Unstar conversation" : "Star conversation")
                    .help(conversation.starred ? "Unstar" : "Star")
                }
                .contentShape(Rectangle())
                .onTapGesture { openConversation(conversation) }
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
                .accessibilityAction { openConversation(conversation) }
                .contextMenu {
                    Button("Rename") {
                        renameText = conversation.title
                        renamingConversation = conversation
                    }
                    Button("Delete", role: .destructive) { deleteConversation(conversation) }
                }
            }
        }
        .frame(width: 220)
        .alert("Rename Conversation", isPresented: Binding(
            get: { renamingConversation != nil },
            set: { if !$0 { renamingConversation = nil } }
        ), presenting: renamingConversation) { conversation in
            TextField("Title", text: $renameText)
            Button("Save") { renameConversation(conversation, newTitle: renameText) }
            Button("Cancel", role: .cancel) {}
        }
    }

    func refreshConversationList() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let list = store.listConversations() ?? []
            DispatchQueue.main.async {
                conversationList = list
            }
        }
    }

    func toggleStarred(_ conversation: CoreMemoryStore.ConversationSummary) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.setConversationStarred(id: conversation.id, starred: !conversation.starred)
            DispatchQueue.main.async { refreshConversationList() }
        }
    }

    func renameConversation(_ conversation: CoreMemoryStore.ConversationSummary, newTitle: String) {
        renamingConversation = nil
        let trimmedTitle = newTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let store = appState.coreStore, !trimmedTitle.isEmpty else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.renameConversation(id: conversation.id, newTitle: trimmedTitle)
            DispatchQueue.main.async { refreshConversationList() }
        }
    }

    func deleteConversation(_ conversation: CoreMemoryStore.ConversationSummary) {
        guard let store = appState.coreStore else { return }
        if conversation.id == activeConversationId {
            exchanges = []
            activeConversationId = nil
        }
        CoreMemoryStore.accessQueue.async {
            _ = store.deleteConversation(id: conversation.id)
            DispatchQueue.main.async { refreshConversationList() }
        }
    }

    func openConversation(_ conversation: CoreMemoryStore.ConversationSummary) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let detail = store.getConversation(id: conversation.id)
            DispatchQueue.main.async {
                guard let detail else { return }
                activeConversationId = detail.id
                exchanges = detail.exchanges.map { row in
                    // Exchanges saved before citations were stored hold "[]".
                    let saved = (try? JSONDecoder().decode([SavedCitation].self, from: Data(row.citationsJson.utf8))) ?? []
                    return ChatExchange(
                        query: row.query,
                        result: .success(CoreChatAnswer(text: row.answer, citations: saved.map(\.item))),
                        contextItems: saved.map(ContextDisplayItem.saved),
                        modelLabel: row.model
                    )
                }
            }
        }
    }

    @ViewBuilder
    var askModeBody: some View {
        if exchanges.isEmpty && !isAsking {
            Text("Ask a question about your captured timeline, answered \(answeringProviderPhrase), with citations back to specific items. Follow-up questions see the earlier exchange, so \"what about the other one?\" works.")
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 40)
        } else {
            // Bug fixed 2026-08-06: this ScrollView never scrolled to reveal new content
            // — a query bubble plus a real answer routinely push the conversation taller
            // than the visible viewport, and with no ScrollViewReader/scrollTo, the view
            // just stayed wherever it was. A user watching a fresh (empty) conversation
            // would see only the very top edge of the new answer bubble peek into view
            // once it arrived, easy to mistake for a blank/empty response rather than
            // "the real answer, scrolled below the fold." `bottomAnchorID` is scrolled to
            // on every exchange-count change (a question was just sent) and on every
            // `isAsking` transition (the loading spinner appeared, or got replaced by the
            // real answer, which may be a different height).
            ScrollViewReader { scrollProxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: Spacing.lg) {
                        ForEach(exchanges) { exchange in
                            VStack(alignment: .leading, spacing: Spacing.xs) {
                                HStack {
                                    Spacer()
                                    queryBubble(exchange.query)
                                }
                                HStack {
                                    answerBubble(for: exchange)
                                    Spacer()
                                }
                            }
                        }
                        if isAsking {
                            HStack {
                                if pendingStreamAsk != nil {
                                    streamingAnswerBubble
                                } else {
                                    askingIndicatorBubble
                                }
                                Spacer()
                            }
                        }
                        Color.clear.frame(height: 1).id(Self.askScrollBottomAnchorID)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .onChange(of: exchanges.count) { _, _ in
                    withAnimation { scrollProxy.scrollTo(Self.askScrollBottomAnchorID, anchor: .bottom) }
                }
                .onChange(of: streamingAsk.streamedText) { _, _ in
                    withAnimation { scrollProxy.scrollTo(Self.askScrollBottomAnchorID, anchor: .bottom) }
                }
                .onChange(of: streamingAsk.phase) { _, phase in
                    handleStreamingAskPhaseChange(phase)
                }
                .onChange(of: isAsking) { _, _ in
                    withAnimation { scrollProxy.scrollTo(Self.askScrollBottomAnchorID, anchor: .bottom) }
                }
            }
        }
    }

    /// #75: chat answers were rendering as literal `**bold**`/`*   item` Markdown
    /// syntax instead of formatted text. `.returnPartiallyParsedIfPossible` (rather
    /// than the default `.throwOnError`) keeps as much real formatting as it can
    /// even if the model emits slightly malformed Markdown, instead of falling back
    /// to zero formatting on any single parse hiccup.
    static func markdownAttributedString(_ text: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.failurePolicy = .returnPartiallyParsedIfPossible
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }

    func queryBubble(_ text: String) -> some View {
        Text(text)
            .font(.whimbrelBody(.callout))
            .foregroundStyle(Color.whimbrelTextOnAccent)
            .textSelection(.enabled)
            .padding(Spacing.md)
            .background(Color.whimbrelAccentPrimary)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .frame(maxWidth: 380, alignment: .trailing)
    }

    @ViewBuilder
    func answerBubble(for exchange: ChatExchange) -> some View {
        switch exchange.result {
        case .success(let answer):
            VStack(alignment: .leading, spacing: Spacing.xs) {
                Text(Self.markdownAttributedString(answer.text))
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .textSelection(.enabled)
                if !answer.citations.isEmpty {
                    WhimbrelSectionLabel(text: "Sources")
                        .padding(.top, Spacing.xs)
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        ForEach(citedItems(for: exchange)) { item in
                            let open: (() -> Void)? = {
                                guard let windowId = item.detailWindowId, let value = item.detailWindowValue else { return nil }
                                return { openWindow(id: windowId, value: value) }
                            }()
                            WhimbrelSourceChip(systemImage: item.systemImage, text: item.chipText, thumbnailPath: item.thumbnailPath, onTap: open)
                                .help("\(item.displayLabel): \(item.displayText)")
                        }
                    }
                }
                if let modelLabel = exchange.modelLabel, !modelLabel.isEmpty {
                    Text(modelLabel)
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
            .whimbrelCard()
            .frame(maxWidth: 380, alignment: .leading)
        case .failure(let error):
            Text(error.userFacingMessage)
                .font(.whimbrelBody(.callout))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .whimbrelCard()
                .frame(maxWidth: 380, alignment: .leading)
        }
    }

    var askingIndicatorBubble: some View {
        ProgressView("Asking \(inFlightModelLabel)…")
            .font(.whimbrelBody(.callout))
            .whimbrelCard()
            .frame(maxWidth: 380, alignment: .leading)
    }

    /// #74: shown in place of `askingIndicatorBubble` while `pendingStreamAsk` is set —
    /// live streamed text (plain `Text`, not `markdownAttributedString` — half-parsed
    /// Markdown mid-stream looks broken; the final answer bubble gets real formatting
    /// once `finalizeStreamingAnswer` appends it as a normal `ChatExchange`), plus a Stop
    /// button that's the actual answer to "cancel and restart with a different model" —
    /// cancelling restores the question to the composer so the user can resubmit.
    var streamingAnswerBubble: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            if streamingAsk.streamedText.isEmpty {
                ProgressView("Asking \(inFlightModelLabel)…")
                    .font(.whimbrelBody(.callout))
            } else {
                Text(streamingAsk.streamedText)
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextPrimary)
                    .textSelection(.enabled)
            }
            Button("Stop") { streamingAsk.cancel() }
                .buttonStyle(.plain)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelAccentPrimary)
        }
        .whimbrelCard()
        .frame(maxWidth: 380, alignment: .leading)
    }

    func citedItems(for exchange: ChatExchange) -> [ContextDisplayItem] {
        guard case .success(let answer) = exchange.result else { return [] }
        return exchange.contextItems.filter { item in answer.citations.contains { item.matchesRef($0) } }
    }

    func sendLibraryPrompt(_ prompt: LibraryPrompt) {
        query = prompt.query
        refresh()
    }

    func askIfNeeded(store: CoreMemoryStore) {
        guard !query.isEmpty, !isBusy else { return }
        let capturedQuery = query
        let capturedOllamaModel = ollamaModel
        let capturedActiveConversationId = activeConversationId
        let isFirstMessageInConversation = capturedActiveConversationId == nil
        let effective = effectiveAskRoutingProvider(
            query: capturedQuery,
            pickerProvider: selectedProvider,
            pickerModelLabel: currentModelLabel,
            pickerTouched: providerPickerTouched,
            defaults: appState.routing.defaultSelection()
        )
        // FR-CHAT-6: title-gen is never time-critical by definition (that's the whole
        // point of running it as a background, best-effort call) -- give it its own
        // routing decision via the sentinel query above instead of reusing `effective`,
        // which reflects the ORIGINAL question's classification and could route to a
        // hosted provider if that question happened to be time-critical while the
        // picker was still at its default.
        let titleGenEffective = effectiveAskRoutingProvider(
            query: Self.nonTimeCriticalSentinelQuery,
            pickerProvider: selectedProvider,
            pickerModelLabel: currentModelLabel,
            pickerTouched: providerPickerTouched,
            defaults: appState.routing.defaultSelection()
        )
        let routing = appState.routing.routingConfig(
            preferredProvider: effective.provider, preferredModel: effective.modelLabel,
            customInstructions: appState.promptLibrary.instructions[.ask] ?? ""
        )
        // A failed exchange never produced a real answer, so it contributes nothing to
        // the conversation history replayed back to the model -- re-including a bare
        // "User" turn with no matching "Assistant" turn would be a malformed transcript.
        let history: [CoreChatTurn] = exchanges.flatMap { exchange -> [CoreChatTurn] in
            guard case .success(let answer) = exchange.result else { return [] }
            return [CoreChatTurn(role: .user, text: exchange.query), CoreChatTurn(role: .assistant, text: answer.text)]
        }
        query = ""
        inFlightModelLabel = effective.modelLabel
        isAsking = true
        // #74/#83: every provider streams (the core has NDJSON and SSE adapters).
        // Local-only mode never streams to a hosted provider -- the blocking path's
        // `build_chain` forces Ollama in that mode, and streaming must not leak past it.
        // A hosted route with no key falls to blocking, whose chain builder already
        // handles a missing key by falling back rather than failing.
        let streamRoute: (provider: InferenceStreamClient.Provider, apiKey: String?)? = {
            let hosted = { (provider: InferenceStreamClient.Provider, keychain: KeychainService.Provider) -> (InferenceStreamClient.Provider, String?)? in
                guard !appState.routing.localOnlyMode, let key = KeychainService.load(for: keychain), !key.isEmpty else { return nil }
                return (provider, key)
            }
            switch effective.provider {
            case .ollama: return (.ollama, nil)
            case .google: return hosted(.google, .google)
            case .anthropic: return hosted(.anthropic, .anthropic)
            case .openai: return hosted(.openai, .openai)
            }
        }()
        if let streamRoute {
            askStreaming(
                store: store,
                capturedQuery: capturedQuery,
                capturedOllamaModel: capturedOllamaModel,
                streamProvider: streamRoute.provider,
                streamModel: effective.modelLabel,
                streamAPIKey: streamRoute.apiKey,
                capturedActiveConversationId: capturedActiveConversationId,
                isFirstMessageInConversation: isFirstMessageInConversation,
                routing: routing,
                history: history,
                titleGenEffective: titleGenEffective
            )
        } else {
            askBlocking(
                capturedQuery: capturedQuery,
                capturedOllamaModel: capturedOllamaModel,
                capturedActiveConversationId: capturedActiveConversationId,
                isFirstMessageInConversation: isFirstMessageInConversation,
                routing: routing,
                history: history,
                titleGenEffective: titleGenEffective
            )
        }
    }

    /// Best-effort local resolver for citation display only -- NOT a mirror of
    /// core/llm's actual assemble_context algorithm (RRF fusion across capture items,
    /// transcript segments, routine runs, episodes, and threads, with recency decay).
    /// Fetched here so citedItems(for:) can resolve refs without a second FFI hop; if a
    /// citation's kind/id isn't among these candidates it just won't resolve to a
    /// display row. Retrieval is per-turn -- it uses this question's text alone, not the
    /// earlier conversation (see `CoreMemoryStore.askQuestion`'s docs). Must run on
    /// `CoreMemoryStore.accessQueue` alongside whatever answer call runs next, since both
    /// touch the same non-thread-safe `coreStore` handle.
    func fetchContextItems(store: CoreMemoryStore, query: String) -> [ContextDisplayItem] {
        var contextItems: [ContextDisplayItem] = []
        contextItems += store.searchKeyword(query, limit: 5).map(ContextDisplayItem.capture)
        contextItems += store.searchTranscriptKeyword(query, limit: 5).map(ContextDisplayItem.transcript)
        contextItems += store.searchSnapshotKeyword(query, limit: 5).map(ContextDisplayItem.snapshot)
        if contextItems.isEmpty {
            contextItems += store.searchSemantic(query, limit: 5).map(ContextDisplayItem.capture)
            contextItems += store.searchTranscriptSemantic(query, limit: 5).map(ContextDisplayItem.transcript)
            contextItems += store.searchSnapshotSemantic(query, limit: 5).map(ContextDisplayItem.snapshot)
        }
        contextItems += store.recentRoutineRuns(limit: 5)?.map(ContextDisplayItem.routineRun) ?? []
        contextItems += store.recentEpisodes(limit: 10)?.map(ContextDisplayItem.episode) ?? []
        contextItems += store.threadsNeedingAttention(limit: 10)?.map(ContextDisplayItem.thread) ?? []
        return contextItems
    }

    /// The original (pre-#74) blocking ask path, unchanged in behavior -- still used
    /// directly for every non-Ollama route, and reused as `askStreaming`'s fallback when
    /// the stream never produced any output (see `handleStreamingAskPhaseChange`).
    func askBlocking(
        capturedQuery: String,
        capturedOllamaModel: String,
        capturedActiveConversationId: Int64?,
        isFirstMessageInConversation: Bool,
        routing: CoreRoutingConfig,
        history: [CoreChatTurn],
        titleGenEffective: (provider: RoutingSettings.Provider, modelLabel: String)
    ) {
        // The ask holds a store across a model call, so it runs on the slow lane.
        guard let store = appState.slowStore else {
            isAsking = false
            return
        }
        CoreMemoryStore.slowLaneQueue.async {
            let contextItems = fetchContextItems(store: store, query: capturedQuery)
            let outcome = store.askQuestion(capturedQuery, model: capturedOllamaModel, contextLimit: 5, history: history, routing: routing)
            DispatchQueue.main.async {
                isAsking = false
                guard let outcome else {
                    // Genuinely invalid usage (null handle, malformed args) -- shouldn't
                    // happen in practice; matches the prior fully-silent behavior for
                    // this narrow case rather than showing a misleading message.
                    return
                }
                if case .failure = outcome {
                    query = capturedQuery
                }
                // Display-layer mirror of the same race the DB-write path already guards
                // against: if the user switched to a different conversation (or started a
                // new one) while this question was in flight, this answer no longer
                // belongs on screen here -- it's still correctly persisted to its own
                // conversation by persistExchange below regardless of this check.
                let exchange = ChatExchange(query: capturedQuery, result: outcome, contextItems: contextItems, modelLabel: inFlightModelLabel)
                if activeConversationId == capturedActiveConversationId {
                    exchanges.append(exchange)
                }
                refreshSpendToday()
                if case .success(let answer) = outcome {
                    persistExchange(
                        query: capturedQuery,
                        answer: answer.text,
                        citations: citedItems(for: exchange),
                        model: exchange.modelLabel,
                        isFirstMessageInConversation: isFirstMessageInConversation,
                        capturedConversationId: capturedActiveConversationId,
                        capturedOllamaModel: capturedOllamaModel,
                        titleGenEffective: titleGenEffective
                    )
                }
            }
        }
    }

    /// #74: prepares the same prompt `askBlocking` would have sent (via
    /// `whimbrel_llm_prepare_ask`), then streams the actual model call through
    /// `streamingAsk` (`InferenceStreamClient`, off `CoreMemoryStore.accessQueue`,
    /// cancellable) instead of blocking on it. `handleStreamingAskPhaseChange` (wired via
    /// `.onChange(of: streamingAsk.phase)` in `askModeBody`) picks up the result once the
    /// stream reaches a terminal state.
    func askStreaming(
        store: CoreMemoryStore,
        capturedQuery: String,
        capturedOllamaModel: String,
        streamProvider: InferenceStreamClient.Provider,
        streamModel: String,
        streamAPIKey: String?,
        capturedActiveConversationId: Int64?,
        isFirstMessageInConversation: Bool,
        routing: CoreRoutingConfig,
        history: [CoreChatTurn],
        titleGenEffective: (provider: RoutingSettings.Provider, modelLabel: String)
    ) {
        CoreMemoryStore.accessQueue.async {
            let contextItems = fetchContextItems(store: store, query: capturedQuery)
            let prepared = store.prepareAsk(capturedQuery, contextLimit: 5, history: history, routing: routing)
            DispatchQueue.main.async {
                let fallbackToBlocking = {
                    askBlocking(
                        capturedQuery: capturedQuery,
                        capturedOllamaModel: capturedOllamaModel,
                        capturedActiveConversationId: capturedActiveConversationId,
                        isFirstMessageInConversation: isFirstMessageInConversation,
                        routing: routing,
                        history: history,
                        titleGenEffective: titleGenEffective
                    )
                }
                guard let prepared else {
                    isAsking = false
                    return
                }
                switch prepared {
                case .failure(let error):
                    isAsking = false
                    query = capturedQuery
                    if activeConversationId == capturedActiveConversationId {
                        exchanges.append(ChatExchange(query: capturedQuery, result: .failure(error), contextItems: contextItems))
                    }
                case .success(let data):
                    pendingStreamAsk = PendingStreamAsk(
                        prepareId: data.prepareId,
                        query: capturedQuery,
                        contextItems: contextItems,
                        activeConversationId: capturedActiveConversationId,
                        isFirstMessageInConversation: isFirstMessageInConversation,
                        ollamaModel: capturedOllamaModel,
                        answerModel: streamModel,
                        titleGenEffective: titleGenEffective,
                        fallbackToBlocking: fallbackToBlocking
                    )
                    streamingAsk.start(prompt: data.prompt, model: streamModel, provider: streamProvider, apiKey: streamAPIKey)
                }
            }
        }
    }

    /// Reacts to `streamingAsk.phase` reaching a terminal state (wired via
    /// `.onChange(of: streamingAsk.phase)`). `.queued`/`.connecting`/`.streaming`/
    /// `.cancelling` fall through `default` -- nothing to do until a run actually ends.
    func handleStreamingAskPhaseChange(_ phase: StreamingAskViewModel.Phase) {
        guard let pending = pendingStreamAsk else { return }
        switch phase {
        case .completed:
            isAsking = false
            pendingStreamAsk = nil
            finalizeStreamingAnswer(pending: pending, finalText: streamingAsk.streamedText)
        case .cancelled:
            // No failure bubble, no partial answer kept -- this IS the issue's "cancel
            // and restart with a different model": the question goes back into the
            // composer so the user can pick a different model and resubmit.
            appState.coreStore?.discardAsk(prepareId: pending.prepareId)
            isAsking = false
            pendingStreamAsk = nil
            query = pending.query
        case .failed:
            pendingStreamAsk = nil
            if streamingAsk.streamedText.isEmpty {
                // Never emitted any output -- safe to retry transparently via the
                // blocking path (e.g. Ollama not running), rather than surfacing an
                // error for something the old blocking-only path would have shown a
                // real answer for via its own retry/fallback. Leave isAsking = true:
                // fallbackToBlocking's askBlocking is the one that clears it, once it
                // actually finishes -- clearing it here would open a gap where isBusy
                // reads false while the fallback request is still in flight.
                appState.coreStore?.discardAsk(prepareId: pending.prepareId)
                pending.fallbackToBlocking()
            } else {
                isAsking = false
                // Output was already emitted -- never silently fall back once that's
                // true (same rule StreamingRunSupervisor enforces server-side, docs/
                // adr/0002). Surface it instead of pretending nothing happened. This
                // prepare_id is never finalized either, so discard it here too.
                appState.coreStore?.discardAsk(prepareId: pending.prepareId)
                query = pending.query
                if activeConversationId == pending.activeConversationId {
                    exchanges.append(ChatExchange(query: pending.query, result: .failure(.providerFailure), contextItems: pending.contextItems))
                }
            }
        default:
            break
        }
    }

    /// #74: the citation-extraction + persistence tail of a completed streaming answer —
    /// mirrors `askBlocking`'s own success tail exactly (same `exchanges.append` +
    /// `refreshSpendToday` + `persistExchange` shape), just reached via
    /// `whimbrel_llm_finalize_ask` instead of `askQuestion`'s already-attached citations.
    /// Token counts come from the run's terminal event (#83: Gemini is paid, so they
    /// feed the spend ledger); -1 means the provider reported none.
    func finalizeStreamingAnswer(pending: PendingStreamAsk, finalText: String) {
        guard let store = appState.coreStore else { return }
        let usage = streamingAsk.usage
        CoreMemoryStore.accessQueue.async {
            let citations = store.finalizeAsk(
                prepareId: pending.prepareId,
                finalText: finalText,
                model: pending.answerModel,
                inputTokens: usage.inputTokens ?? -1,
                outputTokens: usage.outputTokens ?? -1
            ) ?? []
            DispatchQueue.main.async {
                let answer = CoreChatAnswer(text: finalText, citations: citations)
                let exchange = ChatExchange(query: pending.query, result: .success(answer), contextItems: pending.contextItems, modelLabel: inFlightModelLabel)
                if activeConversationId == pending.activeConversationId {
                    exchanges.append(exchange)
                }
                refreshSpendToday()
                persistExchange(
                    query: pending.query,
                    answer: answer.text,
                    citations: citedItems(for: exchange),
                    model: exchange.modelLabel,
                    isFirstMessageInConversation: pending.isFirstMessageInConversation,
                    capturedConversationId: pending.activeConversationId,
                    capturedOllamaModel: pending.ollamaModel,
                    titleGenEffective: pending.titleGenEffective
                )
            }
        }
    }

    /// FR-CHAT-6: fires after a successful Ask-mode exchange. Creates the conversation
    /// on the first message of a fresh session (see `isFirstMessageInConversation`,
    /// captured in `askIfNeeded` before this exchange started), appends the exchange
    /// either way, then — only for the first message — kicks off background
    /// title-generation. All of this is fire-and-forget from the UI's perspective: a
    /// failure anywhere here never surfaces to the user, matching the design's
    /// "never blocks the chat UI" requirement.
    ///
    /// `capturedConversationId` is `activeConversationId` as it was when this exchange's
    /// question was actually sent (captured in `askIfNeeded`, same moment as
    /// `isFirstMessageInConversation`), not read live here -- the sidebar has no
    /// `.disabled(isBusy)` guard, so the user can switch to a different conversation (or
    /// start a new one) while this exchange is still in flight. Reading `activeConversationId`
    /// live at completion time would misfile this exchange onto whatever conversation
    /// happens to be open when the answer lands, not the one it was actually asked in.
    func persistExchange(
        query: String,
        answer: String,
        citations: [ContextDisplayItem],
        model: String?,
        isFirstMessageInConversation: Bool,
        capturedConversationId: Int64?,
        capturedOllamaModel: String,
        titleGenEffective: (provider: RoutingSettings.Provider, modelLabel: String)
    ) {
        guard let store = appState.coreStore else { return }
        let slow = appState.slowStore
        let citationsJson = (try? JSONEncoder().encode(citations.map(\.saved))).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        CoreMemoryStore.accessQueue.async {
            let conversationId: Int64?
            if isFirstMessageInConversation {
                conversationId = store.createConversation(firstQuery: query)
            } else {
                conversationId = capturedConversationId
            }
            guard let conversationId else { return }
            _ = store.appendConversationExchange(conversationId: conversationId, query: query, answer: answer, citationsJson: citationsJson, model: model)
            DispatchQueue.main.async {
                // Only the newly-created (first-message) conversation should become the
                // active one here, and only if the user hasn't navigated away since this
                // exchange was sent (activeConversationId is still whatever it was at
                // send time -- nil, for the first-message case). If they already opened a
                // different conversation (or another new one) from the sidebar mid-flight,
                // this exchange still belongs to (and was just persisted to) its own
                // conversation, but that's no longer the one on screen, so don't yank the
                // UI back to it.
                if isFirstMessageInConversation, activeConversationId == capturedConversationId {
                    activeConversationId = conversationId
                }
                refreshConversationList()
            }
            if isFirstMessageInConversation, let slow {
                CoreMemoryStore.slowLaneQueue.async {
                    generateConversationTitle(store: slow, conversationId: conversationId, query: query, answer: answer, ollamaModel: capturedOllamaModel, effective: titleGenEffective)
                }
            }
        }
    }

    /// Always routes through Ollama-substitution logic the same way any other
    /// non-time-critical Ask-mode call would — title generation is itself
    /// non-time-critical by definition. On any failure the placeholder title (already
    /// set by `createConversation`) simply stays; no retry, no error surfaced anywhere.
    /// `ollamaModel` is the value captured in `askIfNeeded` at send time, not read live,
    /// for the same reason `persistExchange` captures its conversation id. `effective`
    /// is `askIfNeeded`'s `titleGenEffective` -- a routing decision computed from the
    /// sentinel non-time-critical query, NOT the original question's classification
    /// (see `nonTimeCriticalSentinelQuery`'s doc comment).
    func generateConversationTitle(
        store: CoreMemoryStore,
        conversationId: Int64,
        query: String,
        answer: String,
        ollamaModel: String,
        effective: (provider: RoutingSettings.Provider, modelLabel: String)
    ) {
        let titleRouting = appState.routing.routingConfig(preferredProvider: effective.provider, preferredModel: effective.modelLabel)
        guard let generated = store.generateConversationTitle(model: ollamaModel, query: query, answer: answer, routing: titleRouting) else {
            return
        }
        _ = store.setConversationTitleIfPlaceholder(id: conversationId, generatedTitle: generated)
        DispatchQueue.main.async { refreshConversationList() }
    }

    func refreshSpendToday() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.spendToday()
            DispatchQueue.main.async {
                spendTodayCents = result?.userCents
            }
        }
    }
}
