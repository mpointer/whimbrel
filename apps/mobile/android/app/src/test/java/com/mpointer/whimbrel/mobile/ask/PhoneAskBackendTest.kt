package com.mpointer.whimbrel.mobile.ask

import com.mpointer.whimbrel.mobile.core.CoreNative
import com.mpointer.whimbrel.mobile.core.FakeSyncNative
import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.read.ScriptedStore
import com.mpointer.whimbrel.mobile.read.syncWith
import com.mpointer.whimbrel.mobile.sync.AskProvider
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The core's inference calls, scripted: each poll returns the next reply. */
private class StreamNative(private val startReply: String?, private val polls: MutableList<String?>) : FakeSyncNative() {
    val log = mutableListOf<String>()
    var started: List<String?>? = null
    override fun inferenceStart(prompt: String, model: String, provider: String, apiKey: String?, baseUrlOverride: String?): String? {
        started = listOf(prompt, model, provider, apiKey, baseUrlOverride)
        return startReply
    }
    override fun inferencePoll(runId: String): String? = if (polls.isEmpty()) "[]" else polls.removeAt(0)
    override fun inferenceCancel(runId: String): Boolean { log += "cancel $runId"; return true }
    override fun inferenceRelease(runId: String) { log += "release $runId" }
    override fun modelResolve(provider: String, stored: String?): String? = if (provider == "openrouter") stored.orEmpty() else stored ?: "$provider-default"
}

/** A replica that also answers the Ask store calls, recording them. */
private class AskStore(inner: ScriptedStore = ScriptedStore()) : MeetingSyncStore by inner, PhoneAskStore {
    val calls = mutableListOf<String>()
    override fun prepareAsk(query: String, historyJson: String, routingJson: String): PrepareResult {
        calls += "prepare $query"
        return PrepareResult.Ok("p1", "PROMPT")
    }
    override fun finalizeAsk(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>? {
        calls += "finalize $prepareId $model $inputTokens $outputTokens"
        return listOf(CitationCard("capture", 4, found = true))
    }
    override fun discardAsk(prepareId: String) { calls += "discard $prepareId" }
    override fun resolveCitations(refs: List<CitationRef>): List<CitationCard> = emptyList()
    override fun noteModelAnswer(provider: String, requested: String, answered: String, nowUnixMs: Long) { calls += "note $provider $requested $answered $nowUnixMs" }
}

private const val STREAMING = """{"kind":"state","run_id":"r1","state":"streaming","at_unix_ms":1,"model":"m","attempt":1}"""
private const val DELTA = """{"kind":"text_delta","run_id":"r1","sequence":0,"text":"Hi"}"""
private const val COMPLETED = """{"kind":"terminal","run_id":"r1","outcome":{"kind":"completed"},"at_unix_ms":2,"final_text":"Hi","usage":{"input_tokens":5,"output_tokens":1}}"""

@OptIn(ExperimentalCoroutinesApi::class)
class PhoneAskBackendTest {
    @Test
    fun storeCallsGoThroughTheOpenStore() = runTest {
        val store = AskStore()
        val sync = syncWith(store, backgroundScope)
        val backend = PhoneAskBackend({ sync }, FakeSyncNative(), this, clock = { 77 })
        assertEquals(PrepareResult.Ok("p1", "PROMPT"), backend.prepare("Q", "[]", "{}"))
        assertEquals(listOf(CitationCard("capture", 4, found = true)), backend.finalize("p1", "A", "m", 5, 1))
        backend.discard("p2")
        backend.noteAnswer(AskProvider.GOOGLE, "asked", "answered")
        advanceUntilIdle()
        assertEquals(listOf("prepare Q", "finalize p1 m 5 1", "discard p2", "note google asked answered 77"), store.calls)
    }

    @Test
    fun aClosedStoreFailsQuietlyAndIsNeverCalled() = runTest {
        // Revoked or not yet synced: there is no store, so no call can reach a closed handle.
        val sync = syncWith(null, backgroundScope)
        val backend = PhoneAskBackend({ sync }, FakeSyncNative(), this)
        assertEquals(PrepareResult.Failed, backend.prepare("Q", "[]", "{}"))
        assertNull(backend.finalize("p1", "A", "m", -1, -1))
        backend.discard("p1")
        backend.noteAnswer(AskProvider.OPENAI, "a", "b")
        advanceUntilIdle()
    }

    @Test
    fun aStoreWithoutAskFailsThePrepare() = runTest {
        val sync = syncWith(ScriptedStore(), backgroundScope)
        assertEquals(PrepareResult.Failed, PhoneAskBackend({ sync }, FakeSyncNative(), this).prepare("Q", "[]", "{}"))
    }

    private fun backend(native: CoreNative, scope: kotlinx.coroutines.test.TestScope) =
        PhoneAskBackend({ error("the stream never touches the store") }, native, scope, io = StandardTestDispatcher(scope.testScheduler), pollMs = 10)

    @Test
    fun streamPollsUntilTheTerminalEventThenReleasesTheRun() = runTest {
        val native = StreamNative("""{"run_id":"r1"}""", mutableListOf("[]", "[$STREAMING,$DELTA]", """[{"kind":"text_delta","run_id":"other","sequence":0,"text":"not ours"}]""", "[$COMPLETED]"))
        val events = backend(native, this).stream("PROMPT", "m", AskProvider.ANTHROPIC, "sk-a").toList()
        assertEquals(listOf("PROMPT", "m", "anthropic", "sk-a", null), native.started)
        assertEquals(
            listOf(InferenceEvent.State("r1", "streaming", "m"), InferenceEvent.TextDelta("r1", "Hi"), InferenceEvent.Terminal("r1", InferenceEvent.Outcome.Completed, "Hi", 5, 1)),
            events,
        )
        assertEquals("a finished run is released, not cancelled", listOf("release r1"), native.log)
    }

    @Test
    fun aCollectorThatStopsEarlyCancelsAndReleasesTheRun() = runTest {
        val native = StreamNative("""{"run_id":"r1"}""", mutableListOf("[$DELTA]"))
        assertEquals(InferenceEvent.TextDelta("r1", "Hi"), backend(native, this).stream("P", "m", AskProvider.OPENAI, "k").first())
        advanceUntilIdle()
        assertEquals(listOf("cancel r1", "release r1"), native.log)
    }

    @Test
    fun aRefusedStartIsAFailedRunWithNothingToRelease() = runTest {
        val native = StreamNative(null, mutableListOf())
        assertEquals(listOf<InferenceEvent>(InferenceEvent.Terminal("", InferenceEvent.Outcome.Failed)), backend(native, this).stream("P", "m", AskProvider.GOOGLE, "k").toList())
        assertTrue(native.log.isEmpty())
    }

    @Test
    fun aRunTheCoreNoLongerKnowsEndsAsAFailure() = runTest {
        val native = StreamNative("""{"run_id":"r1"}""", mutableListOf("[]", null))
        assertEquals(listOf<InferenceEvent>(InferenceEvent.Terminal("r1", InferenceEvent.Outcome.Failed)), backend(native, this).stream("P", "m", AskProvider.GOOGLE, "k").toList())
        assertEquals(listOf("release r1"), native.log)
    }

    @Test
    fun modelsResolveThroughTheCore() = runTest {
        val backend = backend(StreamNative(null, mutableListOf()), this)
        assertEquals("anthropic-default", backend.resolveModel(AskProvider.ANTHROPIC, null))
        assertEquals("picked", backend.resolveModel(AskProvider.OPENAI, "picked"))
        assertEquals("", backend.resolveModel(AskProvider.OPENROUTER, null))
    }
}
