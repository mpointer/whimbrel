package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.sync.SyncModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * One meeting's detail: the meeting, its action items and its transcript in pages; ported from `MeetingDetailModel`.
 * Call from the main thread.
 */
class MeetingDetailModel(val meetingId: Long) {
    companion object {
        const val pageSize = 100
    }

    private val _meeting = MutableStateFlow<PhoneMeeting?>(null)
    val meeting: StateFlow<PhoneMeeting?> = _meeting.asStateFlow()
    private val _actionItems = MutableStateFlow<List<PhoneActionItem>>(emptyList())
    val actionItems: StateFlow<List<PhoneActionItem>> = _actionItems.asStateFlow()
    private val _transcript = MutableStateFlow<List<PhoneTranscriptSegment>>(emptyList())
    val transcript: StateFlow<List<PhoneTranscriptSegment>> = _transcript.asStateFlow()
    private val _transcriptTotal = MutableStateFlow(0)
    val transcriptTotal: StateFlow<Int> = _transcriptTotal.asStateFlow()
    private val _loaded = MutableStateFlow(false)
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    val hasMoreTranscript: Boolean get() = _transcript.value.size < _transcriptTotal.value

    private class Snapshot(val meeting: PhoneMeeting?, val items: List<PhoneActionItem>, val page: PhoneTranscriptPage?)

    suspend fun reload(sync: SyncModel) {
        val id = meetingId
        // A refresh re-reads as many segments as are already shown, so a pull never jumps the reader back to page 1.
        val count = maxOf(_transcript.value.size, pageSize)
        val snapshot = sync.read { Snapshot(it.meeting(id), it.actionItems(id), it.transcriptPage(id, 0, count)) }
            // No snapshot means the store is not open yet: keep showing "Loading" until it is.
            ?: return
        _meeting.value = snapshot.meeting
        _actionItems.value = snapshot.items
        _transcript.value = snapshot.page?.segments.orEmpty()
        _transcriptTotal.value = snapshot.page?.total ?: 0
        _loaded.value = true
    }

    suspend fun loadMoreTranscript(sync: SyncModel) {
        if (!hasMoreTranscript) return
        val id = meetingId
        val offset = _transcript.value.size
        val page = sync.read { it.transcriptPage(id, offset, pageSize) } ?: return
        // A refresh may have re-read these segments meanwhile: append only if still next in line.
        if (_transcript.value.size != offset) return
        _transcript.value += page.segments
        _transcriptTotal.value = page.total
    }
}
