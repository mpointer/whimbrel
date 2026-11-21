package com.mpointer.whimbrel.mobile.ask

import com.mpointer.whimbrel.mobile.sync.AskProvider
import com.mpointer.whimbrel.mobile.sync.PhoneSettingsSync
import com.mpointer.whimbrel.mobile.sync.PrefStore
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Synced settings as [PhoneSettingsSync] stores them: every field under its prefix. */
internal class AskPrefs(vararg fields: Pair<String, String>) : PrefStore {
    private val m = fields.associate { (k, v) -> PhoneSettingsSync.PREFIX + k to v }.toMutableMap()
    override fun getString(key: String): String? = m[key]
    override fun getLong(key: String): Long? = null
    override fun getBoolean(key: String): Boolean = false
    override fun putString(key: String, v: String) { m[key] = v }
    override fun putLong(key: String, v: Long) {}
    override fun putBoolean(key: String, v: Boolean) {}
    override fun clearPrefix(prefix: String) { m.keys.removeAll { it.startsWith(prefix) } }
}

class AskWireTest {
    @Test
    fun prepareReplyIsOkSpendCapOrFailed() {
        assertEquals(PrepareResult.Ok("p1", "PROMPT"), AskWire.prepareResult("""{"ok":true,"data":{"prepare_id":"p1","prompt":"PROMPT"}}"""))
        assertEquals(PrepareResult.SpendCap, AskWire.prepareResult("""{"ok":false,"error":{"kind":"spend_cap","message":"cap"}}"""))
        assertEquals(PrepareResult.Failed, AskWire.prepareResult("""{"ok":false,"error":{"kind":"other"}}"""))
        assertEquals("ok with no data is not a prepared ask", PrepareResult.Failed, AskWire.prepareResult("""{"ok":true}"""))
        assertEquals(PrepareResult.Failed, AskWire.prepareResult("not json"))
        assertEquals(PrepareResult.Failed, AskWire.prepareResult(null))
    }

    @Test
    fun citationsRoundTrip() {
        assertEquals(listOf(CitationRef("transcript", 3), CitationRef("capture", 9)), AskWire.citationRefs("""{"citations":[{"kind":"transcript","id":3},{"kind":"capture","id":9}]}"""))
        assertEquals(emptyList<CitationRef>(), AskWire.citationRefs("""{"citations":[]}"""))
        assertNull("a failed core call is not the same as no citations", AskWire.citationRefs(null))
        assertEquals("""[{"kind":"transcript","id":3}]""", AskWire.refsJson(listOf(CitationRef("transcript", 3))))
        val cards = AskWire.citationCards(
            """[{"kind":"transcript","id":3,"title":"Call 😀","subtitle":"Meeting","ts_unix_ms":5,"meeting_id":8,"device_id":"mac","found":true},
                {"kind":"capture","id":9,"title":"","subtitle":"Screen","ts_unix_ms":null,"meeting_id":null,"device_id":null,"found":false}]""",
        )
        assertEquals(CitationCard("transcript", 3, "Call 😀", "Meeting", 5, 8, "mac", true), cards[0])
        assertFalse(cards[1].found)
        assertEquals(emptyList<CitationCard>(), AskWire.citationCards("oops"))
    }

    @Test
    fun runIdComesFromTheStartReply() {
        assertEquals("r-1", AskWire.runId("""{"run_id":"r-1"}"""))
        assertNull(AskWire.runId("""{"run_id":""}"""))
        assertNull(AskWire.runId(null))
        assertNull(AskWire.runId("[]"))
    }

    @Test
    fun pollEventsAreReadAndUnknownOnesSkipped() {
        val events = AskWire.events(
            """
            [{"kind":"state","run_id":"r","state":"connecting","at_unix_ms":1,"attempt":1},
             {"kind":"state","run_id":"r","state":"streaming","at_unix_ms":2,"provider":"anthropic","model":"m-2","attempt":1},
             {"kind":"text_delta","run_id":"r","sequence":0,"text":"Hel"},
             {"kind":"hologram","run_id":"r"},
             {"kind":"progress","run_id":"r","label":"x"},
             {"kind":"terminal","run_id":"r","outcome":{"kind":"completed"},"at_unix_ms":3,"final_text":"Hello","usage":{"input_tokens":12,"output_tokens":3}}]
            """,
        )
        assertEquals(
            listOf(
                InferenceEvent.State("r", "connecting", null),
                InferenceEvent.State("r", "streaming", "m-2"),
                InferenceEvent.TextDelta("r", "Hel"),
                InferenceEvent.Progress("r"),
                InferenceEvent.Terminal("r", InferenceEvent.Outcome.Completed, "Hello", 12, 3),
            ),
            events,
        )
        assertEquals(emptyList<InferenceEvent>(), AskWire.events("[]"))
        assertEquals(emptyList<InferenceEvent>(), AskWire.events(null))
    }

    @Test
    fun terminalOutcomes() {
        fun outcome(o: String) = (AskWire.events("""[{"kind":"terminal","run_id":"r","outcome":$o,"at_unix_ms":0,"final_text":""}]""").single() as InferenceEvent.Terminal)
        assertEquals(InferenceEvent.Outcome.Cancelled, outcome("""{"kind":"cancelled"}""").outcome)
        assertEquals(InferenceEvent.Outcome.Failed, outcome("""{"kind":"failed","class":"auth","message":"no"}""").outcome)
        assertEquals("an outcome this build does not know ends the run as a failure", InferenceEvent.Outcome.Failed, outcome("""{"kind":"paused"}""").outcome)
        assertNull("no usage reported", outcome("""{"kind":"completed"}""").inputTokens)
    }

