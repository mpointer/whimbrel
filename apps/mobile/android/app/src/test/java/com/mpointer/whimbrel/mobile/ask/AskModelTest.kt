package com.mpointer.whimbrel.mobile.ask

import com.mpointer.whimbrel.mobile.sync.AskProvider
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A backend the tests script: what prepare returns, the events the stream emits, and a record of every call. */
private class FakeBackend : AskBackend {
    var prepareResult: PrepareResult = PrepareResult.Ok("p1", "PROMPT")
    /** When set, prepare waits here (as it would on a busy store thread) and still returns when stopped. */
    var prepareGate: CompletableDeferred<Unit>? = null
    val events = Channel<InferenceEvent>(Channel.UNLIMITED)
    val prepared = mutableListOf<Triple<String, String, String>>()
    val streamed = mutableListOf<List<String>>()
    val finalized = mutableListOf<List<Any>>()
    val discarded = mutableListOf<String>()
    val noted = mutableListOf<List<String>>()
    var cards: List<CitationCard>? = listOf(CitationCard("transcript", 1, "Call", "Meeting", found = true))

    override suspend fun prepare(query: String, historyJson: String, routingJson: String): PrepareResult {
        prepared += Triple(query, historyJson, routingJson)
        withContext(NonCancellable) { prepareGate?.await() }
        return prepareResult
    }
    override suspend fun finalize(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>? {
        finalized += listOf(prepareId, text, model, inputTokens, outputTokens)
        return cards
    }
    override fun discard(prepareId: String) { discarded += prepareId }
    override fun stream(prompt: String, model: String, provider: AskProvider, key: String): Flow<InferenceEvent> {
        streamed += listOf(prompt, model, provider.raw, key)
        return events.receiveAsFlow()
    }
    override fun noteAnswer(provider: AskProvider, requested: String, answered: String) { noted += listOf(provider.raw, requested, answered) }
    override fun resolveModel(provider: AskProvider, stored: String?): String = stored ?: if (provider == AskProvider.OPENROUTER) "" else "${provider.raw}-default"
}

@OptIn(ExperimentalCoroutinesApi::class)
class AskModelTest {
    private fun TestScope.model(backend: FakeBackend, prefs: AskPrefs = AskPrefs(), keys: Map<AskProvider, String> = mapOf(AskProvider.ANTHROPIC to "sk-a")) =
        AskModel(backend, PhoneRoutingPrefs(prefs), keys::get, this, outputLanguage = { "en-GB" })

    private val done = InferenceEvent.Terminal("r", InferenceEvent.Outcome.Completed, "Hello there", 12, 3)

    // ---- Route ----

    private fun route(prefs: AskPrefs, keys: Map<AskProvider, String>) = AskModel.route(PhoneRoutingPrefs(prefs), keys::get, FakeBackend()::resolveModel)

    @Test
    fun routePrefersTheSyncedProviderWhenItHasAKeyHere() {
        val r = route(AskPrefs("routing.preferredProvider" to "google", "routing.googleDefaultModel" to "gem-pick"), mapOf(AskProvider.ANTHROPIC to "a", AskProvider.GOOGLE to "g")) as AskRoute.Ready
        assertEquals(listOf("google", "gem-pick", "g"), listOf(r.provider.raw, r.model, r.key))
    }

    @Test
    fun routeFallsBackInAFixedOrderAndUsesTheCoresDefaultModel() {
        val r = route(AskPrefs("routing.preferredProvider" to "google"), mapOf(AskProvider.OPENAI to "o", AskProvider.OPENROUTER to "r")) as AskRoute.Ready
        assertEquals("the preferred provider has no key here", listOf("openai", "openai-default", "o"), listOf(r.provider.raw, r.model, r.key))
    }

    @Test
    fun routeNeedsAModelForOpenRouter() {
        assertEquals("OpenRouter has no default model", AskRoute.NoHostedKey, route(AskPrefs(), mapOf(AskProvider.OPENROUTER to "r")))
        val r = route(AskPrefs("routing.openrouterDefaultModel" to "vendor/m"), mapOf(AskProvider.OPENROUTER to "r")) as AskRoute.Ready
        assertEquals("vendor/m", r.model)
    }

