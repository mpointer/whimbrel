package com.mpointer.whimbrel.mobile.read

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.longOrNull

// What the phone reads from its replica. Same JSON keys as the Mac's CoreBridge shapes (and
// PhoneModels.swift); only the fields the screens use. Unknown keys are ignored and absent
// list fields decode as empty.

@Serializable
data class PhoneMeetingCopy(
    val id: Long,
    @SerialName("device_id") val deviceId: String,
    val title: String,
)

@Serializable
data class PhoneMeeting(
    val id: Long,
    @SerialName("device_id") val deviceId: String,
    @SerialName("start_unix_ms") val startUnixMs: Long,
    @SerialName("end_unix_ms") val endUnixMs: Long? = null,
    val title: String,
    val attendees: List<String> = emptyList(),
    val status: String = "",
    @SerialName("summary_tldr") val summaryTldr: String? = null,
    @SerialName("summary_decisions") val summaryDecisions: List<String> = emptyList(),
    @SerialName("summary_action_items") val summaryActionItems: List<String> = emptyList(),
    @SerialName("summary_open_questions") val summaryOpenQuestions: List<String> = emptyList(),
    /** Other recordings of the same call; only set by the grouped list. */
    @SerialName("also_recorded") val alsoRecorded: List<PhoneMeetingCopy>? = null,
)

@Serializable
data class PhoneTranscriptSegment(
    val id: Long,
    @SerialName("start_cs") val startCs: Long,
    @SerialName("speaker_label") val speakerLabel: String,
    val text: String,
)

@Serializable
data class PhoneTranscriptPage(val segments: List<PhoneTranscriptSegment>, val total: Int)

@Serializable
data class PhoneActionItem(
    val id: Long,
    val text: String,
    val completed: Boolean,
    @SerialName("order_index") val orderIndex: Long,
)

@Serializable
data class PhoneRoutineRun(
    val id: Long,
    @SerialName("routine_name") val routineName: String,
    @SerialName("ran_at_unix_ms") val ranAtUnixMs: Long,
    val status: String,
    @SerialName("output_summary") val outputSummary: String,
    @SerialName("origin_device_id") val originDeviceId: String? = null,
) {
    val succeeded: Boolean get() = status == "success"
}

@Serializable
data class PhoneMeetingBrief(
    val summary: Claim,
    val attendees: List<AttendeeNote>,
    @SerialName("open_items") val openItems: List<Claim>,
    @SerialName("open_questions") val openQuestions: List<Claim>,
) {
    @Serializable data class Claim(val text: String)
    @Serializable data class AttendeeNote(val who: String, val note: String)
}

@Serializable
data class PhoneMeetingPrep(
    @SerialName("event_id") val eventId: String,
    val title: String,
    @SerialName("start_unix_ms") val startUnixMs: Long,
    @SerialName("brief_json") val briefJson: String,
) {
    /** The brief is itself a JSON string; null when it doesn't parse. */
    val brief: PhoneMeetingBrief? get() = runCatching { readJson.decodeFromString<PhoneMeetingBrief>(briefJson) }.getOrNull()
}

@Serializable
data class PhoneCalendarEvent(
    @SerialName("external_id") val externalId: String,
    val source: String,
    val title: String,
    @SerialName("start_unix_ms") val startUnixMs: Long,
    val attendees: List<String>,
    /** The local row id; the highest wins when two rows carry the same meeting. Absent from older cores. */
    @SerialName("row_id") val rowId: Long? = null,
)

/** The cross-device identity of a meeting: the device that recorded it and its id there. */
@Serializable
data class PhoneMeetingRef(
    @SerialName("device_id") val deviceId: String,
    @SerialName("source_meeting_id") val sourceMeetingId: Long,
)

/**
 * A topic page as the computer built it (core's `TopicPageSnapshot`): read-only on the phone. Reminders read
 * the name, the next meeting and what you owe; the Topics screens read the rest. The two shapes a newer core
 * could change under us (the lead's `kind`, the attendee pairs) are kept as raw JSON and read leniently, so an
 * unknown one loses that line and not the page.
 */
