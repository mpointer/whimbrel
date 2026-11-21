package com.mpointer.whimbrel.mobile.ask

import com.mpointer.whimbrel.mobile.read.readJson
import com.mpointer.whimbrel.mobile.sync.AskProvider
import com.mpointer.whimbrel.mobile.sync.PhoneSettingsSync
import com.mpointer.whimbrel.mobile.sync.PrefStore
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

// Ask on the phone, ported from the iPhone's AskModel.swift. The core assembles the prompt from the replica,
// the answer streams from the user's own hosted key, then the core names the sources.

/** A citation the core returns from `whimbrel_llm_finalize_ask`, and what `whimbrel_citations_resolve` takes back in. */
@Serializable
data class CitationRef(val kind: String, val id: Long)

/** What the phone shows for one citation (`core/api/src/citations.rs`'s `CitationCard`). */
@Serializable
data class CitationCard(
    val kind: String,
    val id: Long,
    val title: String = "",
    val subtitle: String = "",
    @SerialName("ts_unix_ms") val tsUnixMs: Long? = null,
    @SerialName("meeting_id") val meetingId: Long? = null,
    @SerialName("device_id") val deviceId: String? = null,
    /** False when the row isn't in this replica. */
    val found: Boolean = false,
)

sealed interface PrepareResult {
    data class Ok(val prepareId: String, val prompt: String) : PrepareResult
    data object SpendCap : PrepareResult
    data object Failed : PrepareResult
}

/** The store calls behind Ask, on the store thread. Separate from the read store so the screens' fakes needn't know about Ask. */
interface PhoneAskStore {
    fun prepareAsk(query: String, historyJson: String, routingJson: String): PrepareResult

    /** Citations for a finished answer, as cards; also logs usage. Null only if the core call failed. */
    fun finalizeAsk(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>?
    fun discardAsk(prepareId: String)
    fun resolveCitations(refs: List<CitationRef>): List<CitationCard>

    /** A streamed Ask asked [requested] and [answered] replied: a different model records [requested] as unavailable. */
    fun noteModelAnswer(provider: String, requested: String, answered: String, nowUnixMs: Long)
}

sealed interface AskRoute {
    class Ready(val provider: AskProvider, val model: String, val key: String) : AskRoute {
        /** Never the key: a route can end up in a log line. */
        override fun toString() = "Ready(${provider.raw}, $model)"
    }
    data object NoHostedKey : AskRoute
    data object LocalOnly : AskRoute
}

data class AskTurn(
    val id: Long,
    val question: String,
    /** The model that is answering, or answered: the provider's default when the requested one doesn't exist there. */
    val model: String,
    val answer: String = "",
    val state: State = State.Streaming,
    val citations: List<CitationCard> = emptyList(),
) {
    enum class State { Streaming, Done, Stopped, SpendCap, Failed }
}

/** `llm::streaming::InferenceEvent` (core/llm/src/streaming.rs), tagged on `kind`; only what Ask reads. */
sealed interface InferenceEvent {
    val runId: String

    /** [model] is set once the provider names the model that is answering. */
    data class State(override val runId: String, val state: String, val model: String?) : InferenceEvent
    data class TextDelta(override val runId: String, val text: String) : InferenceEvent
    data class Progress(override val runId: String) : InferenceEvent
    data class Terminal(
        override val runId: String,
        val outcome: Outcome,
        val finalText: String = "",
        val inputTokens: Long? = null,
        val outputTokens: Long? = null,
    ) : InferenceEvent

    enum class Outcome { Completed, Cancelled, Failed }
}

/** The synced routing settings Ask reads; [PhoneSettingsSync] wrote them. An empty value counts as unset. */
class PhoneRoutingPrefs(private val prefs: PrefStore) {
    private fun string(field: String): String? = prefs.getString(PhoneSettingsSync.PREFIX + field)?.takeIf { it.isNotEmpty() }

    val preferredProvider: String? get() = string("routing.preferredProvider")
    fun defaultModel(provider: AskProvider): String? = string("routing.${provider.raw}DefaultModel")
    val localOnlyMode: Boolean get() = string("routing.localOnlyMode") == "true"
    val userDailyCapCents: Long? get() = string("routing.userDailyCapCents")?.toLongOrNull()
    val globalDailyCapCents: Long? get() = string("routing.globalDailyCapCents")?.toLongOrNull()
    val customInstructions: String get() = prefs.getString(PhoneSettingsSync.PREFIX + "summary.customPromptInstructions").orEmpty()
}

/** The language tag answers are written in, as the iPhone's `AppLanguage` gives it for an English app. */
object AskLanguage {
    private val englishVariants = setOf("US", "GB", "CA", "AU")

    /**
     * This app is English only, so the tag is English with the phone's region when that is one of the English
     * variants. "" for en-US, which keeps the prompt byte-identical to an unlocalized one.
     */
    fun outputTag(country: String?): String {
        val tag = "en-" + (country?.uppercase()?.takeIf { it in englishVariants } ?: "US")
        return if (tag == "en-US") "" else tag
    }
}

/** The JSON Ask sends to and reads from the core. Pure, so it is tested without the core. */
object AskWire {
    @Serializable private data class PrepareData(@SerialName("prepare_id") val prepareId: String, val prompt: String)
    @Serializable private data class Failure(val kind: String = "")
    @Serializable private data class Envelope(val ok: Boolean = false, val data: PrepareData? = null, val error: Failure? = null)
    @Serializable private data class Finalized(val citations: List<CitationRef>)

