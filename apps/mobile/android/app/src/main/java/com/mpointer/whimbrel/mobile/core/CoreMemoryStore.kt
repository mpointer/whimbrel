package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import com.mpointer.whimbrel.mobile.ask.AskWire
import com.mpointer.whimbrel.mobile.ask.CitationCard
import com.mpointer.whimbrel.mobile.ask.CitationRef
import com.mpointer.whimbrel.mobile.ask.PhoneAskStore
import com.mpointer.whimbrel.mobile.ask.PrepareResult
import com.mpointer.whimbrel.mobile.read.*
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray

/** The phone's content-sync surface; it is also the read side, so the store a model syncs is the one it reads. */
interface MeetingSyncStore : PhoneReadStore, AutoCloseable {
    /** One blocking pull-only round: never on the main thread. */
    fun pullOnly(c: SyncCredentials, captureWindowDays: Int): Boolean

    /** Entries the Mac's Meetings list would show (grouped, limit 200); null on failure. */
    fun groupedMeetingCount(): Int?
    fun lastSyncError(): String?

    /** The last pull stopped at its per-round cap with rows still waiting: come back soon. */
    fun moreToPull(): Boolean

    /** The last pull's per-peer stats JSON (hashed peer tags, cursors, counts), for the log. */
    fun lastPullStats(): String? = null

    /** A new phone pulls newest first (#572): older meetings are still on their way. Persisted, so true after a relaunch too. */
    fun olderHistoryLoading(): Boolean = false

    /** Other devices that have not announced client deletes yet, as the last pull-only round counted them; null when
     *  not measured yet. The phone never starts a delete: this only feeds the sync line's "update needed". */
    fun devicesStillToUpdate(): Int? = null
}

/** Same context limit as the Mac's Ask. */
private const val ASK_CONTEXT_LIMIT = 5L

@kotlinx.serialization.Serializable
private data class PendingDevices(@Suppress("PropertyName") val pending_devices: Int? = null)

class CoreMemoryStore(private val store: CoreStore, private val native: CoreNative) : MeetingSyncStore, PhoneCaptureStore, PhoneAskStore {
    override fun pullOnly(c: SyncCredentials, captureWindowDays: Int): Boolean {
        native.memorySetDeviceId(store.handle, c.deviceId)
        return native.syncContentPullOnly(store.handle, c.tursoDatabaseUrl, c.tursoToken, c.groupKeyBase64, c.deviceId, captureWindowDays)
    }

    override fun groupedMeetingCount(): Int? =
        native.meetingsGroupedCount(store.handle).takeIf { it >= 0 }?.toInt()

    override fun lastSyncError(): String? = native.syncContentLastError()
    override fun moreToPull(): Boolean = native.syncContentMoreToPull()
    override fun lastPullStats(): String? = native.syncContentLastPullStats()
    override fun olderHistoryLoading(): Boolean = native.syncContentOlderHistoryLoading(store.handle)

    /** `{"pending_devices": n | null}` from `whimbrel_client_pending_devices_json`. */
    override fun devicesStillToUpdate(): Int? =
        native.clientPendingDevicesJson(store.handle)?.let { json ->
            runCatching { readJson.decodeFromString<PendingDevices>(json).pending_devices }.getOrNull()
        }

    private val h get() = store.handle

    // A malformed or missing reply is the same quiet answer as no rows.
    private inline fun <reified T> decode(json: String?): T? =
        json?.let { runCatching { readJson.decodeFromString<T>(it) }.getOrNull() }

    override fun recentMeetings(limit: Int): List<PhoneMeeting> = decode(native.meetingsRecentGrouped(h, limit.toLong())) ?: emptyList()
    override fun meeting(id: Long): PhoneMeeting? = decode(native.memoryGetMeeting(h, id))
    override fun transcriptPage(meetingId: Long, offset: Int, limit: Int): PhoneTranscriptPage? =
        decode(native.memoryGetTranscriptSegmentsPage(h, meetingId, offset.toLong(), limit.toLong()))
    override fun actionItems(meetingId: Long): List<PhoneActionItem> = decode(native.memoryListMeetingActionItems(h, meetingId)) ?: emptyList()
    override fun routineRuns(limit: Int): List<PhoneRoutineRun> = decode(native.routinesRecentRuns(h, limit.toLong())) ?: emptyList()
    override fun prep(eventId: String): PhoneMeetingPrep? = decode(native.meetingPrepGet(h, eventId))
    override fun upcomingEvents(fromUnixMs: Long, untilUnixMs: Long, limit: Int): List<PhoneCalendarEvent> =
        decode(native.memoryUpcomingCalendarEvents(h, fromUnixMs, untilUnixMs, limit.toLong())) ?: emptyList()
    override fun topicSnapshots(): List<PhoneTopicSnapshot> = PhoneTopicSnapshot.decodeList(native.topicSnapshotsJson(h)) ?: emptyList()
    override fun meetingId(deviceId: String, sourceMeetingId: Long): Long? =
        native.meetingIdForSource(h, deviceId, sourceMeetingId).takeIf { it >= 0 }
    override fun captureItem(id: Long): PhoneCaptureItem? = decode(native.memoryGetCaptureItem(h, id))

    // Ask (the iPhone's `CoreMemoryStore: PhoneAskStore`). Store thread only.

    override fun prepareAsk(query: String, historyJson: String, routingJson: String): PrepareResult =
        AskWire.prepareResult(native.llmPrepareAsk(h, query, ASK_CONTEXT_LIMIT, historyJson, routingJson))

    override fun finalizeAsk(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>? =
        AskWire.citationRefs(native.llmFinalizeAsk(h, prepareId, text, model, inputTokens, outputTokens))?.let(::resolveCitations)

    override fun discardAsk(prepareId: String) = native.llmDiscardAsk(prepareId)

    override fun resolveCitations(refs: List<CitationRef>): List<CitationCard> =
        if (refs.isEmpty()) emptyList() else AskWire.citationCards(native.citationsResolve(h, AskWire.refsJson(refs)))

    override fun noteModelAnswer(provider: String, requested: String, answered: String, nowUnixMs: Long) {
        native.modelNoteAnswer(h, provider, requested, answered, nowUnixMs)
    }

    override fun close() = store.close()
}