@Serializable
data class PhoneTopicSnapshot(
    @SerialName("topic_key") val topicKey: String,
    val name: String,
    @SerialName("next_meeting") val nextMeeting: NextMeeting? = null,
    @SerialName("you_owe") val youOwe: List<Step> = emptyList(),
    val kind: String? = null,
    @SerialName("lead") val leadJson: JsonElement? = null,
    val role: Role? = null,
    @SerialName("owed_to_you") val owedToYou: List<Step> = emptyList(),
    val done: List<Step> = emptyList(),
    val recent: List<Recent> = emptyList(),
    @SerialName("recent_total") val recentTotal: Long = 0,
    /** True when the computer dropped steps or recent items to fit the size cap. */
    val truncated: Boolean = false,
    @SerialName("built_at_unix_ms") val builtAtUnixMs: Long = 0,
    /** The name of the computer that built it. */
    @SerialName("built_by") val builtBy: String = "",
) {
    /** The page's one-line lead (core's `SummaryLead`, tagged by `kind`): a dated fact or a step. */
    sealed interface Lead {
        data class Dated(val label: String, val date: String) : Lead
        data class NextStep(val text: String, val dueDate: String?) : Lead
    }

    @Serializable data class Role(val text: String = "", val edited: Boolean = false)

    data class OrgCount(val domain: String, val count: Long)

    @Serializable
    data class NextMeeting(
        val title: String,
        @SerialName("start_unix_ms") val startUnixMs: Long,
        /** Core sends each as a two-element array: `["acme.com", 3]`. */
        @SerialName("attendees_by_org") val attendeesByOrgJson: JsonElement? = null,
        /** Attendees whose address is one of the user's. */
        val me: Long = 0,
    ) {
        /** Attendees other than the user, by email domain, largest first. A pair that does not parse is left out. */
        val attendeesByOrg: List<OrgCount>
            get() = (attendeesByOrgJson as? JsonArray).orEmpty().mapNotNull { pair ->
                val a = pair as? JsonArray ?: return@mapNotNull null
                val domain = (a.getOrNull(0) as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return@mapNotNull null
                val count = (a.getOrNull(1) as? JsonPrimitive)?.longOrNull ?: return@mapNotNull null
                OrgCount(domain, count)
            }
    }

    @Serializable
    data class Step(
        val text: String,
        @SerialName("due_date") val dueDate: String? = null,
        @SerialName("source_kind") val sourceKind: String = "",
        @SerialName("source_title") val sourceTitle: String? = null,
        @SerialName("asked_at_unix_ms") val askedAtUnixMs: Long? = null,
        val meeting: PhoneMeetingRef? = null,
    )

    @Serializable
    data class Recent(
        val kind: String = "",
        val title: String = "",
        /** An email's first sender, by display name. */
        val sender: String? = null,
        @SerialName("ts_unix_ms") val tsUnixMs: Long = 0,
        val meeting: PhoneMeetingRef? = null,
    )

    /** Null for no lead, and for a kind this build does not know. */
    val lead: Lead?
        get() {
            val o = leadJson as? JsonObject ?: return null
            fun text(key: String) = (o[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
            return when (text("kind")) {
                "date" -> Lead.Dated(text("label") ?: return null, text("date") ?: return null)
                "step" -> Lead.NextStep(text("text") ?: return null, text("due_date"))
                else -> null
            }
        }

    /**
     * The next meeting unless it has started by [nowMs]: the page is frozen when it was built, so a meeting that
     * has passed since is treated as no next meeting.
     */
    fun upcomingMeeting(nowMs: Long): NextMeeting? = nextMeeting?.takeIf { it.startUnixMs > nowMs }

    companion object {
        /** Only what reminders read. A page whose other fields no longer parse still reminds. */
        @Serializable
        private data class Core(
            @SerialName("topic_key") val topicKey: String,
            val name: String,
            @SerialName("next_meeting") val nextMeeting: CoreMeeting? = null,
            @SerialName("you_owe") val youOwe: List<CoreStep> = emptyList(),
        )
        @Serializable private data class CoreMeeting(val title: String, @SerialName("start_unix_ms") val startUnixMs: Long)
        @Serializable private data class CoreStep(val text: String, @SerialName("due_date") val dueDate: String? = null)

        /**
         * Every page in the core's reply, one at a time: a page that does not parse in full falls back to the
         * fields reminders read, and one that does not parse at all is left out. Null when the reply is not a list.
         */
        fun decodeList(json: String?): List<PhoneTopicSnapshot>? {
            val array = json?.let { runCatching { readJson.parseToJsonElement(it) }.getOrNull() } as? JsonArray ?: return null
            return array.mapNotNull { element ->
                runCatching { readJson.decodeFromJsonElement(serializer(), element) }.getOrNull()
                    ?: runCatching { readJson.decodeFromJsonElement(Core.serializer(), element) }.getOrNull()?.let { c ->
                        PhoneTopicSnapshot(
                            c.topicKey, c.name, c.nextMeeting?.let { NextMeeting(it.title, it.startUnixMs) },
                            c.youOwe.map { Step(it.text, it.dueDate) },
                        )
                    }
            }
        }
    }
}

/** A capture row as the phone shows it. */
@Serializable
data class PhoneCaptureItem(
    @SerialName("window_title") val windowTitle: String = "",
    @SerialName("app_bundle_id") val appBundleId: String = "",
    val url: String? = null,
    val text: String = "",
) {
    val title: String get() = windowTitle.ifEmpty { appBundleId }
}

internal val readJson = Json { ignoreUnknownKeys = true }
