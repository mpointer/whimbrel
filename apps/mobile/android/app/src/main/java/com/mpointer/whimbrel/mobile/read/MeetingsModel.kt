package com.mpointer.whimbrel.mobile.read

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class MeetingDaySection(val day: LocalDate, val meetings: List<PhoneMeeting>)

/**
 * The Meetings list: the newest grouped meetings, filtered by device and title; ported from `MeetingsModel.swift`.
 * Call from the main thread.
 */
class MeetingsModel {
    companion object {
        /** As the Mac's Meetings list: the 200 newest grouped meetings. */
        const val limit = 200

        /**
         * Meetings grouped by local day, newest day first; [deviceFilter] keeps one device, [query] keeps titles
         * containing it (case-insensitive; blank means everything).
         */
        fun sections(meetings: List<PhoneMeeting>, deviceFilter: String?, query: String, zone: ZoneId): List<MeetingDaySection> {
            val needle = query.trim()
            return meetings
                .filter { (deviceFilter == null || it.deviceId == deviceFilter) && (needle.isEmpty() || it.title.contains(needle, ignoreCase = true)) }
                .groupBy { Instant.ofEpochMilli(it.startUnixMs).atZone(zone).toLocalDate() }
                .toSortedMap(reverseOrder())
                .map { (day, group) -> MeetingDaySection(day, group.sortedByDescending { it.startUnixMs }) }
        }
    }

    private val _meetings = MutableStateFlow<List<PhoneMeeting>>(emptyList())
    val meetings: StateFlow<List<PhoneMeeting>> = _meetings.asStateFlow()
    private val _loaded = MutableStateFlow(false)
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    /** Compose snapshot state: reading these in a composable recomposes it when they change. */
    var deviceFilter: String? by mutableStateOf(null)
    var query: String by mutableStateOf("")

    val isCapped: Boolean get() = _meetings.value.size >= limit

    /** Device ids in order of first appearance (newest meeting first). */
    val devices: List<String> get() = _meetings.value.map { it.deviceId }.distinct()

    suspend fun reload(sync: SyncModel) {
        _meetings.value = sync.read { it.recentMeetings(limit) }.orEmpty()
        _loaded.value = true
    }
}