    @Test
    fun routeIsLocalOnlyOrNoKey() {
        assertEquals(AskRoute.LocalOnly, route(AskPrefs("routing.localOnlyMode" to "true"), mapOf(AskProvider.ANTHROPIC to "a")))
        assertEquals(AskRoute.NoHostedKey, route(AskPrefs(), emptyMap()))
        assertEquals("an empty key is no key", AskRoute.NoHostedKey, route(AskPrefs(), mapOf(AskProvider.ANTHROPIC to "")))
    }

    // ---- A run ----

    @Test
    fun aQuestionStreamsThenGetsItsSources() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("  What changed?  ")
        advanceUntilIdle()
        val asked = model.turns.value.single()
        assertEquals("What changed?", asked.question)
        assertEquals(AskTurn.State.Streaming, asked.state)
        assertTrue(model.isBusy)
        assertEquals("What changed?", backend.prepared.single().first)
        assertEquals("[]", backend.prepared.single().second)
        assertTrue("the routing names the provider", backend.prepared.single().third.contains("\"preferred_provider\":\"anthropic\""))
        assertEquals(listOf("PROMPT", "anthropic-default", "anthropic", "sk-a"), backend.streamed.single())

        backend.events.send(InferenceEvent.State("r", "streaming", "anthropic-default"))
        backend.events.send(InferenceEvent.TextDelta("r", "Hel"))
        backend.events.send(InferenceEvent.TextDelta("r", "lo"))
        advanceUntilIdle()
        assertEquals("Hello", model.turns.value.single().answer)

        backend.events.send(done)
        advanceUntilIdle()
        val turn = model.turns.value.single()
        assertEquals(AskTurn.State.Done, turn.state)
        assertEquals("the final text replaces the deltas", "Hello there", turn.answer)
        assertEquals(backend.cards, turn.citations)
        assertEquals(listOf<Any>("p1", "Hello there", "anthropic-default", 12L, 3L), backend.finalized.single())
        assertEquals(listOf("anthropic", "anthropic-default", "anthropic-default"), backend.noted.single())
        assertTrue("finalized, so never discarded", backend.discarded.isEmpty())
        assertFalse(model.isBusy)
    }

