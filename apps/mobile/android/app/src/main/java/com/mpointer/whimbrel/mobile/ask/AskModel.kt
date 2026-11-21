package com.mpointer.whimbrel.mobile.ask

import com.mpointer.whimbrel.mobile.core.CoreNative
import com.mpointer.whimbrel.mobile.sync.AskProvider
import com.mpointer.whimbrel.mobile.sync.SyncModel
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.takeWhile
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The core calls an Ask needs. [prepare] and [finalize] run on the store thread and always return, even when the
 * caller was stopped meanwhile, so a prepared ask is never lost before it is finalized or discarded. [stream]
 * never runs on the store thread.
 */
interface AskBackend {
    suspend fun prepare(query: String, historyJson: String, routingJson: String): PrepareResult

    /** Extracts the answer's citations and logs usage; null if the core call failed. */
    suspend fun finalize(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>?

    /** Fire and forget: nothing waits on a discard. */
    fun discard(prepareId: String)
    fun stream(prompt: String, model: String, provider: AskProvider, key: String): Flow<InferenceEvent>

    /** A streamed run asked [requested] and [answered] replied; the core records a substitution (or clears an old one). */
    fun noteAnswer(provider: AskProvider, requested: String, answered: String) {}

    /** The model to use: [stored] when it was a real choice, else the core's default for [provider] ("" for none). */
    fun resolveModel(provider: AskProvider, stored: String?): String = stored.orEmpty()
}

/**
 * Ask on the phone: picks a hosted provider from the synced settings, has the core assemble the prompt from the
 * replica, streams the answer with the synced key, then asks the core for the citations. Call from the main thread.
 */
class AskModel(
    private val backend: AskBackend,
    private val prefs: PhoneRoutingPrefs,
    private val keys: (AskProvider) -> String?,
    private val scope: CoroutineScope,
    private val outputLanguage: () -> String = { "" },
) {
    private val _turns = MutableStateFlow<List<AskTurn>>(emptyList())
    val turns: StateFlow<List<AskTurn>> = _turns.asStateFlow()

    /** The running ask, if any. Exposed so tests can join it. */
    var job: Job? = null
        private set

    /** Each run's prepare awaiting a finalize or discard, by turn; whoever takes it clears it, so it is finalized or
     *  discarded exactly once, and a stopped run never touches the next turn's. */
    private val pendingPrepareIds = mutableMapOf<Long, String>()
    private var nextId = 1L

    /** Reads the key store: a few milliseconds of decryption, so screens read it off the main thread. */
    val route: AskRoute get() = route(prefs, keys, backend::resolveModel)
    val isBusy: Boolean get() = _turns.value.lastOrNull()?.state == AskTurn.State.Streaming

    /** Demo screenshots only: turns to show without a provider. */
    internal fun seedDemo(demo: List<AskTurn>) {
        _turns.value = demo
        nextId = (demo.maxOfOrNull { it.id } ?: 0) + 1
    }

    fun send(text: String) {
        val question = text.trim()
        val route = route as? AskRoute.Ready ?: return
        if (question.isEmpty() || isBusy) return
        val history = AskWire.historyJson(_turns.value)
        val turnId = nextId++
        _turns.value += AskTurn(turnId, question, route.model)
        val routing = AskWire.routingJson(route, prefs, outputLanguage())
        job = scope.launch { run(turnId, question, history, routing, route) }
    }

    /** A revoke or sign-out: end the running ask and forget every question and answer. */
    fun reset() {
        stop()
        _turns.value = emptyList()
    }

    /** Ends the running ask: the question stays, with whatever had streamed. */
    fun stop() {
        val last = _turns.value.lastOrNull()?.takeIf { it.state == AskTurn.State.Streaming } ?: return
        update(last.id) { it.copy(state = AskTurn.State.Stopped) }
        job?.cancel()
        discardPending(last.id)
    }

    private suspend fun run(turnId: Long, question: String, history: String, routing: String, route: AskRoute.Ready) {
        when (val prepared = backend.prepare(question, history, routing)) {
            PrepareResult.SpendCap -> updateStreaming(turnId) { it.copy(state = AskTurn.State.SpendCap) }
            PrepareResult.Failed -> updateStreaming(turnId) { it.copy(state = AskTurn.State.Failed) }
            is PrepareResult.Ok -> {
                // Stopped while preparing: nobody will stream or finalize this one.
                if (!currentCoroutineContext().isActive) return backend.discard(prepared.prepareId)
                pendingPrepareIds[turnId] = prepared.prepareId
                // The "streaming" state names the model that answered: the provider's default when the requested
                // one doesn't exist there. Usage is logged under it.
                var answered = route.model
                val terminal = backend.stream(prepared.prompt, route.model, route.provider, route.key)
                    .takeWhile { turn(turnId)?.state == AskTurn.State.Streaming }
                    .onEach { event ->
                        when (event) {
                            is InferenceEvent.State -> {
                                val named = event.model
                                if (event.state == "streaming" && !named.isNullOrEmpty() && named != answered) {
                                    answered = named
                                    update(turnId) { it.copy(model = named) }
                                }
                            }
                            is InferenceEvent.TextDelta -> update(turnId) { it.copy(answer = it.answer + event.text) }
                            is InferenceEvent.Progress, is InferenceEvent.Terminal -> {}
                        }
                    }
                    .filterIsInstance<InferenceEvent.Terminal>()
                    .firstOrNull()
                when (terminal?.outcome) {
                    InferenceEvent.Outcome.Completed -> {
                        val id = pendingPrepareIds.remove(turnId) ?: return
                        update(turnId) { it.copy(answer = terminal.finalText) }
                        if (route.provider in hostedWithDefaults) backend.noteAnswer(route.provider, route.model, answered)
                        val citations = backend.finalize(id, terminal.finalText, answered, terminal.inputTokens ?: -1, terminal.outputTokens ?: -1)
                        updateStreaming(turnId) { it.copy(citations = citations.orEmpty(), state = AskTurn.State.Done) }
                    }
                    InferenceEvent.Outcome.Cancelled, InferenceEvent.Outcome.Failed -> {
                        discardPending(turnId)
                        // A drop after some text keeps it ("Stopped"); before any text it is a failure.
                        update(turnId) { it.copy(state = if (it.answer.isEmpty()) AskTurn.State.Failed else AskTurn.State.Stopped) }
                    }
                    // The stream ended without a terminal event: stopped, or the run went away.
                    null -> {
                        discardPending(turnId)
                        updateStreaming(turnId) { it.copy(state = AskTurn.State.Failed) }
                    }
                }
            }
        }
    }

    private fun turn(id: Long): AskTurn? = _turns.value.firstOrNull { it.id == id }

    private fun update(id: Long, change: (AskTurn) -> AskTurn) {
        _turns.value = _turns.value.map { if (it.id == id) change(it) else it }
    }

    /** A change that only applies while the turn is still running (a stop meanwhile wins). */
    private fun updateStreaming(id: Long, change: (AskTurn) -> AskTurn) =
        update(id) { if (it.state == AskTurn.State.Streaming) change(it) else it }

    private fun discardPending(turnId: Long) {
        pendingPrepareIds.remove(turnId)?.let(backend::discard)
    }

    companion object {
        /** The providers the core keeps a default model for. */
        private val hostedWithDefaults = setOf(AskProvider.ANTHROPIC, AskProvider.OPENAI, AskProvider.GOOGLE)

        /**
         * The synced preferred provider if it has a key here, else the first with one in anthropic, openai,
         * google, openrouter order. Local-only mode has no route. The model is the core's default unless the
         * synced pick was a real choice; OpenRouter has no default, so it needs a synced pick.
         */
        fun route(prefs: PhoneRoutingPrefs, keys: (AskProvider) -> String?, resolveModel: (AskProvider, String?) -> String): AskRoute {
            if (prefs.localOnlyMode) return AskRoute.LocalOnly
            val preferred = prefs.preferredProvider?.let { raw -> AskProvider.entries.firstOrNull { it.raw == raw } }
            for (provider in listOfNotNull(preferred) + AskProvider.entries) {
                val model = resolveModel(provider, prefs.defaultModel(provider))
                val key = keys(provider)
                if (key.isNullOrEmpty() || model.isEmpty()) continue
                return AskRoute.Ready(provider, model, key)
            }
            return AskRoute.NoHostedKey
        }
    }
}

/**
 * The real backend: store calls through [SyncModel.read] (the store thread, null once the store is closed by a
 * revoke), streaming through the core's inference calls on [io], off the store thread and off main.
 */
class PhoneAskBackend(
    private val sync: () -> SyncModel,
    private val native: CoreNative,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val pollMs: Long = 100,
) : AskBackend {
    override suspend fun prepare(query: String, historyJson: String, routingJson: String): PrepareResult =
        withContext(NonCancellable) { sync().read { (it as? PhoneAskStore)?.prepareAsk(query, historyJson, routingJson) } } ?: PrepareResult.Failed

    override suspend fun finalize(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>? =
        withContext(NonCancellable) { sync().read { (it as? PhoneAskStore)?.finalizeAsk(prepareId, text, model, inputTokens, outputTokens) } }

    override fun discard(prepareId: String) {
        scope.launch { sync().read { (it as? PhoneAskStore)?.discardAsk(prepareId) } }
    }

    override fun noteAnswer(provider: AskProvider, requested: String, answered: String) {
        scope.launch { sync().read { (it as? PhoneAskStore)?.noteModelAnswer(provider.raw, requested, answered, clock()) } }
    }

    override fun resolveModel(provider: AskProvider, stored: String?): String = native.modelResolve(provider.raw, stored) ?: stored.orEmpty()

    override fun stream(prompt: String, model: String, provider: AskProvider, key: String): Flow<InferenceEvent> = flow {
        val runId = AskWire.runId(native.inferenceStart(prompt, model, provider.raw, key))
        if (runId == null) {
            emit(InferenceEvent.Terminal("", InferenceEvent.Outcome.Failed))
            return@flow
        }
        var finished = false
        try {
            while (true) {
                // Null: the core no longer knows this run.
                val reply = native.inferencePoll(runId)
                if (reply == null) {
                    finished = true
                    emit(InferenceEvent.Terminal(runId, InferenceEvent.Outcome.Failed))
                    return@flow
                }
                for (event in AskWire.events(reply)) {
                    if (event.runId != runId) continue
                    if (event is InferenceEvent.Terminal) finished = true
                    emit(event)
                    if (finished) return@flow
                }
                delay(pollMs)
            }
        } finally {
            // Stopped by the collector: cancel the run, as the Mac's `releaseCurrentRun` does.
            if (!finished) native.inferenceCancel(runId)
            native.inferenceRelease(runId)
        }
    }.flowOn(io)
}