    /** `{"ok":true,"data":{"prepare_id","prompt"}}`, or an error envelope whose kind may be `spend_cap`. */
    fun prepareResult(json: String?): PrepareResult {
        val env = json?.let { runCatching { readJson.decodeFromString(Envelope.serializer(), it) }.getOrNull() } ?: return PrepareResult.Failed
        if (env.ok && env.data != null) return PrepareResult.Ok(env.data.prepareId, env.data.prompt)
        return if (env.error?.kind == "spend_cap") PrepareResult.SpendCap else PrepareResult.Failed
    }

    /** `{"citations":[{"kind","id"}]}`; null when the reply is missing or malformed. */
    fun citationRefs(json: String?): List<CitationRef>? =
        json?.let { runCatching { readJson.decodeFromString(Finalized.serializer(), it).citations }.getOrNull() }

    fun refsJson(refs: List<CitationRef>): String = readJson.encodeToString(ListSerializer(CitationRef.serializer()), refs)

    fun citationCards(json: String?): List<CitationCard> =
        json?.let { runCatching { readJson.decodeFromString(ListSerializer(CitationCard.serializer()), it) }.getOrNull() }.orEmpty()

    /** `{"run_id":"..."}` from `whimbrel_inference_start`. */
    fun runId(json: String?): String? =
        json?.let { runCatching { (readJson.parseToJsonElement(it) as? JsonObject)?.get("run_id") as? JsonPrimitive }.getOrNull() }
            ?.takeIf { it.isString }?.content?.takeIf { it.isNotEmpty() }

    /** The events in one poll's reply. An event this build does not know is skipped, so the ones after it still arrive. */
    fun events(json: String?): List<InferenceEvent> {
        val array = json?.let { runCatching { readJson.parseToJsonElement(it) }.getOrNull() } as? JsonArray ?: return emptyList()
        return array.mapNotNull { (it as? JsonObject)?.let(::event) }
    }

    private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
    private fun JsonObject.long(key: String): Long? = (this[key] as? JsonPrimitive)?.longOrNull

    private fun event(o: JsonObject): InferenceEvent? {
        val runId = o.text("run_id") ?: return null
        return when (o.text("kind")) {
            "state" -> InferenceEvent.State(runId, o.text("state") ?: return null, o.text("model"))
            "text_delta" -> InferenceEvent.TextDelta(runId, o.text("text") ?: return null)
            "progress" -> InferenceEvent.Progress(runId)
            "terminal" -> {
                // An outcome this build does not know ends the run as a failure, as on the Mac.
                val outcome = when ((o["outcome"] as? JsonObject)?.text("kind")) {
                    "completed" -> InferenceEvent.Outcome.Completed
                    "cancelled" -> InferenceEvent.Outcome.Cancelled
                    else -> InferenceEvent.Outcome.Failed
                }
                val usage = o["usage"] as? JsonObject
                InferenceEvent.Terminal(runId, outcome, o.text("final_text").orEmpty(), usage?.long("input_tokens"), usage?.long("output_tokens"))
            }
            else -> null
        }
    }

    /** `whimbrel_llm_answer`'s `history_json`: finished turns, oldest first. */
    fun historyJson(turns: List<AskTurn>): String = buildJsonArray {
        for (turn in turns) if (turn.state == AskTurn.State.Done) {
            add(buildJsonObject { put("role", "User"); put("text", turn.question) })
            add(buildJsonObject { put("role", "Assistant"); put("text", turn.answer) })
        }
    }.toString()

    /**
     * `routing_config_json`: the minimum, plus `use_local_model: false` (the core adds a local model to every
     * chain unless told otherwise; the phone has none). Only the chosen provider's key is sent. Null caps are
     * left out (the core's default). Never log the result: it holds the key.
     */
    fun routingJson(route: AskRoute.Ready, prefs: PhoneRoutingPrefs, outputLanguage: String): String = buildJsonObject {
        put("sensitive", false)
        put("preferred_provider", route.provider.raw)
        put("preferred_model", route.model)
        put("custom_instructions", prefs.customInstructions)
        put("use_local_model", false)
        put("output_language", outputLanguage)
        when (route.provider) {
            AskProvider.ANTHROPIC -> put("anthropic_key", route.key)
            AskProvider.OPENAI -> put("openai_key", route.key)
            AskProvider.GOOGLE -> put("google_key", route.key)
            AskProvider.OPENROUTER -> {
                put("openrouter_key", route.key)
                put("openrouter_model", route.model)
            }
        }
        prefs.userDailyCapCents?.let { put("user_daily_cap_cents", it) }
        prefs.globalDailyCapCents?.let { put("global_daily_cap_cents", it) }
    }.toString()
}