    @Test
    fun historyIsFinishedTurnsOnly() {
        val turns = listOf(
            AskTurn(1, "Q1", "m", "A1", AskTurn.State.Done),
            AskTurn(2, "Q2", "m", "part", AskTurn.State.Stopped),
            AskTurn(3, "Q3", "m", state = AskTurn.State.Failed),
            AskTurn(4, "Q4", "m", "A4", AskTurn.State.Done),
        )
        assertEquals(
            """[{"role":"User","text":"Q1"},{"role":"Assistant","text":"A1"},{"role":"User","text":"Q4"},{"role":"Assistant","text":"A4"}]""",
            AskWire.historyJson(turns),
        )
        assertEquals("[]", AskWire.historyJson(emptyList()))
    }

    @Test
    fun routingSendsOnlyTheChosenProvidersKey() {
        val prefs = PhoneRoutingPrefs(AskPrefs("summary.customPromptInstructions" to "Be brief", "routing.userDailyCapCents" to "250"))
        val o = Json.parseToJsonElement(AskWire.routingJson(AskRoute.Ready(AskProvider.OPENAI, "gpt-x", "sk-secret"), prefs, "en-GB")).jsonObject
        assertEquals("openai", o.getValue("preferred_provider").jsonPrimitive.content)
        assertEquals("gpt-x", o.getValue("preferred_model").jsonPrimitive.content)
        assertEquals("sk-secret", o.getValue("openai_key").jsonPrimitive.content)
        assertEquals("the phone has no local model", "false", o.getValue("use_local_model").jsonPrimitive.content)
        assertEquals("false", o.getValue("sensitive").jsonPrimitive.content)
        assertEquals("Be brief", o.getValue("custom_instructions").jsonPrimitive.content)
        assertEquals("en-GB", o.getValue("output_language").jsonPrimitive.content)
        assertEquals("250", o.getValue("user_daily_cap_cents").jsonPrimitive.content)
        assertFalse("an unset cap is left to the core's default", o.containsKey("global_daily_cap_cents"))
        assertEquals(emptySet<String>(), o.keys.filter { it.endsWith("_key") }.toSet() - "openai_key")

        val router = Json.parseToJsonElement(AskWire.routingJson(AskRoute.Ready(AskProvider.OPENROUTER, "vendor/model", "or-key"), prefs, "")).jsonObject
        assertEquals("or-key", router.getValue("openrouter_key").jsonPrimitive.content)
        assertEquals("vendor/model", router.getValue("openrouter_model").jsonPrimitive.content)
    }

    @Test
    fun aRouteNeverPrintsItsKey() {
        assertFalse(AskRoute.Ready(AskProvider.ANTHROPIC, "claude", "sk-secret").toString().contains("sk-secret"))
    }

    @Test
    fun routingPrefsTreatEmptyAsUnset() {
        val prefs = PhoneRoutingPrefs(AskPrefs("routing.preferredProvider" to "", "routing.googleDefaultModel" to "gem", "routing.localOnlyMode" to "true", "routing.globalDailyCapCents" to "abc"))
        assertNull(prefs.preferredProvider)
        assertEquals("gem", prefs.defaultModel(AskProvider.GOOGLE))
        assertNull(prefs.defaultModel(AskProvider.OPENAI))
        assertTrue(prefs.localOnlyMode)
        assertNull("a cap that is not a number is no cap", prefs.globalDailyCapCents)
        assertEquals("", prefs.customInstructions)
    }

    @Test
    fun outputLanguageIsEnglishWithAKnownRegion() {
        assertEquals("en-US keeps the prompt as it was before localization", "", AskLanguage.outputTag("US"))
        assertEquals("en-GB", AskLanguage.outputTag("gb"))
        assertEquals("en-AU", AskLanguage.outputTag("AU"))
        assertEquals("an Italian phone gets the English app, so en-US", "", AskLanguage.outputTag("IT"))
        assertEquals("", AskLanguage.outputTag(null))
    }

    @Test
    fun citationLabelsAndTargets() {
        val meeting = CitationCard("transcript", 3, "Acme call", "Meeting", 5, 8, "mac", true)
        assertEquals("Meeting · MacBook · Acme call · Tue", CitationText.label(meeting, "MacBook", "Not on this phone") { "Tue" })
        assertEquals("what is empty is left out", "Screen · Pricing.xlsx", CitationText.label(CitationCard("capture", 1, "Pricing.xlsx", "Screen", found = true), null, "Not on this phone") { "x" })
        assertEquals("Screen · Not on this phone", CitationText.label(CitationCard("capture", 1, "t", "Screen", 5, found = false), "Mac", "Not on this phone") { "x" })
        assertTrue(CitationText.isRecent(0, 6 * 86_400_000L))
        assertFalse(CitationText.isRecent(0, 7 * 86_400_000L))

        assertEquals(CitationTarget.Meeting(8), CitationTarget.of(meeting))
        assertEquals(CitationTarget.Meeting(8), CitationTarget.of(meeting.copy(kind = "snapshot")))
        assertEquals("a transcript with no meeting here goes nowhere", CitationTarget.None, CitationTarget.of(meeting.copy(meetingId = null)))
        assertEquals(CitationTarget.Capture(4), CitationTarget.of(CitationCard("capture", 4, found = true)))
        assertEquals(CitationTarget.RoutineRun(6), CitationTarget.of(CitationCard("routine_run", 6, found = true)))
        assertEquals("a kind with no screen on the phone", CitationTarget.None, CitationTarget.of(CitationCard("episode", 6, found = true)))
        assertEquals("not on this phone", CitationTarget.None, CitationTarget.of(meeting.copy(found = false)))
    }
}
