package com.mpointer.whimbrel.mobile.read

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

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

/** A topic page as the computer built it (core's `TopicPageSnapshot`); only what reminders use. */
@Serializable
data class PhoneTopicSnapshot(
    @SerialName("topic_key") val topicKey: String,
    val name: String,
    @SerialName("next_meeting") val nextMeeting: NextMeeting? = null,
    @SerialName("you_owe") val youOwe: List<Step> = emptyList(),
) {
    @Serializable data class NextMeeting(val title: String, @SerialName("start_unix_ms") val startUnixMs: Long)
    @Serializable data class Step(val text: String, @SerialName("due_date") val dueDate: String? = null)
}

internal val readJson = Json { ignoreUnknownKeys = true }