    @Test
    fun theModelThatAnsweredIsShownAndLogged() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("Q")
        backend.events.send(InferenceEvent.State("r", "connecting", null))
        backend.events.send(InferenceEvent.State("r", "streaming", "fallback-model"))
        backend.events.send(InferenceEvent.Terminal("r", InferenceEvent.Outcome.Completed, "A"))
        advanceUntilIdle()
        assertEquals("fallback-model", model.turns.value.single().model)
        assertEquals("usage is logged under the model that answered; unreported counts are -1", listOf<Any>("p1", "A", "fallback-model", -1L, -1L), backend.finalized.single())
        assertEquals(listOf("anthropic", "anthropic-default", "fallback-model"), backend.noted.single())
    }

    @Test
    fun historyCarriesOnlyFinishedTurns() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("Q1")
        backend.events.send(InferenceEvent.Terminal("r", InferenceEvent.Outcome.Completed, "A1"))
        advanceUntilIdle()
        model.send("Q2")
        backend.events.send(InferenceEvent.Terminal("r", InferenceEvent.Outcome.Failed))
        advanceUntilIdle()
        model.send("Q3")
        advanceUntilIdle()
        assertEquals("""[{"role":"User","text":"Q1"},{"role":"Assistant","text":"A1"}]""", backend.prepared[2].second)
        model.stop() // the third question is still running
    }

    @Test
    fun stoppingKeepsWhatStreamedAndDiscardsThePreparedAsk() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("Q")
        backend.events.send(InferenceEvent.TextDelta("r", "Part"))
        advanceUntilIdle()
        model.stop()
        advanceUntilIdle()
        val turn = model.turns.value.single()
        assertEquals(AskTurn.State.Stopped, turn.state)
        assertEquals("Part", turn.answer)
        assertEquals(listOf("p1"), backend.discarded)
        assertTrue("a stopped run is never finalized", backend.finalized.isEmpty())
        // A late completion for the stopped run changes nothing.
        backend.events.send(done)
        advanceUntilIdle()
        assertEquals(AskTurn.State.Stopped, model.turns.value.single().state)
        assertEquals(listOf("p1"), backend.discarded)
    }

    @Test
    fun stoppingWhilePreparingDiscardsTheAskOnceItIsPrepared() = runTest {
        val backend = FakeBackend().apply { prepareGate = CompletableDeferred() }
        val model = model(backend)
        model.send("Q")
        advanceUntilIdle()
        model.stop()
        assertEquals(AskTurn.State.Stopped, model.turns.value.single().state)
        assertTrue("nothing to discard until the store answers", backend.discarded.isEmpty())
        backend.prepareGate!!.complete(Unit)
        advanceUntilIdle()
        assertEquals("the prepared ask is not lost", listOf("p1"), backend.discarded)
        assertTrue("no stream starts for a stopped question", backend.streamed.isEmpty())
    }

    @Test
    fun prepareFailureAndSpendCapEndTheTurnWithoutAStream() = runTest {
        val backend = FakeBackend().apply { prepareResult = PrepareResult.SpendCap }
        val model = model(backend)
        model.send("Q1")
        advanceUntilIdle()
        assertEquals(AskTurn.State.SpendCap, model.turns.value[0].state)
        backend.prepareResult = PrepareResult.Failed
        model.send("Q2")
        advanceUntilIdle()
        assertEquals(AskTurn.State.Failed, model.turns.value[1].state)
        assertTrue(backend.streamed.isEmpty())
        assertTrue(backend.discarded.isEmpty())
    }

    @Test
    fun aDropBeforeAnyTextIsAFailureAndAfterSomeTextKeepsIt() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("Q1")
        backend.events.send(InferenceEvent.Terminal("r", InferenceEvent.Outcome.Failed))
        advanceUntilIdle()
        assertEquals(AskTurn.State.Failed, model.turns.value[0].state)
        model.send("Q2")
        backend.events.send(InferenceEvent.TextDelta("r", "Some"))
        backend.events.send(InferenceEvent.Terminal("r", InferenceEvent.Outcome.Cancelled))
        advanceUntilIdle()
        assertEquals(AskTurn.State.Stopped, model.turns.value[1].state)
        assertEquals("Some", model.turns.value[1].answer)
        assertEquals("each failed run's prepared ask is discarded", listOf("p1", "p1"), backend.discarded)
    }

    @Test
    fun aStreamThatEndsWithoutATerminalEventFails() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("Q")
        backend.events.close()
        advanceUntilIdle()
        assertEquals(AskTurn.State.Failed, model.turns.value.single().state)
        assertEquals(listOf("p1"), backend.discarded)
    }

    @Test
    fun aFailedFinalizeStillFinishesTheAnswerWithNoSources() = runTest {
        val backend = FakeBackend().apply { cards = null }
        val model = model(backend)
        model.send("Q")
        backend.events.send(done)
        advanceUntilIdle()
        assertEquals(AskTurn.State.Done, model.turns.value.single().state)
        assertTrue(model.turns.value.single().citations.isEmpty())
    }

    @Test
    fun nothingIsSentWithoutARouteWhileBusyOrForABlankQuestion() = runTest {
        val backend = FakeBackend()
        val noKey = model(backend, keys = emptyMap())
        noKey.send("Q")
        advanceUntilIdle()
        assertTrue(noKey.turns.value.isEmpty())
        val localOnly = model(backend, AskPrefs("routing.localOnlyMode" to "true"))
        localOnly.send("Q")
        assertTrue("local-only mode never reaches a hosted model", localOnly.turns.value.isEmpty())

        val model = model(backend)
        model.send("   ")
        assertTrue(model.turns.value.isEmpty())
        model.send("Q1")
        model.send("Q2")
        advanceUntilIdle()
        assertEquals("one question at a time", listOf("Q1"), model.turns.value.map { it.question })
        assertEquals(1, backend.prepared.size)
        model.stop()
    }

    @Test
    fun resetEndsTheRunAndForgetsEveryTurn() = runTest {
        val backend = FakeBackend()
        val model = model(backend)
        model.send("Q1")
        backend.events.send(done)
        advanceUntilIdle()
        model.send("Q2")
        advanceUntilIdle()
        model.reset()
        advanceUntilIdle()
        assertTrue("a revoke leaves no question or answer on screen", model.turns.value.isEmpty())
        assertEquals("the running ask was discarded", listOf("p1"), backend.discarded)
        assertFalse(model.isBusy)
    }
}
