package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.sync.SyncCredentialSource
import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import com.mpointer.whimbrel.mobile.sync.SyncModel
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers

/** A replica the tests fill in: what the screens read, nothing else. */
class ScriptedStore : MeetingSyncStore {
    var meetings: List<PhoneMeeting> = emptyList()
    var events: List<PhoneCalendarEvent> = emptyList()
    val preps = mutableMapOf<String, PhoneMeetingPrep>()
    var runs: List<PhoneRoutineRun> = emptyList()
    val transcripts = mutableMapOf<Long, List<PhoneTranscriptSegment>>()
    val items = mutableMapOf<Long, List<PhoneActionItem>>()

    override fun recentMeetings(limit: Int) = meetings.take(limit)
    override fun meeting(id: Long) = meetings.firstOrNull { it.id == id }
    override fun transcriptPage(meetingId: Long, offset: Int, limit: Int): PhoneTranscriptPage? {
        val all = transcripts[meetingId] ?: return PhoneTranscriptPage(emptyList(), 0)
        return PhoneTranscriptPage(all.drop(offset).take(limit), all.size)
    }
    override fun actionItems(meetingId: Long) = items[meetingId].orEmpty()
    override fun routineRuns(limit: Int) = runs.take(limit)
    override fun prep(eventId: String) = preps[eventId]
    override fun upcomingEvents(fromUnixMs: Long, untilUnixMs: Long, limit: Int) =
        events.filter { it.startUnixMs in fromUnixMs until untilUnixMs }.sortedBy { it.startUnixMs }.take(limit)

    var topics: List<PhoneTopicSnapshot> = emptyList()
    override fun topicSnapshots() = topics

    override fun pullOnly(c: SyncCredentials, captureWindowDays: Int) = true
    override fun groupedMeetingCount(): Int? = meetings.size
    override fun lastSyncError(): String? = null
    override fun moreToPull(): Boolean = false
    override fun close() {}
}

private object NoCredentials : SyncCredentialSource {
    override fun syncCredentials(): SyncCredentials? = null
    override suspend fun renewIfDue(afterFailure: Boolean, force: Boolean) {}
}

/** A [SyncModel] whose store is open ([store] set) or never opens ([store] null); one round opens it. */
suspend fun syncWith(store: MeetingSyncStore?, scope: CoroutineScope): SyncModel =
    SyncModel({ store }, NoCredentials, scope, Dispatchers.Unconfined, {}, { 0L }, log = {}).also { it.syncNow() }
