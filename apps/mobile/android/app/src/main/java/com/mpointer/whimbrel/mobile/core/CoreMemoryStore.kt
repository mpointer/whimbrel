package com.mpointer.whimbrel.mobile.core

import com.mpointer.whimbrel.mobile.sync.SyncCredentials
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
}

class CoreMemoryStore(private val store: CoreStore, private val native: CoreNative) : MeetingSyncStore {
    override fun pullOnly(c: SyncCredentials, captureWindowDays: Int): Boolean {
        native.memorySetDeviceId(store.handle, c.deviceId)
        return native.syncContentPullOnly(store.handle, c.tursoDatabaseUrl, c.tursoToken, c.groupKeyBase64, c.deviceId, captureWindowDays)
    }

    override fun groupedMeetingCount(): Int? =
        native.meetingsGroupedCount(store.handle).takeIf { it >= 0 }?.toInt()

    override fun lastSyncError(): String? = native.syncContentLastError()
    override fun moreToPull(): Boolean = native.syncContentMoreToPull()
    override fun lastPullStats(): String? = native.syncContentLastPullStats()

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

    override fun close() = store.close()
}
